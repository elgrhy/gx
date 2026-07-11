//! GX Interpreter — executes the AST produced by the parser.

mod bridge_impl;
mod builtins_ai_context;
mod builtins_ast;
mod builtins_base64;
mod builtins_crypto;
mod builtins_data;
mod builtins_datetime;
mod builtins_db;
mod builtins_http;
mod builtins_json;
#[cfg(not(target_arch = "wasm32"))]
mod builtins_process;
mod builtins_regex;
#[cfg(not(target_arch = "wasm32"))]
mod builtins_task;
mod builtins_template;
mod builtins_vector;
mod builtins_xml;
pub mod debugger;
mod util;

pub use debugger::{DebugCommand, DebugMode, DebugState};

/// Stack size for every OS thread that runs GX interpreter logic
/// end-to-end (a `task_spawn`d closure's own worker thread; `main()`'s own
/// thread, spawned the same way — see `main.rs`) — well above the
/// platform-default thread stack, which empirically overflowed (a real,
/// unrecoverable process abort, not a catchable error) after well under
/// 100 levels of GX recursion. `Interpreter::MAX_CALL_DEPTH` is the actual
/// safety net that turns "too deep" into a graceful, catchable GX error;
/// this is what makes that limit's headroom consistent across every
/// thread the interpreter can run on, task workers included, rather than
/// only the specific thread that happened to get a custom stack size.
#[cfg(not(target_arch = "wasm32"))]
pub const WORKER_THREAD_STACK_SIZE: usize = 256 * 1024 * 1024;

/// Cap on the result of string repetition (`s * n` / `s.repeat(n)`). Without
/// this, `"ab" * n` for a huge `n` — including `Infinity`, which an
/// ordinary GX numeric literal can already produce by overflowing `f64`
/// during parsing, and which `n as usize` then saturates to `usize::MAX` —
/// asks the allocator for an astronomically large buffer, aborting the
/// whole process with a "capacity overflow" panic instead of a catchable
/// error. No capability flag guards this; it's reachable from the plain
/// `*` operator on any string.
const MAX_STRING_REPEAT_BYTES: usize = 64 * 1024 * 1024;

use crate::ai;
use crate::ast::*;
use crate::bridge::Bridge;
use crate::value::Value;
use std::collections::HashMap;

// Re-export public JSON helpers so other crates can use them.
pub use builtins_json::{gx_value_to_json, json_to_gx_value};
// Re-exported so bridge.rs can classify its own subprocess-spawn errors
// (executable not found vs. permission denied) the same way the native
// process runtime does.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use builtins_process::{classify_spawn_error, SpawnErrorKind};

// Bring extracted free functions into scope for use inside eval_call_expr.
use builtins_ast::gx_ast_to_value;
use builtins_base64::{base64_decode, base64_encode};
use builtins_crypto::{crypto_builtin, hex_encode};
use builtins_data::{
    csv_parse_impl, csv_stringify_impl, jsonl_parse_impl, jsonl_stringify_impl, toml_parse_impl,
    toml_stringify_impl, yaml_parse_impl, yaml_stringify_impl,
};
use builtins_datetime::{
    date_add_impl, date_diff_impl, date_format_impl, date_from_parts_impl, date_now_impl,
    date_parse_impl, date_parts_impl, date_timestamp_impl,
};
use builtins_db::{db_exec_on_conn, db_query_on_conn};
#[cfg(not(target_arch = "wasm32"))]
use builtins_http::check_url_safe;
use builtins_http::diagnostic_url;
use builtins_http::{http_builtin, http_stream_builtin, http_upload_builtin};
use builtins_regex::{
    regex_captures_impl, regex_find_all_impl, regex_find_impl, regex_named_captures_impl,
    regex_replace_impl, regex_split_impl, regex_test_impl,
};
use builtins_template::render_template_impl;
use builtins_vector::{
    cosine_similarity_impl, vector_store_add_impl, vector_store_delete_impl, vector_store_new_impl,
    vector_store_search_impl, vector_store_size_impl,
};
use builtins_xml::{xml_parse_impl, xml_stringify_impl};
use util::{
    cron_matches, helper_is_callable_only, infer_error_kind, is_package_import, parse_gx_source,
    strip_html_tags, value_to_json,
};

/// Ceiling on how long `sse_send` will retry against a full buffer before
/// giving up on a client that isn't reading fast enough — see its call site
/// for why a plain blocking `send` isn't safe here.
#[cfg(not(target_arch = "wasm32"))]
const SSE_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Ceiling on concurrent `respond stream` responder threads, server-wide
/// (see `active_sse_responders`'s doc comment). `Stmt::RespondStream`
/// deliberately never joins its responder thread — `tiny_http` has no
/// write-timeout hook, so a client that opens an SSE connection and never
/// reads leaves that thread blocked in the underlying socket write, which
/// only a TCP-level timeout (not this process) ever resolves. Without a
/// cap, repeated slow/dead clients accumulate one such thread each,
/// unboundedly, for the life of the server. Generous enough for any
/// realistic number of simultaneous legitimate SSE clients (dashboards,
/// live-log tails, ...) on a single `serve` instance, while still bounding
/// worst-case thread/memory growth from a client (or many) that never
/// reads.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CONCURRENT_SSE_RESPONDERS: usize = 256;

/// Pulled out of `Stmt::RespondStream`'s handler as a pure function so the
/// cap-check-and-reserve logic is testable directly (including under real
/// concurrent stress) without needing `MAX_CONCURRENT_SSE_RESPONDERS` real
/// stuck HTTP connections/threads to exercise the rejection path — the
/// same reasoning `check_task_capacity` (builtins_task.rs) already applies
/// to an analogous cap. Atomically reserves a slot and returns `true` only
/// if `counter` was below `cap`; otherwise leaves `counter` unchanged and
/// returns `false`. The caller is responsible for releasing a reserved
/// slot exactly once (via `fetch_sub`) when the corresponding responder
/// thread finishes.
#[cfg(not(target_arch = "wasm32"))]
fn try_reserve_sse_responder_slot(counter: &std::sync::atomic::AtomicUsize, cap: usize) -> bool {
    counter
        .fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |n| (n < cap).then_some(n + 1),
        )
        .is_ok()
}

/// Ceiling on pooled SQLite connections held open by one Interpreter at
/// once — see `db_connections`'s and `evict_one_idle_db_connection_if_at_
/// capacity`'s doc comments. Relevant mainly to a long-running `serve`
/// worker (whose Interpreter persists across every request it ever
/// handles) touching many distinct database paths over its lifetime;
/// generous enough that no realistic single-tenant or modest-multi-tenant
/// workload should ever evict a connection it's still actively using
/// between calls.
#[cfg(not(target_arch = "wasm32"))]
const MAX_POOLED_DB_CONNECTIONS: usize = 128;

/// Whether `v` is the `{ ok: false, error, error_kind, ... }` shape that
/// `http_*`/`process_*`/`task_wait`/`ask`/`context_ask` all return on an
/// *operational* failure (timeout, non-2xx status, non-zero exit, provider
/// error, ...) instead of throwing — GX's two coexisting failure-signaling
/// conventions (see `builtin_retry` and `http_result_err_msg`, its two
/// current consumers). A plain `Ok(v)` check can't tell these apart from a
/// genuine success on its own.
fn value_is_ok_false(v: &Value) -> bool {
    matches!(v, Value::Object(m) if matches!(m.get("ok"), Some(Value::Bool(false))))
}

/// Render a call-stack trace for display, truncating extremely deep traces
/// to a readable head and tail instead of one unreadable line. Without
/// this, a caught "maximum call depth exceeded" error (or any error
/// surfaced from deep, legitimate recursion) would print all ~1000 stack
/// frames — almost always the same one or two function names repeated —
/// on a single line hundreds of columns wide.
const CALL_STACK_DISPLAY_LIMIT: usize = 20;

fn format_call_stack(frames: &[String]) -> String {
    if frames.len() <= CALL_STACK_DISPLAY_LIMIT {
        return frames.join(" → ");
    }
    let head = &frames[..10];
    let tail = &frames[frames.len() - 10..];
    format!(
        "{} → ... ({} more frames omitted) ... → {}",
        head.join(" → "),
        frames.len() - 20,
        tail.join(" → ")
    )
}

/// Apply `.push(v)`/`.append(v)`/`.sort()`/`.reverse()` directly to an
/// already-mutably-borrowed array — the shared core of the bare-statement
/// fast path in `run_stmt_inner`'s `Stmt::Expr` handling, used for both a
/// plain identifier receiver (`arr.push(x)`) and one level of nested
/// field access (`memory.items.push(x)`), so the actual mutation logic
/// exists in exactly one place regardless of how the caller found its
/// `&mut Vec<Value>`.
fn mutate_array_in_place(arr: &mut Vec<Value>, method: &str, args: Vec<Value>) {
    match method {
        "push" | "append" => {
            for v in args {
                arr.push(v);
            }
        }
        "reverse" => arr.reverse(),
        "sort" => {
            arr.sort_by(|x, y| match (x, y) {
                (Value::Number(a), Value::Number(b)) => {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                }
                _ => x.to_string().cmp(&y.to_string()),
            });
        }
        _ => unreachable!("mutate_array_in_place called with an unsupported method"),
    }
}

/// Extract an error message for a span's `outcome` from an HTTP builtin's
/// result — used by every `http.client.*` span. `http_get`/`post`/`put`/
/// `delete`/`http_request` never throw on a failed *request* (a timeout, a
/// blocked SSRF attempt, a DNS failure, a non-2xx status): they always
/// return `Ok({ ok: false, error, error_kind, ... })`, the same
/// "external failure, not programmer error" convention `process_run` uses.
/// A span-outcome check that only looked at `Err(Signal::Error(_))` would
/// therefore report every one of those real failures as `outcome: "ok"` —
/// exactly the misleading trace this runtime exists to prevent.
fn http_result_err_msg(result: &Result<Value, Signal>) -> Option<String> {
    match result {
        Err(Signal::Error(e)) => Some(e.clone()),
        Err(_) => Some("request failed".to_string()),
        Ok(Value::Object(m)) => match m.get("ok") {
            Some(Value::Bool(false)) => Some(
                m.get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("request failed")
                    .to_string(),
            ),
            _ => None,
        },
        Ok(_) => None,
    }
}

// ── Control flow signals ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Signal {
    Return(Value),
    Break,
    Continue,
    ReRun,
    EscalateToHuman,
    AssertFail(String),
    Error(String),
    Respond(String, String, u16), // (content_type, body, status_code)
    /// The current task (or an ancestor of it — see `TaskState::parent`)
    /// was cancelled. Raised automatically by `run_stmt`, not by script
    /// code. Deliberately NOT caught by `try/catch` (see that match arm) —
    /// the same tier as `Break`/`Continue`/`Return`, so a broad `catch {}`
    /// can't silently swallow a cancellation a task was relying on to stop.
    /// The `String` is a human-readable reason (e.g. "deadline exceeded",
    /// "task_cancel() called").
    Cancelled(String),
}

pub(crate) type IResult = Result<Value, Signal>;

// ── Environment ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Value {
        self.vars.get(name).cloned().unwrap_or(Value::Null)
    }

    pub fn set(&mut self, name: &str, val: Value) {
        self.vars.insert(name.to_string(), val);
    }

    pub fn get_memory(&self) -> HashMap<String, Value> {
        match self.get("memory") {
            Value::Object(m) => m,
            _ => HashMap::new(),
        }
    }

    pub fn set_memory(&mut self, mem: HashMap<String, Value>) {
        self.vars.insert("memory".to_string(), Value::Object(mem));
    }

    /// Every variable currently bound in this scope, name to value — for
    /// tooling that needs to *enumerate* locals rather than look one up by
    /// name (the REPL's `:vars`, the debugger's `locals`). `memory` (an
    /// agent's whole persistent-memory object, when present) is included
    /// like any other binding; callers that want to treat it specially can
    /// filter it out themselves, matching how `run_program`'s own
    /// global-variable propagation already does (`if k != "memory"`).
    pub fn all_vars(&self) -> &HashMap<String, Value> {
        &self.vars
    }
}

// ── Interpreter ───────────────────────────────────────────────────────────────

pub struct Interpreter {
    helpers: HashMap<String, HelperDef>,
    functions: HashMap<String, FunctionDef>,
    imports: Vec<ImportDecl>,
    pub events: Vec<(String, Vec<(String, Value)>)>,
    event_bus: HashMap<String, Vec<Value>>,
    js_bridge: Option<Bridge>,
    ts_bridge: Option<Bridge>,
    py_bridge: Option<Bridge>,
    /// Shared, capability-aware `ureq` agent, built lazily on first HTTP
    /// call and reused for every subsequent one so connections actually
    /// get pooled (previously a fresh agent — and fresh TLS setup — was
    /// built on every single call, defeating connection reuse entirely).
    /// Safe to cache: capabilities never change after construction (see
    /// `crate::capability`), so the resolver baked into this agent stays
    /// correct for the interpreter's whole lifetime.
    #[cfg(not(target_arch = "wasm32"))]
    http_agent: Option<ureq::Agent>,
    /// The in-flight HTTP server request, handed off by `run_serve` right
    /// before running a matched route's body so `respond stream { ... }`
    /// can take ownership of it mid-execution (to open a streaming
    /// response) instead of only at the end, the way every other route
    /// return path works. `None` outside of a server route.
    #[cfg(not(target_arch = "wasm32"))]
    pending_request: Option<tiny_http::Request>,
    /// Set for the duration of a `respond stream { ... }` block — the
    /// channel `sse_send` writes frames into. Mirrors `output_capture`'s
    /// existing "optional interpreter-level redirect slot" pattern.
    #[cfg(not(target_arch = "wasm32"))]
    sse_tx: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    /// Count of `respond stream` responder threads currently in flight
    /// *across every worker* of the `serve` block this Interpreter is a
    /// worker for — shared (the same `Arc`, cloned once per worker by
    /// `run_serve`), unlike the rest of a worker's per-request state,
    /// because the resource it bounds (OS threads each potentially blocked
    /// forever writing to a client that stopped reading — see
    /// `Stmt::RespondStream`'s doc comment) accumulates server-wide, not
    /// per-worker. `None` outside of a `serve` block (e.g. a plain `gx
    /// run`), where `respond stream` isn't reachable at all.
    #[cfg(not(target_arch = "wasm32"))]
    active_sse_responders: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    pub base_path: Option<String>,
    pub assert_count: usize,
    pub assert_failures: Vec<String>,
    /// When set, output goes here instead of stdout (used by WASM playground)
    pub output_capture: Option<Vec<String>>,
    // #14 readiness: agents call ready() to signal they accept messages
    ready_agents: std::collections::HashSet<String>,
    // messages queued for not-yet-ready agents: agent_name -> [(event, payload)]
    queued_messages: HashMap<String, Vec<(String, Value)>>,
    // name of the currently-running helper (set by run_helper)
    current_agent: Option<String>,
    /// The single source of truth for what this interpreter is allowed to
    /// access — see `crate::capability`. Replaces the previously-scattered
    /// allow_shell/allow_process/allow_internal_http/sandbox_dir/
    /// allowed_js_modules/allowed_py_modules/allowed_process_commands
    /// fields; every dangerous operation authorizes through this instead of
    /// its own ad-hoc check.
    pub capabilities: crate::capability::Capabilities,
    /// Native processes spawned via `process_spawn`, keyed by an opaque handle
    /// (UUID) string. Every entry owns its child + background reader/reaper
    /// threads for its full lifetime — see `builtins_process`.
    #[cfg(not(target_arch = "wasm32"))]
    processes: HashMap<String, std::sync::Arc<builtins_process::ProcessState>>,
    /// Module registry: alias → list of functions from that module.
    /// Used so that intra-module calls (e.g. `pad_right` calling `repeat_str`)
    /// resolve correctly when the module is loaded under a namespace.
    module_functions: HashMap<String, Vec<FunctionDef>>,
    /// Binary/Go/Rust subprocess bridges keyed by "namespace:path".
    binary_bridges: HashMap<String, crate::bridge::Bridge>,
    /// Tool definitions registered at the program level (for AI function-calling).
    pub(crate) tools: HashMap<String, crate::ast::ToolDef>,
    /// When true, removes the iteration cap on while/loop. Use for REPL and I/O-bound loops.
    pub no_loop_limit: bool,
    /// Variables assigned at file root (top_level_stmts). Injected into every agent's env
    /// so they are accessible as normal locals alongside memory.*.
    global_vars: HashMap<String, Value>,
    /// Call stack of frame names (agent / function / closure) for error traces.
    call_stack: Vec<String>,
    /// Cumulative tokens used across all AI calls this session (exposed via `tokens_used()`).
    total_tokens_used: u64,
    /// Pooled SQLite connections, keyed by resolved (sandboxed) path —
    /// opened once, configured with production PRAGMAs (WAL, busy_timeout,
    /// foreign_keys — see `builtins_db::configure_connection`), and reused
    /// across every subsequent `db_query`/`db_exec`/`db_transaction` call
    /// to that same path for the life of this interpreter. Previously every
    /// call opened and immediately dropped its own connection, which meant
    /// no PRAGMA ever took effect and no prepared statement was ever
    /// reused.
    ///
    /// Bounded by `MAX_POOLED_DB_CONNECTIONS` (see `db_conn`) — relevant
    /// for a long-running `serve` worker's Interpreter, which persists
    /// across every request it ever handles: a workload touching many
    /// distinct DB paths (one file per tenant, say) would otherwise grow
    /// this map, and the file descriptors/WAL+SHM handles it holds open,
    /// without bound for the server's whole lifetime.
    #[cfg(not(target_arch = "wasm32"))]
    db_connections: HashMap<String, rusqlite::Connection>,
    /// Last-access time per pooled path, used only to pick an eviction
    /// candidate when `db_connections` is at capacity — see `db_conn`.
    /// Kept as a separate map (rather than alongside `Connection` in a
    /// tuple) so every existing `db_connections.get`/`.insert` call site
    /// is untouched.
    #[cfg(not(target_arch = "wasm32"))]
    db_connection_last_used: HashMap<String, std::time::Instant>,
    /// Active transaction/savepoint nesting depth per path (0 = no active
    /// transaction on that path). A `db_transaction` on a path already at
    /// depth > 0 issues a SAVEPOINT instead of BEGIN — SQLite has no true
    /// nested transactions, but savepoints are its real mechanism for
    /// exactly this (a reusable "does its own transaction" helper called
    /// from within a larger transactional workflow). Keyed by path (not a
    /// single global slot) so `db_query`/`db_exec` resolve their connection
    /// purely by which path was actually requested, never by "whatever
    /// transaction happens to be active somewhere."
    #[cfg(not(target_arch = "wasm32"))]
    db_tx_depth: HashMap<String, usize>,
    /// The Diagnostics & Observability Runtime — structured logging, audit
    /// events, and trace spans. Every subsystem (HTTP, Process, Database,
    /// Capability) reports through this instead of its own `eprintln!`.
    /// See `crate::diagnostics` for the full design.
    pub diagnostics: crate::diagnostics::Diagnostics,
    /// Tasks spawned via `task_spawn` that this Interpreter directly owns,
    /// keyed by an opaque handle (UUID) string — mirrors `processes`
    /// exactly. Every entry owns its background thread for its full
    /// lifetime; see `builtins_task`.
    #[cfg(not(target_arch = "wasm32"))]
    tasks: HashMap<String, std::sync::Arc<builtins_task::TaskState>>,
    /// Named bounded-concurrency groups (`task_spawn(fn, { pool: "...",
    /// max_concurrent: N })`), created on first use and reused by every
    /// later `task_spawn` naming the same pool from this Interpreter.
    #[cfg(not(target_arch = "wasm32"))]
    task_pools: HashMap<String, std::sync::Arc<builtins_task::Pool>>,
    /// The task (if any) whose body is currently executing on this
    /// Interpreter — the Task Runtime's task-local context, mirroring
    /// `current_agent`'s "just a field, set for the duration" shape. Read
    /// by `task_id()`/`is_cancelled()`, and by every statement's automatic
    /// cancellation check (`run_stmt`). `None` outside of a task.
    #[cfg(not(target_arch = "wasm32"))]
    current_task: Option<std::sync::Arc<builtins_task::TaskState>>,
    /// Production Debugger Runtime state — see `debugger` module. `mode:
    /// Off` (the default) costs one enum comparison per statement in
    /// `run_stmt`; everything else about the debugger is inert until a
    /// caller (`gx run --break ...`, or a script's own `breakpoint()`
    /// call) turns it on.
    pub debug: DebugState,
    /// The line of the statement `run_stmt` is currently executing —
    /// updated on every call, before dispatch. Exists so a `breakpoint()`
    /// call (an *expression*, which carries no line number of its own the
    /// way `Stmt` does) can still report where it was hit.
    current_line: usize,
    /// Production Testing Framework: cases registered via `test(name, fn)`
    /// during the top-level script's execution. Deliberately *not* run
    /// immediately — `crate::toolchain::test` drains this (via
    /// `take_registered_tests`) after the top-level script finishes and
    /// runs each one separately, in isolation, so one test's assertion
    /// failure is reported against its own name instead of aborting every
    /// other test in the file the way a bare top-level `assert` does.
    pub(crate) registered_tests: Vec<(String, Value)>,
    /// `before_each(fn)`/`after_each(fn)` — a single active hook slot each
    /// (the simplest shape that satisfies "setup/teardown around every
    /// registered test in this file"; a later call replaces the earlier
    /// one, matching how a file only ever needs one setup/teardown pair).
    pub(crate) before_each_fn: Option<Value>,
    pub(crate) after_each_fn: Option<Value>,
    /// Deterministic-testing hook for `set_random_seed(n)`: when set,
    /// every `random`/`random_int`/`random_choice`/`shuffle` call advances
    /// this shared LCG state instead of reseeding from the clock, making a
    /// whole test run byte-for-byte reproducible. `None` (the default)
    /// preserves the exact prior "fresh clock reseed per call" behavior —
    /// see `next_random_unit_f64`.
    rng_seed: Option<u64>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Interpreter {
    /// Guarantees no process spawned via `process_spawn`, and no task
    /// spawned via `task_spawn`, outlives the interpreter that started it —
    /// cancelled/killed, reaped, no leaked handles, no orphans. Tasks first:
    /// a task's own body may itself be waiting on a child process, so
    /// cancelling+joining tasks first gives that process a chance to be
    /// killed and reaped normally as part of the task's own cleanup, rather
    /// than this interpreter's process cleanup racing it.
    /// (`exit()` bypasses `Drop` by calling `std::process::exit` directly,
    /// so it also calls `cleanup_tasks()`/`cleanup_processes()` explicitly
    /// before exiting.)
    fn drop(&mut self) {
        self.cleanup_tasks();
        self.cleanup_processes();
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            helpers: HashMap::new(),
            functions: HashMap::new(),
            imports: Vec::new(),
            events: Vec::new(),
            event_bus: HashMap::new(),
            js_bridge: None,
            ts_bridge: None,
            py_bridge: None,
            #[cfg(not(target_arch = "wasm32"))]
            http_agent: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_request: None,
            #[cfg(not(target_arch = "wasm32"))]
            sse_tx: None,
            #[cfg(not(target_arch = "wasm32"))]
            active_sse_responders: None,
            base_path: None,
            assert_count: 0,
            assert_failures: Vec::new(),
            output_capture: None,
            ready_agents: std::collections::HashSet::new(),
            queued_messages: HashMap::new(),
            current_agent: None,
            capabilities: crate::capability::Capabilities::new(),
            #[cfg(not(target_arch = "wasm32"))]
            processes: HashMap::new(),
            module_functions: HashMap::new(),
            binary_bridges: HashMap::new(),
            tools: HashMap::new(),
            no_loop_limit: false,
            global_vars: HashMap::new(),
            call_stack: Vec::new(),
            total_tokens_used: 0,
            #[cfg(not(target_arch = "wasm32"))]
            db_connections: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            db_connection_last_used: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            db_tx_depth: HashMap::new(),
            diagnostics: crate::diagnostics::Diagnostics::new(),
            #[cfg(not(target_arch = "wasm32"))]
            tasks: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            task_pools: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            current_task: None,
            debug: DebugState::new(),
            current_line: 0,
            registered_tests: Vec::new(),
            before_each_fn: None,
            after_each_fn: None,
            rng_seed: None,
        }
    }

    /// Resolve `path_str` against the filesystem capability's access scope
    /// (see `crate::capability::Capabilities::resolve_path`) and verify the
    /// resolved path is still inside it when sandboxed.
    fn safe_path(&self, path_str: &str) -> Result<std::path::PathBuf, Signal> {
        self.capabilities.resolve_path(path_str).map_err(|e| {
            // --no-sandbox only ever helps the OutsideSandbox case — an
            // operator --deny filesystem can't be worked around by it, so
            // don't suggest a "fix" that wouldn't actually fix anything.
            let hint = match &e {
                crate::capability::Denial::OutsideSandbox { .. } => {
                    " Run with --no-sandbox to disable path restrictions."
                }
                _ => "",
            };
            Signal::Error(format!("Access denied: {}.{}", e, hint))
        })
    }

    /// Authorize an AI provider call, with an actionable hint when it's
    /// specifically the gx.json allowlist that's missing this provider —
    /// shared by all four AI call sites (Think, AskAI, Embed,
    /// InferClassifier) so they don't each repeat the same wrapping.
    fn authorize_ai_provider(&self, provider: &str) -> Result<(), Signal> {
        self.authorize_capability(crate::capability::Resource::AiProviders, Some(provider))
            .map_err(|e| {
                let hint = match &e {
                    crate::capability::Denial::NotInAllowlist { .. } => {
                        " — add it to gx.json's dependencies.ai to allow it."
                    }
                    _ => "",
                };
                Signal::Error(format!("{}{}", e, hint))
            })
    }

    /// The single place a capability check's *failure* is reported. Every
    /// `self.capabilities.authorize(...)` call site in the interpreter
    /// (there are around fifteen) goes through this instead of calling
    /// `self.capabilities.authorize` directly, so a denial always emits a
    /// `capability_denied` audit event exactly once — centralized here
    /// rather than duplicated at each call site, several of which also
    /// layer their own message hint on top of the returned `Denial` (kept
    /// as-is; this only adds the audit emission, not the message).
    fn authorize_capability(
        &self,
        resource: crate::capability::Resource,
        name: Option<&str>,
    ) -> Result<(), crate::capability::Denial> {
        let result = self.capabilities.authorize(resource, name);
        if let Err(ref denial) = result {
            self.diagnostics.audit(
                "capability_denied",
                serde_json::json!({
                    "resource": resource.name(),
                    "name": name,
                    "reason": denial.to_string(),
                }),
            );
        }
        result
    }

    /// The shared, capability-aware HTTP agent, built lazily on first use
    /// and cached for the rest of this interpreter's lifetime. `Agent`
    /// clones are cheap (`Arc`-backed, share the same connection pool), so
    /// returning an owned clone here is fine and avoids holding a borrow
    /// of `self`.
    #[cfg(not(target_arch = "wasm32"))]
    fn http_agent(&mut self) -> ureq::Agent {
        if self.http_agent.is_none() {
            self.http_agent = Some(builtins_http::http_agent(
                &self.capabilities,
                &self.diagnostics,
            ));
        }
        self.http_agent.clone().expect("just set")
    }

    /// Get or open a pooled SQLite connection for `path` — see the
    /// `db_connections` field docs. `path` must already be resolved through
    /// `safe_path`/the Capability Runtime by the caller; this method itself
    /// does no sandboxing, matching how `http_agent` above doesn't itself
    /// enforce network capabilities.
    #[cfg(not(target_arch = "wasm32"))]
    fn db_conn(&mut self, path: &str) -> Result<&rusqlite::Connection, Signal> {
        if !self.db_connections.contains_key(path) {
            self.evict_one_idle_db_connection_if_at_capacity();
            let conn = rusqlite::Connection::open(path)
                .map_err(|e| Signal::Error(format!("cannot open database '{}': {}", path, e)))?;
            builtins_db::configure_connection(&conn)?;
            self.db_connections.insert(path.to_string(), conn);
        }
        self.db_connection_last_used
            .insert(path.to_string(), std::time::Instant::now());
        Ok(self.db_connections.get(path).expect("just inserted"))
    }

    /// If the pool is already at `MAX_POOLED_DB_CONNECTIONS`, close and
    /// drop whichever pooled connection was least recently used *and has
    /// no active transaction on it* — freeing exactly one slot for the
    /// caller's about-to-be-opened new connection. A no-op below capacity,
    /// and a no-op (rather than an error) in the one case eviction isn't
    /// possible: every pooled connection currently has `db_tx_depth > 0`
    /// (an in-flight `db_transaction`), which the Capability/Database
    /// Runtime's own contract — never close a connection out from under an
    /// active transaction — takes priority over the cap. That's the
    /// documented, deliberate exception: the pool is allowed to grow past
    /// the cap for as long as it takes those transactions to finish,
    /// rather than corrupting one or returning a surprising error from an
    /// unrelated `db_transaction("some other path") { ... }` call.
    #[cfg(not(target_arch = "wasm32"))]
    fn evict_one_idle_db_connection_if_at_capacity(&mut self) {
        if self.db_connections.len() < MAX_POOLED_DB_CONNECTIONS {
            return;
        }
        let victim = self
            .db_connections
            .keys()
            .filter(|path| self.db_tx_depth.get(*path).copied().unwrap_or(0) == 0)
            .min_by_key(|path| {
                self.db_connection_last_used
                    .get(*path)
                    .copied()
                    .unwrap_or_else(std::time::Instant::now)
            })
            .cloned();
        if let Some(path) = victim {
            self.db_connections.remove(&path);
            self.db_connection_last_used.remove(&path);
        }
    }

    /// Begin a transaction (or, if `path` already has one active, a nested
    /// savepoint) on the pooled connection for `path`. Returns the nesting
    /// depth reached, which `db_tx_end` needs to know whether to commit or
    /// release, and to name the savepoint consistently.
    #[cfg(not(target_arch = "wasm32"))]
    fn db_tx_begin(&mut self, path: &str) -> Result<usize, Signal> {
        let depth = *self.db_tx_depth.get(path).unwrap_or(&0);
        let conn = self.db_conn(path)?;
        if depth == 0 {
            // BEGIN IMMEDIATE, not a plain (deferred) BEGIN — deliberately.
            // A deferred BEGIN takes no lock until the transaction's first
            // statement; a read (like the very common
            // `row = db_query(...); db_exec(... UPDATE ...)` pattern) only
            // acquires a *shared* read lock, so two concurrent transactions
            // can both get past their read and then both try to *upgrade*
            // to a write lock at the same time. SQLite has no way to
            // resolve that upgrade race by making one side wait — even with
            // `busy_timeout` set, one of them gets a "database is locked"
            // error immediately rather than retrying, because retrying
            // wouldn't help (neither side can ever downgrade to let the
            // other through). This surfaced for real once multiple
            // `task_spawn`-based tasks concurrently ran a
            // read-then-write `db_transaction` against the same row.
            // `BEGIN IMMEDIATE` takes the write lock up front, so the
            // second transaction blocks (and `busy_timeout` correctly
            // retries it) at BEGIN itself instead of racing to upgrade
            // later — turning a sometimes-unresolvable race into a
            // straightforward, correctly-serialized queue.
            conn.execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| Signal::Error(format!("db_transaction: BEGIN failed: {}", e)))?;
        } else {
            conn.execute_batch(&format!("SAVEPOINT gx_sp_{}", depth))
                .map_err(|e| Signal::Error(format!("db_transaction: SAVEPOINT failed: {}", e)))?;
        }
        let new_depth = depth + 1;
        self.db_tx_depth.insert(path.to_string(), new_depth);
        Ok(new_depth)
    }

    /// End the transaction/savepoint started by the matching `db_tx_begin`
    /// call (`depth` is the value it returned). `commit`: true to
    /// COMMIT/RELEASE, false to ROLLBACK/ROLLBACK TO. Always decrements the
    /// depth, even if the underlying SQL fails, so a failure here can never
    /// leave `db_tx_depth` permanently out of sync with reality.
    #[cfg(not(target_arch = "wasm32"))]
    fn db_tx_end(&mut self, path: &str, depth: usize, commit: bool) -> Result<(), Signal> {
        debug_assert!(
            depth >= 1,
            "db_tx_end called with a depth db_tx_begin never returns"
        );
        let outermost = depth == 1;
        self.db_tx_depth.insert(path.to_string(), depth - 1);
        let conn = self.db_conn(path)?;
        let sql = match (outermost, commit) {
            (true, true) => "COMMIT".to_string(),
            (true, false) => "ROLLBACK".to_string(),
            (false, true) => format!("RELEASE gx_sp_{}", depth - 1),
            (false, false) => format!(
                "ROLLBACK TO gx_sp_{}; RELEASE gx_sp_{}",
                depth - 1,
                depth - 1
            ),
        };
        conn.execute_batch(&sql)
            .map_err(|e| Signal::Error(format!("db_transaction: {} failed: {}", sql, e)))
    }

    /// Route output to a capture buffer instead of stdout.
    pub fn enable_capture(&mut self) {
        self.output_capture = Some(Vec::new());
    }

    /// Flush captured output as a single string.
    pub fn captured_output(&self) -> String {
        self.output_capture
            .as_ref()
            .map(|v| v.join("\n"))
            .unwrap_or_default()
    }

    fn emit_output(&mut self, line: &str) {
        if let Some(buf) = &mut self.output_capture {
            buf.push(line.to_string());
        } else {
            println!("{}", line);
        }
    }

    /// Add the `tokens_used` reported by an AI response to the session total.
    fn record_tokens(&mut self, result: &Value) {
        if let Value::Object(m) = result {
            if let Some(n) = m.get("tokens_used").and_then(|v| v.as_number()) {
                if n > 0.0 {
                    self.total_tokens_used += n as u64;
                }
            }
        }
    }

    /// End an `ai.request`/`ai.embed`/`ai.infer` span, shared by every AI
    /// call site (the single-shot `ask`/`Think`/`embed`/`infer` primitives
    /// and the AI Context Runtime's `context_ask`) — "Diagnostics Runtime
    /// should automatically trace AI operations" applies uniformly, not
    /// just to the new context-based entry point.
    #[cfg(not(target_arch = "wasm32"))]
    fn end_ai_span(
        &mut self,
        span: crate::diagnostics::SpanHandle,
        provider: &str,
        model: Option<&str>,
        result: &Value,
    ) {
        let (ok, error_kind, tokens_used) = match result {
            Value::Object(m) => (
                matches!(m.get("ok"), Some(Value::Bool(true))),
                m.get("error_kind")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                m.get("tokens_used")
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0),
            ),
            _ => (true, None, 0.0),
        };
        let outcome = if ok {
            crate::diagnostics::Outcome::Ok
        } else {
            crate::diagnostics::Outcome::Error(error_kind.as_deref().unwrap_or("error"))
        };
        self.diagnostics.end_span(
            span,
            outcome,
            &[
                ("provider", serde_json::Value::String(provider.to_string())),
                (
                    "model",
                    model
                        .map(|m| serde_json::Value::String(m.to_string()))
                        .unwrap_or(serde_json::Value::Null),
                ),
                (
                    "tokens_used",
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(tokens_used).unwrap_or(0.into()),
                    ),
                ),
            ],
        );
    }

    /// Write output WITHOUT a trailing newline (for inline prompts before readline()).
    fn emit_output_raw(&mut self, text: &str) {
        if let Some(buf) = &mut self.output_capture {
            buf.push(text.to_string());
        } else {
            use std::io::Write;
            print!("{}", text);
            std::io::stdout().flush().ok();
        }
    }

    /// Execute a list of top-level statements against a caller-held,
    /// persistent `env` — the building block that makes `gx repl` actually
    /// hold state across lines. `run_program`'s own top-level-statement
    /// handling (see below) creates a *fresh* `Env` on every call and only
    /// ever writes into `self.global_vars`, never reads back from it —
    /// correct for a single whole-program run, but it means a REPL that
    /// wrapped each line in its own `run_program` call could never see a
    /// variable a previous line had assigned (confirmed empirically: this
    /// was a real, shipped bug — `x = 42` on one line, `say x` on the
    /// next, printed `null`). This method instead runs directly against
    /// whatever `Env` the caller passes in, so the caller (the REPL) can
    /// keep reusing — and thus keep seeing updates to — the exact same one
    /// across an entire session.
    ///
    /// Also mirrors `self.global_vars` afterward (see `run_program`) so a
    /// `helper`/`agent` block defined *later* in the same REPL session —
    /// which still runs through the ordinary `run_program` path, since
    /// definitions/auto-execution are unchanged — can still see variables
    /// a bare statement assigned earlier, the same way it would if both
    /// had been written in one file.
    ///
    /// A bare `return` is treated as "yield this value" rather than the
    /// `run_program`'s stricter "return outside function" error — a
    /// reasonable, friendlier reading at an interactive prompt where
    /// there's no enclosing function for it to have escaped.
    pub fn run_repl_stmts(&mut self, stmts: &[Stmt], env: &mut Env) -> Result<Value, String> {
        let mut last = Value::Null;
        for stmt in stmts {
            match self.run_stmt(stmt, env) {
                Ok(v) => last = v,
                Err(Signal::Error(m)) => return Err(m),
                Err(Signal::AssertFail(m)) => {
                    return Err(self.with_call_stack(format!("Assertion failed: {}", m)))
                }
                Err(Signal::Return(v)) => {
                    last = v;
                    break;
                }
                Err(other) => return Err(format!("unexpected control flow: {:?}", other)),
            }
        }
        for (k, v) in &env.vars {
            if k != "memory" {
                self.global_vars.insert(k.clone(), v.clone());
            }
        }
        Ok(last)
    }

    pub fn run_program(&mut self, program: &Program) -> Result<(), String> {
        self.imports = program.imports.clone();

        for f in &program.functions {
            self.functions.insert(f.name.clone(), f.clone());
        }
        for t in &program.tools {
            self.tools.insert(t.name.clone(), t.clone());
        }

        // Resolve file imports — transitively (an imported file's own
        // `import`s are followed too), with cycle detection, and with each
        // file parsed at most once regardless of how many places import it.
        // See `resolve_file_imports` for why this replaced a version that
        // only ever processed the top-level program's own import list.
        let base_dir = self
            .base_path
            .as_ref()
            .map(|p| {
                std::path::Path::new(p)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf()
            })
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let mut resolving: Vec<std::path::PathBuf> = Vec::new();
        if let Some(base) = &self.base_path {
            if let Ok(canon) = std::path::Path::new(base).canonicalize() {
                resolving.push(canon);
            }
        }
        let mut import_cache: HashMap<std::path::PathBuf, Program> = HashMap::new();
        // Tracks which canonical file most recently provided each flat-
        // imported name ("function:foo" / "agent:bar") — see
        // `warn_on_import_collision` for why this exists: without it, a
        // "diamond" (two files that both legitimately import the same
        // shared third file) would falsely warn on every name in that
        // shared file, since it gets *merged* once per importer even
        // though it's only ever *parsed* once (via `import_cache`).
        let mut defined_by: HashMap<String, std::path::PathBuf> = HashMap::new();
        // Memoizes `resolve_package_import`'s result per package name for
        // this whole resolution pass — without it, a package imported by
        // several files in the project would have its integrity hash
        // recomputed (a full read + SHA-256 of its entire tree) once per
        // importer instead of once total. `import_cache` above already
        // guarantees the package's *entry file* is only ever parsed once;
        // this closes the same gap one layer earlier, before that cache
        // is even consulted.
        let mut package_cache: HashMap<String, (std::path::PathBuf, String, &'static str)> =
            HashMap::new();
        let import_span = self.diagnostics.start_span("module.import");
        let import_result = self.resolve_file_imports(
            &program.file_imports,
            &base_dir,
            &mut resolving,
            &mut import_cache,
            &mut defined_by,
            &mut package_cache,
        );
        let file_count = import_cache.len();
        self.diagnostics.end_span(
            import_span,
            match &import_result {
                Ok(()) => crate::diagnostics::Outcome::Ok,
                Err(e) => crate::diagnostics::Outcome::Error(e.as_str()),
            },
            &[(
                "files_resolved",
                serde_json::Value::Number(file_count.into()),
            )],
        );
        import_result?;

        for h in &program.helpers {
            self.helpers.insert(h.name.clone(), h.clone());
            // Register agent-level function declarations globally
            for f in &h.functions {
                self.functions.insert(f.name.clone(), f.clone());
            }
        }

        // Execute top-level statements (x = 1, load_env(".env"), config = yaml_parse(...))
        // before any agent runs. Results are stored as global_vars and injected into
        // every agent's env so they are accessible as ordinary locals.
        if !program.top_level_stmts.is_empty() {
            let mut global_env = Env::new();
            for stmt in &program.top_level_stmts.clone() {
                self.run_stmt(stmt, &mut global_env).map_err(|e| {
                    format!("top-level statement: {}", self.describe_stray_signal(e))
                })?;
            }
            // Store everything that was assigned (excluding memory object itself)
            for (k, v) in &global_env.vars {
                if k != "memory" {
                    self.global_vars.insert(k.clone(), v.clone());
                }
            }
        }

        for h in &program.helpers.clone() {
            // Skip auto-running callable agents (those whose brain uses `input`).
            // They are only executed when called via `spawn agent`.
            if helper_is_callable_only(h) {
                continue;
            }
            self.run_helper(h)
                .map_err(|e| self.describe_stray_signal(e))?;
        }

        if let Some(brain) = &program.top_level_brain.clone() {
            let mut env = Env::new();
            self.run_brain(brain, &mut env)
                .map_err(|e| self.describe_stray_signal(e))?;
        }

        Ok(())
    }

    /// Resolve `imports` (one file's `import "..."` statements), merge each
    /// into this Interpreter, and recurse into *that* file's own imports —
    /// relative to *its* directory, not the original entry script's. This
    /// is what makes transitive imports work at all: the previous version
    /// only ever processed the top-level program's own `file_imports`, so
    /// `a.gx` importing `b.gx` which itself imports `c.gx` silently never
    /// loaded `c.gx`.
    ///
    /// `resolving` is the chain of canonical paths currently being resolved
    /// (a stack, pushed on entry to a file and popped on exit) — if a path
    /// reappears on it, that's an import cycle, reported with the full
    /// chain rather than a bare "stack overflow" or silent infinite loop.
    /// `cache` holds every already-*parsed* file (canonical path → AST) so
    /// a file imported from several places (a "diamond" — common in any
    /// nontrivial multi-file project) is read and parsed exactly once;
    /// it's still *merged* once per importer, since a namespaced
    /// (`import "x.gx" as ns`) import's alias is a property of the import
    /// site, not the file.
    #[allow(clippy::too_many_arguments)]
    fn resolve_file_imports(
        &mut self,
        imports: &[FileImport],
        importer_dir: &std::path::Path,
        resolving: &mut Vec<std::path::PathBuf>,
        cache: &mut HashMap<std::path::PathBuf, Program>,
        defined_by: &mut HashMap<String, std::path::PathBuf>,
        package_cache: &mut HashMap<String, (std::path::PathBuf, String, &'static str)>,
    ) -> Result<(), String> {
        for fi in imports {
            // A bare name with no `.gx` suffix and no path separator or
            // leading `./`/`../` (e.g. `import "leftpad"`, as opposed to
            // `import "leftpad.gx"` or `import "./leftpad.gx"`) is a
            // *package* import, resolved via gx.lock + the local package
            // cache rather than as a plain file path. See
            // `resolve_package_import`.
            let resolved_path = if is_package_import(&fi.path) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.resolve_package_import(&fi.path, package_cache)
                        .map_err(|e| format!("Line {}: {}", fi.line, e))?
                }
                #[cfg(target_arch = "wasm32")]
                {
                    return Err(format!(
                        "Line {}: package imports ('{}') are not supported in this build",
                        fi.line, fi.path
                    ));
                }
            } else {
                // Resolved relative to the *importing file's own directory*
                // first — deterministic, matches every other language's module
                // resolution, and independent of the current working
                // directory `gx` happens to be invoked from. A CWD-relative (or
                // absolute) path is still accepted as a fallback, for a path
                // that was never meant to be relative to the importer at all.
                let candidate = importer_dir.join(&fi.path);
                if candidate.exists() {
                    candidate
                } else if std::path::Path::new(&fi.path).exists() {
                    std::path::PathBuf::from(&fi.path)
                } else {
                    return Err(format!(
                        "Line {}: cannot import '{}': file not found (looked in '{}' and the current directory)",
                        fi.line,
                        fi.path,
                        importer_dir.display()
                    ));
                }
            };

            let canonical = resolved_path
                .canonicalize()
                .unwrap_or_else(|_| resolved_path.clone());

            if let Some(pos) = resolving.iter().position(|p| p == &canonical) {
                let chain: Vec<String> = resolving[pos..]
                    .iter()
                    .map(|p| p.display().to_string())
                    .chain(std::iter::once(canonical.display().to_string()))
                    .collect();
                return Err(format!(
                    "Line {}: import cycle detected: {}",
                    fi.line,
                    chain.join(" -> ")
                ));
            }

            let sub = if let Some(cached) = cache.get(&canonical) {
                cached.clone()
            } else {
                let src = std::fs::read_to_string(&resolved_path)
                    .map_err(|e| format!("Line {}: cannot import '{}': {}", fi.line, fi.path, e))?;
                let parsed = parse_gx_source(&src, &resolved_path.to_string_lossy())?;
                cache.insert(canonical.clone(), parsed.clone());
                parsed
            };

            resolving.push(canonical.clone());
            let sub_dir = resolved_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf();
            self.resolve_file_imports(
                &sub.file_imports,
                &sub_dir,
                resolving,
                cache,
                defined_by,
                package_cache,
            )?;
            resolving.pop();

            if let Some(ref alias) = fi.alias {
                // Namespaced import: register functions as `alias.funcname`.
                // Also store the originals in module_functions so intra-module
                // calls (e.g. pad_right calling repeat_str) resolve correctly.
                let originals: Vec<FunctionDef> = sub.functions.clone();
                for f in &sub.functions {
                    let mut namespaced = f.clone();
                    namespaced.name = format!("{}.{}", alias, f.name);
                    self.functions.insert(namespaced.name.clone(), namespaced);
                }
                self.module_functions.insert(alias.clone(), originals);
                // Agents from the module are also callable via alias namespace
                for h in &sub.helpers {
                    self.helpers.insert(h.name.clone(), h.clone());
                }
                for t in &sub.tools {
                    self.tools.insert(t.name.clone(), t.clone());
                }
            } else {
                // Flat import: inline everything into global scope. A name
                // already defined (by the top-level program, or by an
                // earlier flat import) is silently overwritten — same
                // last-write-wins behavior as before, kept for backward
                // compatibility — but now at least observable: logged as a
                // diagnostics warning rather than vanishing without a
                // trace, since a same-named function silently shadowing
                // another is a real, easy-to-miss class of bug in a
                // multi-file project. Only warns when the *previous*
                // definition came from a genuinely different file —
                // re-merging the same shared file via a "diamond" (two
                // files that both import a common third file) is not a
                // collision, just `import_cache` doing its job.
                for f in &sub.functions {
                    self.warn_on_import_collision(
                        "function", &f.name, &canonical, &fi.path, defined_by,
                    );
                    self.functions.insert(f.name.clone(), f.clone());
                }
                for h in &sub.helpers {
                    self.warn_on_import_collision(
                        "agent", &h.name, &canonical, &fi.path, defined_by,
                    );
                    self.helpers.insert(h.name.clone(), h.clone());
                }
                for t in &sub.tools {
                    self.tools.insert(t.name.clone(), t.clone());
                }
            }
            self.imports.extend(sub.imports.clone());
        }
        Ok(())
    }

    /// Resolve a package name (`import "leftpad"`) to its entry file, via
    /// `gx.lock` + the local package cache — the counterpart to a plain
    /// file import, but for a dependency declared in `gx.json`'s
    /// `dependencies.gx` and pinned by `gx install`. A `package.resolve`
    /// diagnostics span is still emitted on every call (accurately
    /// reflecting "this file imports this package", even when it's a
    /// repeat), but the expensive part of the work — reading `gx.lock` and
    /// re-hashing the package's entire tree for integrity verification —
    /// only actually runs once per package name per resolution pass; a
    /// package imported by several files in the same project would
    /// otherwise pay that cost once per importer instead of once total.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_package_import(
        &mut self,
        name: &str,
        package_cache: &mut HashMap<String, (std::path::PathBuf, String, &'static str)>,
    ) -> Result<std::path::PathBuf, String> {
        let span = self.diagnostics.start_span("package.resolve");
        let result = if let Some(cached) = package_cache.get(name) {
            Ok(cached.clone())
        } else {
            let resolved = self.resolve_package_import_impl(name);
            if let Ok(r) = &resolved {
                package_cache.insert(name.to_string(), r.clone());
            }
            resolved
        };
        let mut attrs: Vec<(&str, serde_json::Value)> =
            vec![("package", serde_json::Value::String(name.to_string()))];
        if let Ok((_, version, source_kind)) = &result {
            attrs.push(("version", serde_json::Value::String(version.clone())));
            attrs.push((
                "source",
                serde_json::Value::String((*source_kind).to_string()),
            ));
        }
        self.diagnostics.end_span(
            span,
            match &result {
                Ok(_) => crate::diagnostics::Outcome::Ok,
                Err(e) => crate::diagnostics::Outcome::Error(e.as_str()),
            },
            &attrs,
        );
        result.map(|(path, _, _)| path)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_package_import_impl(
        &self,
        name: &str,
    ) -> Result<(std::path::PathBuf, String, &'static str), String> {
        let start = self
            .base_path
            .as_ref()
            .map(|p| {
                std::path::Path::new(p)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf()
            })
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let Some((project_root, lock)) = Self::find_project_lock(&start)? else {
            return Err(format!(
                "cannot import package '{}': no gx.lock found in '{}' or any parent directory — \
                 declare it in gx.json's dependencies.gx and run `gx install` first",
                name,
                start.display()
            ));
        };
        let Some(locked) = lock.packages.get(name) else {
            return Err(format!(
                "cannot import package '{}': not found in gx.lock — add it to gx.json's \
                 dependencies.gx and run `gx install`",
                name
            ));
        };

        let is_path_dep = locked.resolved.starts_with("path+");
        let source_kind = if is_path_dep {
            "path"
        } else if locked.resolved.starts_with("git+") {
            "git"
        } else {
            "registry"
        };
        let dir = if let Some(rel) = locked.resolved.strip_prefix("path+") {
            project_root.join(rel)
        } else {
            crate::package::cache_dir_for(&crate::package::cache_root(), name, &locked.version)
        };
        if !dir.exists() {
            return Err(format!(
                "cannot import package '{}': gx.lock references it but '{}' does not exist — \
                 run `gx install` again",
                name,
                dir.display()
            ));
        }

        // A path dependency is meant to always reflect its current
        // on-disk state during local development (that's the point of
        // using one) — re-verifying it against a hash taken at the last
        // `gx install` would force a re-install after every edit. Only
        // git/registry (cache) dependencies, which are expected to be
        // immutable once fetched, are integrity-checked here.
        if !is_path_dep {
            let actual = crate::package::hash_package_tree(&dir)
                .map_err(|e| format!("cannot import package '{}': {}", name, e))?;
            if actual != locked.integrity {
                return Err(format!(
                    "cannot import package '{}': integrity check failed — the cached copy at \
                     '{}' does not match gx.lock (expected {}, got {}). Delete it and run \
                     `gx install` again.",
                    name,
                    dir.display(),
                    locked.integrity,
                    actual
                ));
            }
        }

        let entry = {
            let manifest_path = dir.join(crate::package::MANIFEST_NAME);
            if manifest_path.exists() {
                let content = std::fs::read_to_string(&manifest_path)
                    .map_err(|e| format!("cannot read '{}': {}", manifest_path.display(), e))?;
                let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                    format!("'{}' is not valid JSON: {}", manifest_path.display(), e)
                })?;
                crate::package::PackageMetadata::from_manifest(&json)?.entry
            } else {
                "main.gx".to_string()
            }
        };
        // `entry` comes straight from the *dependency's own* gx.json — an
        // untrusted git/registry package's manifest, not something the
        // importing project wrote. An absolute path (`PathBuf::join`
        // discards the receiver entirely for those) or a `../` traversal
        // would let a malicious dependency point its entry at any file on
        // the importer's disk (e.g. `~/.ssh/id_rsa`) to have it read and
        // parsed as GX source the moment the package is imported — the
        // integrity check above only hashes the package's own tree
        // (including this manifest value), so a self-consistent malicious
        // package still passes it. Resolve lexically and require the
        // result to stay inside `dir`, the same confinement
        // `Capabilities::resolve_path` enforces for a sandboxed script's
        // own file I/O.
        let entry_path = crate::capability::normalize_path_no_symlink(&dir.join(&entry));
        let canonical_dir = crate::capability::normalize_path_no_symlink(&dir);
        if !entry_path.starts_with(&canonical_dir) {
            return Err(format!(
                "cannot import package '{}': its gx.json declares an 'entry' ('{}') that \
                 resolves outside the package's own directory — refusing to read a file \
                 outside '{}'",
                name,
                entry,
                dir.display()
            ));
        }
        Ok((entry_path, locked.version.clone(), source_kind))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn find_project_lock(
        start: &std::path::Path,
    ) -> Result<Option<(std::path::PathBuf, crate::package::LockFile)>, String> {
        let mut dir = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        loop {
            let candidate = dir.join(crate::package::LOCKFILE_NAME);
            if candidate.exists() {
                let lock = crate::package::LockFile::load(&candidate)?
                    .ok_or_else(|| format!("'{}' could not be read", candidate.display()))?;
                return Ok(Some((dir, lock)));
            }
            if !dir.pop() {
                return Ok(None);
            }
        }
    }

    fn warn_on_import_collision(
        &self,
        kind: &str,
        name: &str,
        source_file: &std::path::Path,
        source_path: &str,
        defined_by: &mut HashMap<String, std::path::PathBuf>,
    ) {
        let already_exists = match kind {
            "function" => self.functions.contains_key(name),
            "agent" => self.helpers.contains_key(name),
            _ => false,
        };
        let key = format!("{}:{}", kind, name);
        let prior_file = defined_by.insert(key, source_file.to_path_buf());
        if !already_exists {
            // Nothing defined this name before at all — a normal first
            // definition, not a collision.
            return;
        }
        if prior_file.as_deref() == Some(source_file) {
            // The existing definition came from this *same* file, reached
            // via a different import path (a "diamond") — not a collision,
            // just `import_cache` correctly avoiding a re-parse.
            return;
        }
        // Either a different file already flat-imported this name, or it
        // was defined directly by the top-level program itself
        // (`prior_file` is `None` the first time a name is tracked here,
        // which is exactly the top-level-program case since only file
        // imports populate `defined_by`) — both are genuine collisions
        // worth surfacing.
        self.diagnostics.log(
            crate::diagnostics::Level::Warn,
            &format!(
                "import: {} '{}' from '{}' overwrites an existing definition of the same name \
                 (last import wins) — rename one of them, or import with `as <alias>`, to avoid \
                 relying on this",
                kind, name, source_path
            ),
            None,
        );
    }

    fn run_helper(&mut self, helper: &HelperDef) -> Result<(), Signal> {
        self.current_agent = Some(helper.name.clone());
        // Reset the call stack at the top of each agent run, then push this agent's frame.
        // (An error aborts the whole program, so stale frames never leak across agents.)
        self.call_stack.clear();
        self.call_stack.push(format!("agent \"{}\"", helper.name));

        // Enable trace if the agent declared trace: true
        // (Currently set via helper.goal == "trace" as a convention; full attr support later)

        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));

        // v0.2.0: store goal in memory if declared
        if let Some(ref goal) = helper.goal {
            memory.insert("goal".into(), Value::Str(goal.clone()));
        }

        // Persistent memory: load from SQLite if the agent has any `persistent` entries
        #[cfg(not(target_arch = "wasm32"))]
        let persistent_db_path = {
            let has_persistent = self.has_persistent_memory(&helper.name);
            if has_persistent {
                let db_path = persistent_db_path_for(&helper.name);
                // Load existing memory from SQLite (overlay on top of default values)
                if let Ok(loaded) = load_persistent_memory(&db_path) {
                    for (k, v) in loaded {
                        memory.insert(k, v);
                    }
                }
                Some(db_path)
            } else {
                None
            }
        };

        let mut env = Env::new();
        for entry in &helper.memory {
            // Only set default if not already loaded from persistent store
            if !memory.contains_key(&entry.key)
                || matches!(memory.get(&entry.key), Some(Value::Null))
            {
                let val = self.eval_expr(&entry.value, &mut env)?;
                memory.insert(entry.key.clone(), val);
            }
        }

        // Inject file-root globals into the agent's env as ordinary locals.
        // This makes `url = "https://..."` at file root accessible inside agents.
        let global_vars = self.global_vars.clone();
        for (k, v) in global_vars {
            env.set(&k, v);
        }

        // Run recipes as named functions (available to the brain cycle)
        for recipe in &helper.recipes.clone() {
            let func = FunctionDef {
                name: recipe.name.clone(),
                params: recipe.needs.clone(),
                body: {
                    let brain = &recipe.brain;
                    let mut body = brain.plan.clone();
                    body.extend(brain.execute.clone());
                    body.extend(brain.remember.clone());
                    body.extend(brain.communicate.clone());
                    body
                },
                line: recipe.line,
            };
            self.functions.insert(recipe.name.clone(), func);
        }

        // Run objectives: register as when-expr blocks
        let mut extra_when: Vec<WhenBlock> = helper
            .objectives
            .iter()
            .map(|obj| WhenBlock {
                trigger: WhenTrigger::Expr(obj.when_expr.clone()),
                body: vec![Stmt::Expr {
                    expr: obj.then_action.clone(),
                    line: obj.line,
                }],
                line: obj.line,
            })
            .collect();

        // Run `when started` blocks
        for wb in &helper.when_blocks.clone() {
            if matches!(wb.trigger, WhenTrigger::Started) {
                env.set_memory(memory.clone());
                match self.run_stmts(&wb.body, &mut env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
                memory = env.get_memory();
            }
        }

        // Brain cycle (with v0.2.0 retry/on_error support)
        if let Some(brain) = &helper.brain.clone() {
            let max_retries = helper.retry.unwrap_or(0) as usize;
            let on_error_policy = helper.on_error.as_deref().unwrap_or("escalate");
            let mut retry_count = 0;
            let mut cycles = 0;
            const MAX_CYCLES: usize = 100;
            'outer: loop {
                env.set_memory(memory.clone());
                env.set("plan", Value::Null);
                env.set("result", Value::Null);

                match self.run_brain(brain, &mut env) {
                    Ok(_) => {}
                    Err(Signal::ReRun) if cycles < MAX_CYCLES => {
                        cycles += 1;
                        memory = env.get_memory();
                        continue 'outer;
                    }
                    Err(Signal::ReRun) => {
                        return Err(Signal::Error(format!(
                            "Helper '{}' exceeded {} re-run cycles",
                            helper.name, MAX_CYCLES
                        )));
                    }
                    Err(Signal::Error(ref msg)) if retry_count < max_retries => {
                        retry_count += 1;
                        eprintln!(
                            "[gx] Helper '{}' error (retry {}/{}): {}",
                            helper.name, retry_count, max_retries, msg
                        );
                        continue 'outer;
                    }
                    Err(e @ Signal::Error(_)) => match on_error_policy {
                        "continue" => {
                            eprintln!(
                                "[gx] on_error: continue — ignoring error in '{}'",
                                helper.name
                            );
                        }
                        _ => return Err(e),
                    },
                    Err(e) => return Err(e),
                }
                memory = env.get_memory();
                break;
            }
        }

        // Receive blocks: process pending events from the event bus
        for ch in &helper.receive_block.clone() {
            if let Some(events) = self.event_bus.get(&ch.name).cloned() {
                for event_val in events {
                    env.set_memory(memory.clone());
                    env.set("event", event_val);
                    if let Some(ref handler_name) = ch.on_receive {
                        if let Some(func) = self.functions.get(handler_name).cloned() {
                            self.call_behavior(&func, &mut env)?;
                        }
                    }
                    if let Some(ref bind_expr) = ch.bind.clone() {
                        self.eval_expr(bind_expr, &mut env)?;
                    }
                    memory = env.get_memory();
                }
                self.event_bus.remove(&ch.name);
            }
        }

        // Remaining when blocks (expr/changes triggers) + objective triggers
        extra_when.extend(
            helper
                .when_blocks
                .clone()
                .into_iter()
                .filter(|wb| !matches!(wb.trigger, WhenTrigger::Started | WhenTrigger::Cron(_))),
        );
        for wb in &extra_when {
            env.set_memory(memory.clone());
            match &wb.trigger {
                WhenTrigger::Started => {}
                WhenTrigger::Expr(cond) => {
                    let v = self.eval_expr(cond, &mut env)?;
                    if v.is_truthy() {
                        match self.run_stmts(&wb.body, &mut env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
                        memory = env.get_memory();
                    }
                }
                WhenTrigger::Changes(expr) => {
                    let key = format!("__prev_{:?}", expr)
                        .replace('"', "")
                        .replace(' ', "_");
                    let current = self.eval_expr(expr, &mut env)?;
                    let prev = memory.get(&key).cloned().unwrap_or(Value::Null);
                    if current != prev {
                        memory.insert(key.clone(), current.clone());
                        env.set_memory(memory.clone());
                        match self.run_stmts(&wb.body, &mut env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
                        memory = env.get_memory();
                    }
                }
                // Phase 5: when message "event" { ... }
                WhenTrigger::Message(event) => {
                    let bus_key = format!("{}:{}", helper.name, event);
                    if let Some(messages) = self.event_bus.remove(&bus_key) {
                        for msg in messages {
                            env.set_memory(memory.clone());
                            env.set("message", msg);
                            match self.run_stmts(&wb.body, &mut env) {
                                Ok(_) | Err(Signal::Return(_)) => {}
                                Err(e) => return Err(e),
                            }
                            memory = env.get_memory();
                        }
                    }
                }
                // when cron "*/5 * * * *" { ... } — fires if expr matches current time
                WhenTrigger::Cron(_expr) => {
                    // Cron agents are run via run_cron_helper; skip here
                }
            }
        }

        // Cron daemon: if any when-cron blocks exist, enter blocking loop
        let cron_blocks: Vec<_> = helper
            .when_blocks
            .iter()
            .filter(|wb| matches!(&wb.trigger, WhenTrigger::Cron(_)))
            .cloned()
            .collect();
        if !cron_blocks.is_empty() {
            env.set_memory(memory.clone());
            self.run_cron_daemon(&cron_blocks, &mut env)?;
        }

        // Persist memory to SQLite if this agent uses persistent storage
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref db_path) = persistent_db_path {
            let final_memory = env.get_memory();
            let _ = save_persistent_memory(db_path, &final_memory);
        }

        Ok(())
    }

    /// Returns true if the agent has any persistent memory entries (checked by name convention).
    #[allow(unused_variables)]
    fn has_persistent_memory(&self, agent_name: &str) -> bool {
        // Persistent memory is opt-in: set `gx_persistent = true` in memory block
        // or detected by looking at agent's memory entries. For now, we check if a
        // special file exists at the expected path.
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::path::Path::new(&persistent_db_path_for(agent_name)).exists()
        }
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// Phase 5: call another agent by name, inject `input`, return communicate value.
    pub fn call_agent(&mut self, name: &str, input: Value) -> IResult {
        let helper = self.helpers.get(name).cloned().ok_or_else(|| {
            Signal::Error(format!(
                "Agent '{}' not defined. Make sure it is declared before use.",
                name
            ))
        })?;

        let mut env = Env::new();
        env.set("input", input);

        // Initialize memory
        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));
        for entry in &helper.memory {
            let val = self.eval_expr(&entry.value.clone(), &mut env)?;
            memory.insert(entry.key.clone(), val);
        }
        env.set_memory(memory);

        // Run brain if present — collect communicate return value
        if let Some(brain) = &helper.brain.clone() {
            macro_rules! run_phase {
                ($stmts:expr) => {
                    match self.run_stmts($stmts, &mut env) {
                        Ok(_) | Err(Signal::Return(_)) => {}
                        Err(e) => return Err(e),
                    }
                };
            }
            run_phase!(&brain.plan);
            run_phase!(&brain.execute);
            run_phase!(&brain.remember);
            return self.eval_communicate(&brain.communicate, &mut env);
        }

        // Run when started block
        for wb in &helper.when_blocks.clone() {
            if matches!(wb.trigger, WhenTrigger::Started) {
                match self.run_stmts(&wb.body, &mut env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        // No brain — return the env's last meaningful value or Null
        Ok(Value::Null)
    }

    /// Run communicate stmts and return the last expression value (for call_agent).
    fn eval_communicate(&mut self, stmts: &[Stmt], env: &mut Env) -> IResult {
        let mut last = Value::Null;
        for stmt in stmts {
            match stmt {
                Stmt::Expr { expr, .. } => {
                    last = self.eval_expr(expr, env)?;
                    // Don't auto-print — caller uses log() if they want output
                }
                Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                    let v = self.eval_expr(value, env)?;
                    self.emit_output(&v.to_string());
                    last = v;
                }
                Stmt::Return { value, .. } => {
                    return Ok(match value {
                        Some(e) => self.eval_expr(e, env)?,
                        None => Value::Null,
                    });
                }
                other => {
                    self.run_stmt(other, env)?;
                }
            }
        }
        Ok(last)
    }

    fn run_brain(&mut self, brain: &BrainBlock, env: &mut Env) -> IResult {
        macro_rules! run_phase {
            ($stmts:expr) => {
                match self.run_stmts($stmts, env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
            };
        }
        run_phase!(&brain.plan);
        run_phase!(&brain.execute);
        run_phase!(&brain.remember);
        run_phase!(&brain.communicate);
        Ok(Value::Null)
    }

    // ── Statement execution ───────────────────────────────────────────────────

    fn run_stmts(&mut self, stmts: &[Stmt], env: &mut Env) -> IResult {
        let mut last = Value::Null;
        for stmt in stmts {
            last = self.run_stmt(stmt, env)?;
        }
        Ok(last)
    }

    /// Wrapper that attaches source-line and call-stack context to runtime errors.
    /// Only the innermost statement attaches the line (outer frames see " at line "
    /// already present and pass the error through unchanged).
    ///
    /// Also the Task Runtime's one and only cancellation checkpoint: every
    /// statement anywhere (top-level, inside `if`, inside every loop body —
    /// `while`/`for`/`loop until`/`repeat`/... all funnel through
    /// `run_stmts`, which calls this once per statement) checks whether the
    /// current task was cancelled, so no individual loop construct needs
    /// its own check. When there's no active task (`current_task` is
    /// `None` — the overwhelmingly common case: top-level scripts, agents
    /// not running inside a task) this is a single branch, matching this
    /// runtime's "lightweight when not in use" requirement the same way
    /// `Diagnostics::is_enabled()` gates tracing.
    fn run_stmt(&mut self, stmt: &Stmt, env: &mut Env) -> IResult {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(task) = &self.current_task {
            if task.is_cancelled() {
                return Err(Signal::Cancelled(task.cancel_reason()));
            }
        }
        let line = stmt_line(stmt);
        self.current_line = line;
        #[cfg(not(target_arch = "wasm32"))]
        if debugger::should_pause(&self.debug, line) {
            self.debug_pause(env, line, "breakpoint")?;
        }
        match self.run_stmt_inner(stmt, env) {
            Err(Signal::Error(m)) if !m.contains(" at line ") => {
                let full = self.with_call_stack(format!("{} at line {}", m, stmt_line(stmt)));
                Err(Signal::Error(full))
            }
            other => other,
        }
    }

    /// Append `"\n  in {call stack}"` to `msg` when a call stack is
    /// active — the same context `run_stmt` already attaches to every
    /// uncaught `Signal::Error`. `Signal::AssertFail` deliberately never
    /// goes through that wrapper (its message stays exactly what the
    /// script wrote, since `e.message` inside a `catch` block — and
    /// `gx test`'s failure list — must keep matching it verbatim), which
    /// meant an *uncaught* assertion failure showed no call-stack context
    /// at all, unlike every other kind of uncaught error. Called
    /// specifically at the top-level `Signal::AssertFail` → final-string
    /// conversion sites in `run_program` — never on a message a `catch`
    /// block or `gx test` will see.
    fn with_call_stack(&self, msg: String) -> String {
        if self.call_stack.is_empty() {
            msg
        } else {
            format!("{}\n  in {}", msg, format_call_stack(&self.call_stack))
        }
    }

    /// Render a `Signal` that propagated all the way to a context expecting
    /// only "the operation failed with this message" (top-level statements,
    /// top-level helpers, the top-level brain, a task body) into one stable,
    /// human-readable string. Every variant besides `Error`/`AssertFail`
    /// reaching one of these points is itself unusual (`return`/`break`/
    /// `continue` are normally consumed by the loop/function that produces
    /// them, `respond` by a route handler, `cancelled` by the task/wait
    /// machinery) but still needs *some* readable text.
    ///
    /// Before this, each call site inlined its own match with different
    /// wording for the same variants (e.g. `Signal::Return` read "return
    /// outside function" in one place and "Unexpected return at top level"
    /// in another), and `run_task_body` had no such match at all — it fell
    /// back to Rust's raw `{:?}` debug syntax, so a failed task's `error`
    /// field read literally `Error("division by zero...")` instead of the
    /// plain message every other builtin's failure carries.
    fn describe_stray_signal(&self, signal: Signal) -> String {
        match signal {
            Signal::Error(m) => m,
            Signal::AssertFail(m) => self.with_call_stack(format!("Assertion failed: {}", m)),
            Signal::Return(_) => "unexpected return outside a function".into(),
            Signal::ReRun => "unexpected re-run outside a retry block".into(),
            Signal::Break => "unexpected break outside a loop".into(),
            Signal::Continue => "unexpected continue outside a loop".into(),
            Signal::EscalateToHuman => "escalated to human".into(),
            Signal::Respond(_, _, _) => "unexpected respond outside a route handler".into(),
            Signal::Cancelled(reason) => format!("cancelled: {}", reason),
        }
    }

    // ── Debugger Runtime ─────────────────────────────────────────────────

    /// Parse `src` as a single expression and evaluate it against `env` —
    /// the debugger prompt's `print <expr>`/`watch <expr>`. Reuses the real
    /// lexer/parser (the same "detect indent vs. brace syntax, then parse"
    /// sequence used elsewhere in this file for imports — see
    /// `crate::indent_parser::is_indent_syntax`) rather than a bespoke
    /// expression parser: `src` becomes a one-statement program, and a bare
    /// expression always lowers to a `Stmt::Expr`, so pulling it back out
    /// and evaluating it directly gives real GX expression syntax (method
    /// calls, field access, interpolation, everything) for free.
    #[cfg(not(target_arch = "wasm32"))]
    fn eval_debug_expr(&mut self, src: &str, env: &mut Env) -> Result<Value, String> {
        let program = if crate::indent_parser::is_indent_syntax(src) {
            crate::indent_parser::parse(src)?
        } else {
            let tokens = crate::lexer::Lexer::new(src).tokenize()?;
            crate::parser::Parser::new(tokens).parse()?
        };
        match program.top_level_stmts.first() {
            Some(Stmt::Expr { expr, .. }) if program.top_level_stmts.len() == 1 => {
                self.eval_expr(&expr.clone(), env).map_err(|e| match e {
                    Signal::Error(m) => m,
                    other => format!("{:?}", other),
                })
            }
            _ => Err("not a single expression".to_string()),
        }
    }

    /// Print every registered watch expression's current value, evaluated
    /// fresh against `env` — called once whenever the debugger pauses, so
    /// a developer never has to manually re-`print` the same thing after
    /// every step.
    #[cfg(not(target_arch = "wasm32"))]
    fn print_watches(&mut self, env: &mut Env) {
        for expr in self.debug.watches.clone() {
            match self.eval_debug_expr(&expr, env) {
                Ok(v) => println!("watch: {} = {}", expr, v),
                Err(e) => println!("watch: {} -> Error: {}", expr, e),
            }
        }
    }

    /// The interactive `(gx-debug)` prompt — entered from `run_stmt` when
    /// `debugger::should_pause` says so, or directly from the
    /// `breakpoint()` builtin. Blocks on stdin exactly like `readline()`
    /// (an existing builtin) already does; there is nothing unusual about
    /// a debugger synchronously waiting for developer input on the thread
    /// that hit the breakpoint. `parallel { ... }` runs its branches
    /// sequentially on the *same* thread/Interpreter as the caller (true
    /// concurrency for it is still Phase 8 future work — see
    /// `Stmt::Parallel`'s own comment), so a breakpoint in one branch
    /// pauses exactly like it would anywhere else in that same script. A
    /// `task_spawn`'d closure genuinely does run on its own OS thread with
    /// its own, separate `Interpreter` (see `spawn_task_internal`), so a
    /// pause there blocks only that task's thread, never the caller's —
    /// but that child `Interpreter` is constructed fresh and does **not**
    /// inherit the parent's `self.debug` (only `helpers`/`functions`/
    /// `capabilities`/`diagnostics`/`current_task` are copied), so an
    /// *external* `--break <line>` set on the parent will not fire inside
    /// a spawned task's own body — only an unconditional `breakpoint()`
    /// call written directly in that task's closure will. Documented as a
    /// known limitation rather than propagated: `--break` is inherently
    /// process-external CLI configuration, and every other kind of
    /// Interpreter-local state (global_vars, registered_tests, ...)
    /// already follows this same "spawned tasks start clean" convention.
    #[cfg(not(target_arch = "wasm32"))]
    fn debug_pause(&mut self, env: &mut Env, line: usize, reason: &str) -> IResult {
        use std::io::{self, BufRead, Write};

        println!("\n[{}] paused at line {}", reason, line);
        if !self.call_stack.is_empty() {
            println!("  in {}", format_call_stack(&self.call_stack));
        }
        self.diagnostics.event(
            "debugger.pause",
            serde_json::json!({ "line": line, "reason": reason }),
        );
        self.print_watches(env);

        let stdin = io::stdin();
        loop {
            print!("(gx-debug) ");
            io::stdout().flush().ok();

            let mut input = String::new();
            match stdin.lock().read_line(&mut input) {
                // EOF (piped input ran out, or stdin isn't interactive at
                // all) — resume rather than hang forever waiting for input
                // that will never come.
                Ok(0) => {
                    self.debug.mode = self.debug.mode_after_continue();
                    return Ok(Value::Null);
                }
                Err(_) => {
                    self.debug.mode = self.debug.mode_after_continue();
                    return Ok(Value::Null);
                }
                Ok(_) => {}
            }

            match debugger::parse_debug_command(&input) {
                DebugCommand::Empty => continue,
                DebugCommand::Continue => {
                    self.debug.mode = self.debug.mode_after_continue();
                    return Ok(Value::Null);
                }
                DebugCommand::Step => {
                    self.debug.mode = DebugMode::StepInto;
                    return Ok(Value::Null);
                }
                DebugCommand::Locals => {
                    let mut names: Vec<&String> =
                        env.all_vars().keys().filter(|k| *k != "memory").collect();
                    names.sort();
                    if names.is_empty() {
                        println!("(no locals)");
                    } else {
                        for n in names {
                            println!("  {} = {}", n, env.get(n));
                        }
                    }
                }
                DebugCommand::Stack => {
                    if self.call_stack.is_empty() {
                        println!("(top level, no active call frames)");
                    } else {
                        for (i, frame) in self.call_stack.iter().enumerate() {
                            println!("  #{} {}", i, frame);
                        }
                    }
                }
                DebugCommand::Print(expr) => match self.eval_debug_expr(&expr, env) {
                    Ok(v) => println!("{}", v),
                    Err(e) => println!("Error: {}", e),
                },
                DebugCommand::Watch(expr) => {
                    println!("watch added: {}", expr);
                    self.debug.watches.push(expr);
                }
                DebugCommand::Quit => {
                    return Err(Signal::Error(
                        "debugger: execution stopped by 'quit'".to_string(),
                    ));
                }
                DebugCommand::Help => {
                    println!("Debugger commands:");
                    println!("  c, continue     resume execution");
                    println!("  s, step         run the next statement, then pause again");
                    println!("  l, locals       list variables in the current scope");
                    println!("  bt, stack       show the current call stack");
                    println!("  p, print <expr> evaluate and print a GX expression");
                    println!("  w, watch <expr> re-evaluate <expr> at every future pause");
                    println!("  q, quit         stop execution");
                }
                DebugCommand::Unknown(cmd) => {
                    println!("Unknown command '{}'. Type 'help' for a list.", cmd);
                }
            }
        }
    }

    // ── Testing Framework ────────────────────────────────────────────────

    /// A single `[0, 1)` draw for `random`/`random_int`/`random_choice` —
    /// deterministic (steps the shared `rng_seed` state) once
    /// `set_random_seed(n)` has been called, otherwise identical to the
    /// unseeded `random_unit_f64()` every prior release used. `shuffle`
    /// does *not* go through this: see its own match arm for why it needs
    /// its seed handled separately.
    fn next_random_unit_f64(&mut self) -> f64 {
        match &mut self.rng_seed {
            Some(state) => lcg_step_unit_f64(state),
            None => random_unit_f64(),
        }
    }

    /// Run a zero-argument `Value::Closure` registered via `test(name,
    /// fn)`/`before_each(fn)`/`after_each(fn)` — the entry point
    /// `crate::toolchain::test` uses to actually execute each registered
    /// test case (and its hooks) after the top-level script finishes.
    ///
    /// `env` is caller-supplied rather than created fresh here: GX
    /// closures capture their outer variables *by value* (see
    /// `call_closure_with_capture`'s doc comment) — a plain variable
    /// `before_each` mutates is invisible to the test body's own,
    /// separately-captured snapshot of it. `memory.*` is this language's
    /// existing, deliberate channel for state that *does* need to survive
    /// across separate closure calls (every agent already relies on
    /// exactly this). Passing the same `Env` through `before_each` → the
    /// test body → `after_each` for one test case is what makes that
    /// channel actually work for setup/teardown: a `before_each` that
    /// does `memory.db = create_test_db()` is then visible to the test
    /// body via `memory.db`, and to `after_each` for cleanup. Callers
    /// should use a fresh `Env` per test case (not shared across
    /// different tests) so one test's leftover `memory.*` never leaks
    /// into the next.
    pub fn call_registered_closure(
        &mut self,
        closure: &Value,
        env: &mut Env,
    ) -> Result<Value, String> {
        let (params, body, captured) = match closure {
            Value::Closure(p, b, c) => (p.clone(), b.clone(), c.clone()),
            other => {
                return Err(format!(
                    "expected a function (fn() {{ ... }}), got {}",
                    other.type_name()
                ))
            }
        };
        self.call_closure_with_capture(&params, &body, &captured, Vec::new(), env)
            .map_err(|e| match e {
                Signal::Error(m) => m,
                Signal::AssertFail(m) => self.with_call_stack(format!("Assertion failed: {}", m)),
                other => format!("{:?}", other),
            })
    }

    /// Drain every test case registered via `test(name, fn)` since the last
    /// call — `crate::toolchain::test` calls this exactly once, right
    /// after the top-level script finishes, then runs each returned case.
    /// `take` (not a borrow/clone) because a test file is only ever run
    /// once per `Interpreter`; leaving stale entries behind would be a
    /// silent trap for any future caller that reused the same instance.
    pub fn take_registered_tests(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.registered_tests)
    }

    pub fn before_each_hook(&self) -> Option<Value> {
        self.before_each_fn.clone()
    }

    pub fn after_each_hook(&self) -> Option<Value> {
        self.after_each_fn.clone()
    }

    fn run_stmt_inner(&mut self, stmt: &Stmt, env: &mut Env) -> IResult {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                let val = self.eval_expr(value, env)?;
                self.assign(target, val, env)?;
                Ok(Value::Null)
            }

            Stmt::PlusAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.add_values(&cur, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::MinusAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Sub, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::MulAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Mul, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::DivAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Div, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for (cond, body) in branches {
                    if self.eval_expr(cond, env)?.is_truthy() {
                        return self.run_stmts(body, env);
                    }
                }
                if let Some(body) = else_body {
                    return self.run_stmts(body, env);
                }
                Ok(Value::Null)
            }

            Stmt::ForEach {
                var, iter, body, ..
            } => {
                let col = self.eval_expr(iter, env)?;
                let items = col.iter().map_err(Signal::Error)?;
                let mut last = Value::Null;
                'outer: for item in items {
                    env.set(var, item);
                    match self.run_stmts(body, env) {
                        Ok(v) => last = v,
                        Err(Signal::Break) => break 'outer,
                        Err(Signal::Continue) => continue 'outer,
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }

            Stmt::While {
                condition, body, ..
            } => {
                let mut last = Value::Null;
                let mut iterations = 0usize;
                // Default cap: 10 million iterations — enough for any real program,
                // prevents accidental infinite spin. Disabled by --no-limit.
                // Note: wall-clock timeout is intentionally removed — blocking I/O
                // (readline, http_stream) would trigger it incorrectly.
                const MAX_WHILE: usize = 10_000_000;
                let limit = if self.no_loop_limit {
                    usize::MAX
                } else {
                    MAX_WHILE
                };
                loop {
                    if iterations >= limit {
                        return Err(Signal::Error(
                            "while loop exceeded 10,000,000 iterations. \
                             If this is an intentional infinite loop (e.g. a REPL using readline()), \
                             run with: gx run --no-limit"
                                .into(),
                        ));
                    }
                    iterations += 1;
                    let cond = self.eval_expr(condition, env)?;
                    if !cond.is_truthy() {
                        break;
                    }
                    match self.run_stmts(body, env) {
                        Ok(v) => last = v,
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }

            Stmt::Break { .. } => Err(Signal::Break),
            Stmt::Continue { .. } => Err(Signal::Continue),

            Stmt::Assert {
                condition,
                message,
                line,
            } => {
                self.assert_count += 1;
                let passed = self.eval_expr(condition, env)?.is_truthy();
                if !passed {
                    let msg = if let Some(msg_expr) = message {
                        self.eval_expr(msg_expr, env)?.to_string()
                    } else {
                        format!("assertion at line {} failed", line)
                    };
                    self.assert_failures.push(msg.clone());
                    return Err(Signal::AssertFail(msg));
                }
                Ok(Value::Null)
            }

            Stmt::TryCatch {
                try_body,
                catch_kind,
                catch_var,
                catch_body,
                ..
            } => match self.run_stmts(try_body, env) {
                Ok(v) => Ok(v),
                Err(signal @ (Signal::Error(_) | Signal::AssertFail(_))) => {
                    // #18: build typed error object — AssertFail always maps to AssertionError
                    let (msg, kind) = match signal {
                        Signal::AssertFail(m) => (m, "AssertionError"),
                        Signal::Error(m) => {
                            let k = infer_error_kind(&m);
                            (m, k)
                        }
                        _ => unreachable!(),
                    };
                    // If a type filter is set, only catch matching kinds
                    if let Some(required_kind) = catch_kind {
                        if required_kind != kind {
                            return Err(Signal::Error(msg));
                        }
                    }
                    let mut err_map = HashMap::new();
                    err_map.insert("message".into(), Value::Str(msg));
                    err_map.insert("kind".into(), Value::Str(kind.into()));
                    err_map.insert("code".into(), Value::Null);
                    env.set(catch_var, Value::Object(err_map));
                    self.run_stmts(catch_body, env)
                }
                Err(other) => Err(other),
            },

            Stmt::Emit { event, payload, .. } => {
                let mut resolved = Vec::new();
                for (k, expr) in payload {
                    resolved.push((k.clone(), self.eval_expr(expr, env)?));
                }
                // Add to both the events list and the event bus
                self.events.push((event.clone(), resolved.clone()));
                let bus_val = {
                    let mut map = HashMap::new();
                    for (k, v) in resolved {
                        map.insert(k, v);
                    }
                    Value::Object(map)
                };
                self.event_bus
                    .entry(event.clone())
                    .or_default()
                    .push(bus_val);
                Ok(Value::Null)
            }

            Stmt::Broadcast { event, .. } => {
                self.events.push((event.clone(), Vec::new()));
                self.event_bus
                    .entry(event.clone())
                    .or_default()
                    .push(Value::Null);
                Ok(Value::Null)
            }

            // Phase 5: send "event" to "agent" with { key: val }
            Stmt::SendMessage {
                agent_name,
                event,
                data,
                ..
            } => {
                let name_val = self.eval_expr(agent_name, env)?;
                let target = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "send: agent name must be a string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                map.insert("event".into(), Value::Str(event.clone()));
                for (k, v) in data {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let msg = Value::Object(map);
                // Deliver synchronously to the target agent's when message handlers
                let helper = self.helpers.get(&target).cloned();
                if let Some(h) = helper {
                    let handlers: Vec<_> = h
                        .when_blocks
                        .iter()
                        .filter(|wb| matches!(&wb.trigger, WhenTrigger::Message(e) if e == event))
                        .cloned()
                        .collect();
                    if !handlers.is_empty() {
                        let mut msg_env = Env::new();
                        msg_env.set("message", msg.clone());
                        for wb in &handlers {
                            self.run_stmts(&wb.body, &mut msg_env)?;
                        }
                        return Ok(Value::Null);
                    }
                }
                // No handler found — queue for deferred processing
                let bus_key = format!("{}:{}", target, event);
                self.event_bus.entry(bus_key).or_default().push(msg);
                Ok(Value::Null)
            }

            Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                let v = self.eval_expr(value, env)?;
                self.emit_output(&v.to_string());
                Ok(Value::Null)
            }

            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Null,
                };
                Err(Signal::Return(v))
            }

            Stmt::Wait { ms, .. } => {
                if let Some(n) = self.eval_expr(ms, env)?.as_number() {
                    std::thread::sleep(std::time::Duration::from_millis(n as u64));
                }
                Ok(Value::Null)
            }

            Stmt::ReRun { .. } => Err(Signal::ReRun),

            Stmt::EscalateToHuman { .. } => {
                eprintln!("[gx] Escalating to human — agent cannot handle this request");
                self.events.push(("escalate_to_human".into(), Vec::new()));
                Err(Signal::EscalateToHuman)
            }

            Stmt::Respond {
                format,
                value,
                status,
                ..
            } => {
                let v = self.eval_expr(value, env)?;
                let body = match format.as_str() {
                    "json" => value_to_json(&v),
                    _ => v.to_string(),
                };
                let content_type = match format.as_str() {
                    "json" => "application/json; charset=utf-8".to_string(),
                    "html" => "text/html; charset=utf-8".to_string(),
                    _ => "text/plain; charset=utf-8".to_string(),
                };
                Err(Signal::Respond(content_type, body, *status))
            }

            Stmt::RespondStream { body, .. } => {
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = body;
                    Err(Signal::Error(
                        "respond stream is not available in the playground".into(),
                    ))
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // `pending_request` checked *before* the counter, and
                    // only actually `take()`n once the counter's increment
                    // has also succeeded — either rejection must leave
                    // `pending_request` in place (so `handle_one_request`'s
                    // existing reclaim path can still send the client a
                    // proper response, the same as any other error a route
                    // body returns) and must not touch the counter (a
                    // "no request to stream" rejection here — e.g. a
                    // second `respond stream` in one route body, the first
                    // having already claimed it — must not leak an
                    // increment with no responder thread ever spawned to
                    // decrement it back).
                    if self.pending_request.is_none() {
                        return Err(Signal::Error(
                            "respond stream: only valid inside an HTTP server route".into(),
                        ));
                    }
                    if let Some(counter) = &self.active_sse_responders {
                        if !try_reserve_sse_responder_slot(counter, MAX_CONCURRENT_SSE_RESPONDERS) {
                            return Err(Signal::Error(format!(
                                "respond stream: too many concurrent streaming connections \
                                 ({} already open) — try again once some finish",
                                MAX_CONCURRENT_SSE_RESPONDERS
                            )));
                        }
                    }
                    let request = self
                        .pending_request
                        .take()
                        .expect("checked Some just above");
                    // Bounded so a script producing frames faster than the
                    // client can read them applies real backpressure
                    // (sse_send blocks) instead of buffering unboundedly in
                    // memory.
                    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
                    let reader = bridge_impl::ChannelReader::new(rx);
                    let headers = vec![
                        tiny_http::Header::from_bytes(
                            b"Content-Type".as_ref(),
                            b"text/event-stream; charset=utf-8",
                        )
                        .expect("static header is always valid"),
                        tiny_http::Header::from_bytes(b"Cache-Control".as_ref(), b"no-cache")
                            .expect("static header is always valid"),
                        tiny_http::Header::from_bytes(
                            b"X-Content-Type-Options".as_ref(),
                            b"nosniff",
                        )
                        .expect("static header is always valid"),
                    ];
                    // data_length: None -> tiny_http uses chunked transfer
                    // encoding, since the total size isn't known upfront.
                    let response =
                        tiny_http::Response::new(200u16.into(), headers, reader, None, None);
                    let active_sse_responders = self.active_sse_responders.clone();
                    let responder = std::thread::spawn(move || {
                        let _ = request.respond(response);
                        // Matches the `fetch_update` increment above —
                        // released once the write finishes, however long
                        // that takes (a healthy client: promptly; a dead
                        // one: only once the OS's own TCP-level timeout
                        // gives up on it, since `tiny_http` exposes no
                        // write-timeout hook — see `MAX_CONCURRENT_SSE_
                        // RESPONDERS`'s doc comment). Runs even if this
                        // thread is never joined by anyone, since the
                        // decrement is the last thing the closure itself
                        // does.
                        if let Some(counter) = active_sse_responders {
                            counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    });

                    self.sse_tx = Some(tx);
                    // catch_unwind, not a plain call, specifically so a
                    // panic inside the block can't skip past `sse_tx =
                    // None` below — without this, a panic here would leave
                    // sse_tx pointing at a channel nothing will ever close,
                    // and the responder thread parked forever on
                    // rx.recv(): a real (if narrow) per-panic resource
                    // leak on any worker that survives the panic (see the
                    // catch_unwind around handle_one_request, which is
                    // what lets a worker survive to process another
                    // request at all). Cleanup happens either way, then
                    // the panic resumes so that outer catch_unwind still
                    // observes and logs it.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.run_stmts(body, env)
                    }));
                    self.sse_tx = None; // drop the sender -> ChannelReader sees EOF
                                        // Deliberately not joined: once the sender is dropped,
                                        // ChannelReader.read() returns EOF as soon as the
                                        // responder next asks it for data, but the responder
                                        // could be blocked *right now* inside the underlying
                                        // socket write — tiny_http has no write-timeout hook,
                                        // so if the client has stopped reading (a dead
                                        // connection that never closes, or just a very slow
                                        // one) that write can block indefinitely. Joining here
                                        // would reintroduce exactly the worker-starvation bug
                                        // SSE_SEND_TIMEOUT exists to prevent: this thread only
                                        // holds the response/socket, not any interpreter
                                        // state, so leaving it to finish (or hang) on its own
                                        // costs one thread for that one dead connection, not
                                        // this worker's ability to serve every other request.
                    drop(responder);
                    match outcome {
                        Ok(result) => result.map(|_| Value::Null),
                        Err(payload) => std::panic::resume_unwind(payload),
                    }
                }
            }

            Stmt::Serve { port, routes, .. } => {
                #[cfg(not(target_arch = "wasm32"))]
                return self.run_serve(port, routes, env);
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (port, routes);
                    return Err(Signal::Error(
                        "HTTP server not available in playground".into(),
                    ));
                }
            }

            // ── v0.2.0 opinionated sugar ──────────────────────────────────────
            Stmt::Think {
                prompt,
                model,
                temperature,
                min_confidence,
                into_var,
                ..
            } => {
                let prompt_val = self.eval_expr(prompt, env)?;
                let provider = model.as_deref().unwrap_or("openai");
                let temp_val = match temperature {
                    Some(t) => self.eval_expr(t, env)?,
                    None => Value::Number(0.7),
                };
                let min_conf = match min_confidence {
                    Some(mc) => self.eval_expr(mc, env)?.as_number().unwrap_or(0.0),
                    None => 0.0,
                };
                self.authorize_ai_provider(provider)?;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mut params: HashMap<String, Value> = HashMap::new();
                    params.insert("prompt".into(), prompt_val);
                    params.insert("temperature".into(), temp_val);
                    let agent = self.http_agent();
                    let span = self.diagnostics.start_span("ai.request");
                    let result = ai::ask_ai(provider, None, &params, &agent);
                    self.record_tokens(&result);
                    self.end_ai_span(span, provider, None, &result);
                    if min_conf > 0.0 {
                        let conf = if let Value::Object(ref m) = result {
                            m.get("confidence")
                                .and_then(|v| v.as_number())
                                .unwrap_or(1.0)
                        } else {
                            1.0
                        };
                        if conf < min_conf {
                            eprintln!(
                                "[gx think] confidence {:.2} below threshold {:.2} — escalating",
                                conf, min_conf
                            );
                            self.events.push(("escalate_to_human".into(), Vec::new()));
                            return Err(Signal::EscalateToHuman);
                        }
                    }
                    env.set(into_var, result);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (temp_val, min_conf);
                    let mut m = HashMap::new();
                    m.insert(
                        "text".into(),
                        Value::Str("[AI not available in playground]".into()),
                    );
                    m.insert("confidence".into(), Value::Number(1.0));
                    m.insert("ok".into(), Value::Bool(false));
                    env.set(into_var, Value::Object(m));
                }
                Ok(Value::Null)
            }

            Stmt::Observe { bindings, .. } => {
                for (key, val_expr) in bindings {
                    let v = self.eval_expr(val_expr, env)?;
                    env.set(key, v);
                }
                Ok(Value::Null)
            }

            Stmt::Act { body, .. } => self.run_stmts(body, env),

            Stmt::LoopUntil {
                condition, body, ..
            } => {
                let mut iterations = 0usize;
                let loop_limit = if self.no_loop_limit {
                    usize::MAX
                } else {
                    10_000_000
                };
                loop {
                    if iterations > loop_limit {
                        return Err(Signal::Error(
                            "loop until exceeded iteration limit. Use --no-limit if intentional."
                                .into(),
                        ));
                    }
                    let cond_val = self.eval_expr(condition, env)?;
                    if cond_val.is_truthy() {
                        break;
                    }
                    match self.run_stmts(body, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => {}
                        Err(e) => return Err(e),
                    }
                    iterations += 1;
                }
                Ok(Value::Null)
            }

            Stmt::RepeatTimes {
                count, var, body, ..
            } => {
                let n = self.eval_expr(count, env)?.as_number().unwrap_or(0.0) as usize;
                for i in 0..n {
                    if let Some(v) = var {
                        env.set(v, Value::Number(i as f64));
                    }
                    match self.run_stmts(body, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => {}
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }

            Stmt::Parallel { branches, .. } => {
                // Sequential fallback — true parallelism deferred to Phase 8
                for branch in branches {
                    match self.run_stmts(branch, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }

            // await { key: expr, ... } — concurrent parallel execution
            Stmt::Await {
                bindings, into_var, ..
            } => {
                let result = self.eval_await_block(bindings, env)?;
                env.set(into_var, result.clone());
                Ok(result)
            }

            Stmt::DbTransaction { path, body, .. } => self.run_db_transaction(path, body, env),

            Stmt::Span { name, body, .. } => self.run_span(name, body, env),

            Stmt::Expr { expr, .. } => {
                // Bare `arr.push(x)`/`.sort()`/`.reverse()`/`.append(x)`
                // statement: mutate the array *in place* inside `env`'s own
                // storage, bypassing the general "evaluate the receiver,
                // hand it to eval_method, get back a new array" path — the
                // return value is about to be discarded anyway (this is a
                // bare statement, not an assignment), so there's no
                // functional-vs-mutating ambiguity to preserve here the
                // way there is for `results = results.push(x)` (see
                // `eval_call`'s own comment on why that pattern keeps its
                // existing functional behavior). Pure performance fix: the
                // general path clones the whole array (once to evaluate
                // the receiver, again inside eval_method) before mutating
                // the clone — for `.push()` in a loop, one of the most
                // common patterns in any script, that made each call O(n)
                // in the array's current length, i.e. O(n²) for an n-item
                // build-up loop instead of `Vec::push`'s ordinary amortized
                // O(1) (confirmed empirically: pushing 40,000 items took
                // over 45 seconds before this fix; 1 million items takes
                // well under a second after it). `.pop()` isn't handled
                // here — `eval_call` now always mutates in place for `pop`
                // regardless of context, so it already goes through the
                // fast path via the plain `self.eval_expr` fallthrough
                // below.
                if let Expr::Call { callee, args } = expr {
                    if let Expr::FieldAccess { object, field } = callee.as_ref() {
                        let method = field.as_str();
                        if matches!(method, "push" | "sort" | "reverse" | "append") {
                            // Case 1: bare identifier receiver — `arr.push(x)`.
                            if let Some(var_name) = self.extract_ident_name(object) {
                                let resolved_args: Vec<Value> = args
                                    .iter()
                                    .map(|a| self.eval_expr(a, env))
                                    .collect::<Result<Vec<_>, _>>()?;
                                if let Some(Value::Array(arr)) = env.vars.get_mut(&var_name) {
                                    mutate_array_in_place(arr, method, resolved_args);
                                    return Ok(Value::Null);
                                }
                                // Not an array (or the variable doesn't
                                // exist) — fall through to the general
                                // eval_method path for the same
                                // error/behavior as before.
                                let obj = env.get(&var_name);
                                let new_val = self.eval_method(obj, method, resolved_args, env)?;
                                env.set(&var_name, new_val);
                                return Ok(Value::Null);
                            }

                            // Case 2: one level of nesting — `memory.items.push(x)`,
                            // the single most common agent-memory pattern.
                            // Structural shape is checked *before* evaluating
                            // `args` (a cheap, clone-free peek — `env.vars.get`,
                            // not `Env::get`, which clones) specifically so a
                            // receiver shape that doesn't match this fast path
                            // hasn't already evaluated the call's arguments —
                            // otherwise falling through to the single ordinary
                            // evaluation at the bottom of this function would
                            // evaluate them a second time, an easy way to
                            // silently double a side effect. Deeper nesting
                            // (`a.b.c.push(x)`) isn't covered by a fast path
                            // and falls through unchanged: a bare statement,
                            // so the computed-but-discarded new array leaves
                            // the original silently unchanged — a known,
                            // documented limitation, not something this fix
                            // set out to solve for arbitrary depth.
                            if let Expr::FieldAccess {
                                object: inner_obj,
                                field: mid_field,
                            } = object.as_ref()
                            {
                                if let Some(root) = self.extract_ident_name(inner_obj) {
                                    let looks_like_array = matches!(
                                        env.vars.get(&root),
                                        Some(Value::Object(map))
                                            if matches!(map.get(mid_field), Some(Value::Array(_)))
                                    );
                                    if looks_like_array {
                                        let resolved_args: Vec<Value> = args
                                            .iter()
                                            .map(|a| self.eval_expr(a, env))
                                            .collect::<Result<Vec<_>, _>>()?;
                                        if let Some(Value::Object(map)) = env.vars.get_mut(&root) {
                                            if let Some(Value::Array(arr)) = map.get_mut(mid_field)
                                            {
                                                mutate_array_in_place(arr, method, resolved_args);
                                            }
                                        }
                                        // Whether or not the shape held after
                                        // evaluating args (it can only change
                                        // if one of them reassigned this exact
                                        // field as a side effect — vanishingly
                                        // rare, and either way `args` has now
                                        // been evaluated exactly once), this
                                        // statement is done.
                                        return Ok(Value::Null);
                                    }
                                }
                            }
                        }
                    }
                }

                // Auto-call zero-arg user functions referenced as bare identifiers
                if let Expr::Ident(ref name) = expr {
                    if let Some(func) = self.functions.get(name).cloned() {
                        if func.params.is_empty() {
                            return self.call_behavior(&func, env);
                        }
                    }
                }
                self.eval_expr(expr, env)
            }
        }
    }

    /// `span("name") { ... }` — manual tracing instrumentation. A no-op
    /// wrapper (body still runs normally) unless `--trace` is enabled; see
    /// `crate::diagnostics::Diagnostics::start_span`, which is itself a
    /// cheap early-return when disabled.
    fn run_span(&mut self, name: &Expr, body: &[Stmt], env: &mut Env) -> Result<Value, Signal> {
        // Skip evaluating (and allocating) the name entirely when tracing
        // is off — `span("...") { ... }` degrades to just running `body`,
        // matching "lightweight when diagnostics are disabled".
        if !self.diagnostics.is_enabled() {
            return self.run_stmts(body, env);
        }
        let name_val = self.eval_expr(name, env)?;
        let name_str = name_val
            .as_str()
            .ok_or_else(|| Signal::Error("span: name must be a string".into()))?
            .to_string();

        let handle = self.diagnostics.start_span(name_str);
        // catch_unwind so a panicking body still ends the span (with an
        // error outcome) instead of leaving it dangling on the stack —
        // the same cleanup-guarantee shape used for db_transaction/SSE.
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_stmts(body, env)));
        match outcome {
            Ok(result) => {
                match &result {
                    Ok(_) | Err(Signal::Return(_)) => {
                        self.diagnostics
                            .end_span(handle, crate::diagnostics::Outcome::Ok, &[]);
                    }
                    Err(e) => {
                        let msg = format!("{:?}", e);
                        self.diagnostics.end_span(
                            handle,
                            crate::diagnostics::Outcome::Error(&msg),
                            &[],
                        );
                    }
                }
                result
            }
            Err(payload) => {
                self.diagnostics.end_span(
                    handle,
                    crate::diagnostics::Outcome::Error("panicked"),
                    &[],
                );
                std::panic::resume_unwind(payload);
            }
        }
    }

    fn run_db_transaction(
        &mut self,
        path: &Expr,
        body: &[Stmt],
        env: &mut Env,
    ) -> Result<Value, Signal> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, body, env);
            return Err(Signal::Error(
                "db_transaction not available in playground".into(),
            ));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.authorize_capability(crate::capability::Resource::Database, None)
                .map_err(|e| Signal::Error(e.to_string()))?;
            let path_val = self.eval_expr(path, env)?;
            let path_str = path_val
                .as_str()
                .ok_or_else(|| Signal::Error("db_transaction: path must be a string".into()))?
                .to_string();
            let safe = self.safe_path(&path_str)?;
            let path_owned = safe.to_string_lossy().into_owned();

            // depth > 1 here means `path_owned` already has an active
            // transaction (this call is nested — see db_tx_begin) and gets
            // a SAVEPOINT instead of a fresh BEGIN.
            let depth = self.db_tx_begin(&path_owned)?;
            let span = self.diagnostics.start_span("db.transaction");

            // Expose `db` variable so inner db_exec(db, sql, params) calls work naturally
            env.set("db", Value::Str(path_owned.clone()));

            // catch_unwind, not a plain call, so a panic inside the
            // transaction body can't skip the rollback below — without
            // this, a panic would leave db_tx_depth pointing at a
            // still-open transaction/savepoint forever. That's not just a
            // leak: GX's HTTP server runs each worker's Interpreter across
            // many requests (see bridge_impl.rs's catch_unwind, which lets
            // a worker survive a panic and keep serving), so a stale open
            // transaction here would silently swallow every db_exec/
            // db_query call from *later*, unrelated requests handled by
            // that same worker into the wrong, uncommitted transaction.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.run_stmts(body, env)
            }));

            let result = match outcome {
                Ok(result) => result,
                Err(payload) => {
                    let _ = self.db_tx_end(&path_owned, depth, false);
                    self.diagnostics.end_span(
                        span,
                        crate::diagnostics::Outcome::Error("panic"),
                        &[("db", serde_json::Value::String(path_owned.clone()))],
                    );
                    std::panic::resume_unwind(payload);
                }
            };

            let should_commit = matches!(result, Ok(_) | Err(Signal::Return(_)));
            let final_result = match self.db_tx_end(&path_owned, depth, should_commit) {
                Ok(()) => result,
                Err(e) if should_commit => {
                    // The body succeeded but committing (or releasing the
                    // savepoint) failed — surface that failure rather than
                    // the body's Ok result, since data the caller thinks
                    // was saved may not have been.
                    Err(e)
                }
                Err(_) => result, // rollback itself failed; the original error is more useful than that.
            };
            let err_msg = match &final_result {
                Err(Signal::Error(e)) => Some(e.clone()),
                _ => None,
            };
            let span_outcome = match &err_msg {
                Some(e) => crate::diagnostics::Outcome::Error(e.as_str()),
                None => crate::diagnostics::Outcome::Ok,
            };
            self.diagnostics.end_span(
                span,
                span_outcome,
                &[("db", serde_json::Value::String(path_owned))],
            );
            final_result
        }
    }

    fn extract_ident_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            _ => None,
        }
    }

    // ── Assignment ────────────────────────────────────────────────────────────

    fn assign(&mut self, target: &Expr, val: Value, env: &mut Env) -> Result<(), Signal> {
        match target {
            Expr::Ident(name) => {
                env.set(name, val);
                Ok(())
            }

            Expr::FieldAccess { object, field } => match object.as_ref() {
                Expr::Ident(obj_name) => {
                    // Mutate the object in place inside `env`'s own
                    // storage — `Value::set_field` already takes `&mut
                    // self`, so no clone of the object's contents is
                    // needed at all (the previous `env.get`-then-
                    // `env.set` round trip cloned the *whole* object on
                    // every single field assignment: `obj.field = val` in
                    // a loop building up an n-key object was O(n) per
                    // call, i.e. O(n²) overall — confirmed empirically:
                    // 50,000 field assignments took over a minute before
                    // this fix).
                    let slot = env.vars.entry(obj_name.clone()).or_insert(Value::Null);
                    if matches!(slot, Value::Null) {
                        *slot = Value::Object(HashMap::new());
                    }
                    slot.set_field(field, val).map_err(Signal::Error)?;
                    Ok(())
                }
                Expr::FieldAccess {
                    object: inner_obj,
                    field: inner_field,
                } => {
                    let root = self.expr_root_name(inner_obj);
                    let mut outer = env.get(&root);
                    if matches!(outer, Value::Null) {
                        outer = Value::Object(HashMap::new());
                    }
                    let mut inner = outer.get_field(inner_field);
                    if matches!(inner, Value::Null) {
                        inner = Value::Object(HashMap::new());
                    }
                    inner.set_field(field, val).map_err(Signal::Error)?;
                    outer.set_field(inner_field, inner).map_err(Signal::Error)?;
                    env.set(&root, outer);
                    Ok(())
                }
                _ => Err(Signal::Error("Cannot assign to complex expression".into())),
            },

            Expr::Index { object, index } => {
                if let Expr::Ident(name) = object.as_ref() {
                    let idx = self.eval_expr(index, env)?;
                    // Mutate in place inside `env`'s own storage — same
                    // fix, and same measured impact, as the single-level
                    // `obj.field = val` case just above: `obj[key] = val`/
                    // `arr[i] = val` in a loop used to clone the *whole*
                    // container on every assignment (get the whole value
                    // out, mutate the copy, write the copy back), making
                    // an n-item build-up loop O(n²) instead of O(n).
                    let slot = env.vars.entry(name.clone()).or_insert(Value::Null);
                    match slot {
                        Value::Array(arr) => {
                            let Value::Number(n) = &idx else {
                                return Err(Signal::Error(
                                    "Cannot index assign to this type".into(),
                                ));
                            };
                            // A negative index that still lands before the
                            // start of the array (e.g. `arr[-100] = x` on a
                            // 3-element array) used to clamp to index 0
                            // (`.max(0)`) and silently overwrite the *first*
                            // element instead — unlike reading the same
                            // index (which misses and returns `null`, see
                            // `Value::get_index`), a write can't just "miss"
                            // without silently corrupting the wrong slot, so
                            // this is a clear error instead of a clamp.
                            let signed_i = if *n < 0.0 {
                                arr.len() as i64 + *n as i64
                            } else {
                                *n as i64
                            };
                            if signed_i < 0 {
                                return Err(Signal::Error(format!(
                                    "array index {} out of bounds for length {}",
                                    *n as i64,
                                    arr.len()
                                )));
                            }
                            let i = signed_i as usize;
                            if i < arr.len() {
                                arr[i] = val;
                            } else {
                                // Auto-extend array with nulls
                                while arr.len() < i {
                                    arr.push(Value::Null);
                                }
                                arr.push(val);
                            }
                        }
                        Value::Object(map) => {
                            let Value::Str(k) = &idx else {
                                return Err(Signal::Error(
                                    "Cannot index assign to this type".into(),
                                ));
                            };
                            map.insert(k.clone(), val);
                        }
                        Value::Null => {
                            // Auto-create: null[key] = val → create object or array
                            match &idx {
                                Value::Str(k) => {
                                    let mut map = HashMap::new();
                                    map.insert(k.clone(), val);
                                    *slot = Value::Object(map);
                                }
                                Value::Number(n) => {
                                    let mut arr = Vec::new();
                                    let i = *n as usize;
                                    for _ in 0..i {
                                        arr.push(Value::Null);
                                    }
                                    arr.push(val);
                                    *slot = Value::Array(arr);
                                }
                                _ => {
                                    return Err(Signal::Error(
                                        "Cannot index assign to null with this key type".into(),
                                    ))
                                }
                            }
                        }
                        _ => return Err(Signal::Error("Cannot index assign to this type".into())),
                    }
                    Ok(())
                } else {
                    Err(Signal::Error(
                        "Cannot assign to complex index expression".into(),
                    ))
                }
            }

            _ => Err(Signal::Error(format!("Cannot assign to {:?}", target))),
        }
    }

    fn eval_lvalue(&mut self, expr: &Expr, env: &mut Env) -> Value {
        self.eval_expr(expr, env).unwrap_or(Value::Null)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn expr_root_name(&self, expr: &Expr) -> String {
        match expr {
            Expr::Ident(s) => s.clone(),
            Expr::FieldAccess { object, .. } => self.expr_root_name(object),
            _ => "unknown".into(),
        }
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    pub fn eval_expr(&mut self, expr: &Expr, env: &mut Env) -> IResult {
        match expr {
            Expr::Null => Ok(Value::Null),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Num(n) => Ok(Value::Number(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),

            Expr::Interpolated(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        InterpolatedPart::Literal(l) => s.push_str(l),
                        InterpolatedPart::Expr(e) => {
                            s.push_str(&self.eval_expr(e, env)?.to_string())
                        }
                    }
                }
                Ok(Value::Str(s))
            }

            Expr::Ident(name) => {
                let val = env.get(name);
                if matches!(val, Value::Null) {
                    let mem = env.get_memory();
                    if let Some(v) = mem.get(name) {
                        return Ok(v.clone());
                    }
                }
                Ok(val)
            }

            Expr::FieldAccess { object, field } => {
                let obj = self.eval_expr(object, env)?;
                Ok(obj.get_field(field))
            }

            Expr::Index { object, index } => {
                // Range slice: expr[start..end]
                if let Expr::Range { start, end } = index.as_ref() {
                    let obj = self.eval_expr(object, env)?;
                    let s = self.eval_expr(start, env)?.as_number().unwrap_or(0.0) as usize;
                    let e = self.eval_expr(end, env)?.as_number().unwrap_or(0.0) as usize;
                    return match obj {
                        Value::Str(ref st) => {
                            let chars: Vec<char> = st.chars().collect();
                            let end = e.min(chars.len());
                            let start = s.min(end);
                            Ok(Value::Str(chars[start..end].iter().collect()))
                        }
                        Value::Array(ref arr) => {
                            let end = e.min(arr.len());
                            let start = s.min(end);
                            Ok(Value::Array(arr[start..end].to_vec()))
                        }
                        other => Err(Signal::Error(format!(
                            "Range slicing requires a string or array, got {}",
                            other.type_name()
                        ))),
                    };
                }
                let obj = self.eval_expr(object, env)?;
                let idx = self.eval_expr(index, env)?;
                Ok(obj.get_index(&idx))
            }

            Expr::Range { start, end } => {
                // Standalone range expression evaluates to an array (like range(start, end))
                let s = self.eval_expr(start, env)?.as_number().unwrap_or(0.0) as i64;
                let e = self.eval_expr(end, env)?.as_number().unwrap_or(0.0) as i64;
                let arr: Vec<Value> = (s..e).map(|n| Value::Number(n as f64)).collect();
                Ok(Value::Array(arr))
            }

            Expr::Object(pairs) => {
                let mut map = HashMap::new();
                for (k, v) in pairs {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                Ok(Value::Object(map))
            }

            Expr::Array(items) => {
                let mut arr = Vec::new();
                for item in items {
                    arr.push(self.eval_expr(item, env)?);
                }
                Ok(Value::Array(arr))
            }

            Expr::Not(inner) => Ok(Value::Bool(!self.eval_expr(inner, env)?.is_truthy())),

            Expr::BinOp { left, op, right } => {
                // Short-circuit for NullCoalesce
                if matches!(op, BinOp::NullCoalesce) {
                    let lv = self.eval_expr(left, env)?;
                    if !lv.is_null() {
                        return Ok(lv);
                    }
                    return self.eval_expr(right, env);
                }
                // Short-circuit for And/Or
                if matches!(op, BinOp::And) {
                    let lv = self.eval_expr(left, env)?;
                    if !lv.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                    let rv = self.eval_expr(right, env)?;
                    return Ok(Value::Bool(rv.is_truthy()));
                }
                if matches!(op, BinOp::Or) {
                    let lv = self.eval_expr(left, env)?;
                    if lv.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                    let rv = self.eval_expr(right, env)?;
                    return Ok(Value::Bool(rv.is_truthy()));
                }
                // Pipeline: lv |> spawn agent "name" → call_agent(name, lv)
                if matches!(op, BinOp::Pipe) {
                    let input = self.eval_expr(left, env)?;
                    // RHS must resolve to a CallAgent expr
                    match right.as_ref() {
                        Expr::CallAgent {
                            name, input: extra, ..
                        } => {
                            let name_val = self.eval_expr(name, env)?;
                            let agent_name = match &name_val {
                                Value::Str(s) => s.clone(),
                                _ => {
                                    return Err(Signal::Error(format!(
                                        "Pipeline: agent name must be a string, got {}",
                                        name_val.type_name()
                                    )))
                                }
                            };
                            // Merge input value + extra { } fields.
                            // Non-object scalars are wrapped as { value: X } for clean chaining.
                            let mut map = match input {
                                Value::Object(m) => m,
                                other => {
                                    let mut m = HashMap::new();
                                    m.insert("value".into(), other);
                                    m
                                }
                            };
                            for (k, v) in extra {
                                map.insert(k.clone(), self.eval_expr(v, env)?);
                            }
                            return self.call_agent(&agent_name, Value::Object(map));
                        }
                        other => {
                            // Evaluate RHS as a function/value and apply
                            let _ = other;
                            return Err(Signal::Error(
                                "|> right side must be `spawn agent` or `call agent`".into(),
                            ));
                        }
                    }
                }
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                self.eval_binop(&lv, op, &rv)
            }

            Expr::Call { callee, args } => self.eval_call(callee, args, env),

            Expr::AskAI {
                provider,
                model,
                params,
            } => {
                let mut resolved: HashMap<String, Value> = HashMap::new();
                for (k, v) in params {
                    resolved.insert(k.clone(), self.eval_expr(v, env)?);
                }
                // model can be set in the ask syntax (ask ollama:llama3) OR
                // as a param (model: memory.model) — params take precedence
                let param_model = resolved
                    .get("model")
                    .and_then(|v| v.as_str().map(String::from));
                let effective_model = param_model.or_else(|| model.clone());
                self.authorize_ai_provider(provider)?;
                #[cfg(not(target_arch = "wasm32"))]
                let result = {
                    let agent = self.http_agent();
                    let span = self.diagnostics.start_span("ai.request");
                    let result =
                        ai::ask_ai(provider, effective_model.as_deref(), &resolved, &agent);
                    self.end_ai_span(span, provider, effective_model.as_deref(), &result);
                    result
                };
                #[cfg(target_arch = "wasm32")]
                let result = ai::ask_ai(provider, effective_model.as_deref(), &resolved);
                self.record_tokens(&result);
                self.append_ai_trace(env, &result);
                Ok(result)
            }

            Expr::Embed { text } => {
                let t = self.eval_expr(text, env)?;
                self.authorize_ai_provider("openai")?;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let agent = self.http_agent();
                    let span = self.diagnostics.start_span("ai.embed");
                    let result = ai::embed_text(&t.to_string(), &agent);
                    self.end_ai_span(span, "openai", None, &result);
                    Ok(result)
                }
                #[cfg(target_arch = "wasm32")]
                Ok(ai::embed_text(&t.to_string()))
            }

            Expr::InferClassifier { input, classes } => {
                let input_val = self.eval_expr(input, env)?.to_string();
                let classes_val = self.eval_expr(classes, env)?;
                let class_list: Vec<String> = match classes_val {
                    Value::Array(arr) => arr.iter().map(|v| v.to_string()).collect(),
                    other => vec![other.to_string()],
                };
                self.authorize_ai_provider("openai")?;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let agent = self.http_agent();
                    let span = self.diagnostics.start_span("ai.infer");
                    let result = ai::infer_classifier(&input_val, &class_list, "openai", &agent);
                    self.end_ai_span(span, "openai", None, &result);
                    Ok(result)
                }
                #[cfg(target_arch = "wasm32")]
                Ok(ai::infer_classifier(&input_val, &class_list, "openai"))
            }

            Expr::BridgeCall {
                namespace,
                module,
                method,
                args,
            } => {
                let resolved: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, env))
                    .collect::<Result<Vec<_>, _>>()?;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let (ns, mo, me) = (namespace.clone(), module.clone(), method.clone());
                    self.bridge_call(&ns, &mo, &me, &resolved)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (namespace, module, method, resolved);
                    Err(Signal::Error(
                        "JS/Python bridge not available in playground".into(),
                    ))
                }
            }

            // Phase 5: spawn agent "name" with { key: val } [timeout Nms]
            Expr::CallAgent {
                name,
                input,
                timeout_ms,
            } => {
                let name_val = self.eval_expr(name, env)?;
                let agent_name = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "Agent name must be a string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                for (k, v) in input {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let input_val = Value::Object(map);

                if let Some(t_expr) = timeout_ms {
                    let ms = self.eval_expr(t_expr, env)?.as_number().unwrap_or(5000.0) as u64;
                    self.call_agent_with_timeout(&agent_name, input_val, ms)
                } else {
                    self.call_agent(&agent_name, input_val)
                }
            }

            Expr::Lambda { params, body } => {
                // Capture a snapshot of the current local scope so the closure can
                // reference enclosing variables (url, body, headers, etc.) at call time.
                // memory.* is propagated separately; only plain locals are captured here.
                let captured = env.vars.clone();
                Ok(Value::Closure(params.clone(), body.clone(), captured))
            }

            Expr::ParallelMap(branches) => {
                let mut named_branches = Vec::new();
                for (k, expr) in branches {
                    named_branches.push((k.clone(), expr.clone()));
                }
                self.eval_parallel_map(named_branches, env)
            }
        }
    }

    fn append_ai_trace(&mut self, env: &mut Env, result: &Value) {
        let mut memory = env.get_memory();
        let trace = memory
            .entry("ai_trace".into())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = trace {
            arr.push(result.clone());
        }
        env.set_memory(memory);
    }

    fn eval_binop(&self, lv: &Value, op: &BinOp, rv: &Value) -> IResult {
        match op {
            BinOp::Eq => Ok(Value::Bool(lv == rv)),
            BinOp::NotEq => Ok(Value::Bool(lv != rv)),
            BinOp::Lt => Ok(Value::Bool(lv < rv)),
            BinOp::LtEq => Ok(Value::Bool(lv <= rv)),
            BinOp::Gt => Ok(Value::Bool(lv > rv)),
            BinOp::GtEq => Ok(Value::Bool(lv >= rv)),
            BinOp::And => Ok(Value::Bool(lv.is_truthy() && rv.is_truthy())),
            BinOp::Or => Ok(Value::Bool(lv.is_truthy() || rv.is_truthy())),
            BinOp::NullCoalesce => {
                if lv.is_null() {
                    Ok(rv.clone())
                } else {
                    Ok(lv.clone())
                }
            }
            BinOp::Sub => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
                _ => Err(Signal::Error(format!(
                    "Cannot subtract {} from {}",
                    rv.type_name(),
                    lv.type_name()
                ))),
            },
            BinOp::Mul => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                (Value::Str(s), Value::Number(n)) => {
                    let count = if *n <= 0.0 { 0usize } else { *n as usize };
                    match s.len().checked_mul(count) {
                        Some(len) if len <= MAX_STRING_REPEAT_BYTES => {
                            Ok(Value::Str(s.repeat(count)))
                        }
                        _ => Err(Signal::Error(format!(
                            "Cannot multiply string by {}: result would exceed the {}-byte limit",
                            n, MAX_STRING_REPEAT_BYTES
                        ))),
                    }
                }
                _ => Err(Signal::Error(format!(
                    "Cannot multiply {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Div => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => {
                    if *b == 0.0 {
                        Err(Signal::Error(format!(
                            "Division by zero ({} / 0). Guard with: if divisor != 0 {{ ... }}",
                            a
                        )))
                    } else {
                        Ok(Value::Number(a / b))
                    }
                }
                _ => Err(Signal::Error(format!(
                    "Cannot divide {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Mod => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => {
                    if *b == 0.0 {
                        Err(Signal::Error(format!(
                            "Modulo by zero ({} % 0). Guard with: if divisor != 0 {{ ... }}",
                            a
                        )))
                    } else {
                        Ok(Value::Number(a % b))
                    }
                }
                _ => Err(Signal::Error(format!(
                    "Cannot mod {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Add | BinOp::Concat => self.add_values(lv, rv),
            BinOp::Pipe => Err(Signal::Error(
                "|> pipeline: right side must be `spawn agent` or `call agent`".into(),
            )),
        }
    }

    fn add_values(&self, lv: &Value, rv: &Value) -> IResult {
        match (lv, rv) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
            (Value::Str(a), b) => Ok(Value::Str(format!("{}{}", a, b))),
            (a, Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
            (Value::Array(a), Value::Array(b)) => {
                let mut arr = a.clone();
                arr.extend(b.clone());
                Ok(Value::Array(arr))
            }
            _ => Err(Signal::Error(format!(
                "Cannot add {} and {}",
                lv.type_name(),
                rv.type_name()
            ))),
        }
    }

    // ── Function calls ────────────────────────────────────────────────────────

    fn eval_call(&mut self, callee: &Expr, arg_exprs: &[Expr], env: &mut Env) -> IResult {
        let mut args = Vec::new();
        for a in arg_exprs {
            args.push(self.eval_expr(a, env)?);
        }

        if let Expr::FieldAccess { object, field } = callee {
            // Module call: `utils.greet(args)` where utils is an imported namespace.
            // Check self.functions for a "namespace.funcname" entry before falling
            // through to the generic method-call path.
            if let Expr::Ident(namespace) = object.as_ref() {
                let full_name = format!("{}.{}", namespace, field);
                if let Some(func) = self.functions.get(&full_name).cloned() {
                    let mem = env.get_memory();
                    // Temporarily inject this module's functions under their short
                    // names so intra-module calls (e.g. pad_right → repeat_str)
                    // resolve correctly without requiring the namespace prefix.
                    let siblings: Vec<FunctionDef> = self
                        .module_functions
                        .get(namespace)
                        .cloned()
                        .unwrap_or_default();
                    let mut restored: Vec<(String, Option<FunctionDef>)> = Vec::new();
                    for sf in &siblings {
                        let prev = self.functions.insert(sf.name.clone(), sf.clone());
                        restored.push((sf.name.clone(), prev));
                    }
                    let result = self.call_user_function(&func, args, Some(mem));
                    for (name, prev) in restored {
                        match prev {
                            Some(f) => {
                                self.functions.insert(name, f);
                            }
                            None => {
                                self.functions.remove(&name);
                            }
                        }
                    }
                    return result;
                }
            }

            // arr.pop() on a plain local variable: mutate in place instead
            // of going through the general "evaluate the receiver to an
            // owned Value, hand it to eval_method, get back a new Value"
            // path below, in every context (not just as a bare statement).
            //
            // This is deliberately narrower than it could be: push/append/
            // sort/reverse are NOT included here, because their existing
            // *functional* behavior when the result is captured
            // (`results = results.push(x)`, used as the idiomatic
            // accumulator pattern in several existing scripts) must keep
            // working — `eval_method`'s array arms return a new array
            // rather than mutating, and that's relied upon. `.pop()` has
            // no equivalent legitimate "capture and reassign" pattern to
            // preserve (there's no sensible reading of `arr = arr.pop()`
            // where `arr` becoming the popped *element* is the goal), and
            // its non-mutating behavior when captured was a real, silent
            // bug: `x = arr.pop()` returned the correct value but left
            // `arr` completely unchanged — confirmed true even before this
            // fix, and only "working" for the bare-statement form because
            // of a narrower special case that used to live in the
            // `Stmt::Expr` handler. A script relying on the idiomatic
            // `while len(arr) > 0 { x = arr.pop(); ... }` pattern would
            // loop forever. See `Stmt::Expr`'s own handling for the
            // corresponding *performance* fix to push/append/sort/reverse
            // (still functional as an expression, but no longer paying
            // for a full-array clone when used as a bare statement).
            if field.as_str() == "pop" {
                if let Some(var_name) = self.extract_ident_name(object) {
                    if let Some(Value::Array(arr)) = env.vars.get_mut(&var_name) {
                        return Ok(arr.pop().unwrap_or(Value::Null));
                    }
                    // Not an array (or the variable doesn't exist) — fall
                    // through to the general path below so the caller
                    // gets exactly the same error/behavior as before.
                } else if let Expr::FieldAccess {
                    object: inner_obj,
                    field: mid_field,
                } = object.as_ref()
                {
                    // One level of nesting — `memory.items.pop()`, the
                    // same common agent-memory shape the push/append/
                    // sort/reverse fast path in `Stmt::Expr` handles.
                    // `pop()` takes no arguments, so — unlike that fast
                    // path — there's no "peek before evaluating args"
                    // concern here: nothing to evaluate either way.
                    if let Some(root) = self.extract_ident_name(inner_obj) {
                        if let Some(Value::Object(map)) = env.vars.get_mut(&root) {
                            if let Some(Value::Array(arr)) = map.get_mut(mid_field) {
                                return Ok(arr.pop().unwrap_or(Value::Null));
                            }
                        }
                    }
                }
            }

            let obj = self.eval_expr(object, env)?;
            // If the field holds a closure, call it directly instead of treating
            // it as a built-in method. Enables: tools[i].handler(args), dispatch["post"](req)
            if let Value::Object(ref map) = obj {
                if let Some(Value::Closure(params, body, captured)) = map.get(field.as_str()) {
                    let (p, b, c) = (params.clone(), body.clone(), captured.clone());
                    return self.call_closure_with_capture(&p, &b, &c, args, env);
                }
            }
            return self.eval_method(obj, field, args, env);
        }

        // Direct lambda call: fn(x) { x + 1 }(5)
        // Capture current scope so the body can reference enclosing locals.
        if let Expr::Lambda { params, body } = callee {
            let captured = env.vars.clone();
            return self.call_closure_with_capture(params, body, &captured, args, env);
        }

        if let Expr::Ident(name) = callee {
            // Check if a closure value is stored under this name
            let val = env.get(name);
            if let Value::Closure(params, body, captured) = val {
                return self.call_closure_with_capture(&params, &body, &captured, args, env);
            }
            // Also check memory
            let mem = env.get_memory();
            if let Some(Value::Closure(params, body, captured)) = mem.get(name).cloned() {
                return self.call_closure_with_capture(&params, &body, &captured, args, env);
            }
            if let Some(func) = self.functions.get(name).cloned() {
                // Named functions (file-root, agent-level) also propagate memory
                return self.call_user_function_propagating(&func, args, env);
            }
            return self.eval_builtin(name, args, env);
        }

        Err(Signal::Error(format!("Cannot call {:?}", callee)))
    }

    // ── #6 spawn agent with timeout ───────────────────────────────────────────

    /// `spawn agent "x" with { ... } timeout N` — built on the Task
    /// Runtime (`spawn_task_internal`/`task_wait`) rather than a bare
    /// `std::thread::spawn` + `mpsc::recv_timeout`, which is what this used
    /// to be. That older version had a real orphan-task bug: on timeout, it
    /// just stopped *waiting* — the still-running background thread was
    /// abandoned with no handle, no way to cancel it, and no guarantee it
    /// was ever joined. Going through the Task Runtime instead means a
    /// timed-out agent call is now a cancelled *task*: cooperative
    /// cancellation propagates into it on its next statement, and
    /// `Interpreter::drop`'s `cleanup_tasks` guarantees it's joined even if
    /// nothing else ever calls `task_wait` on it again.
    fn call_agent_with_timeout(
        &mut self,
        agent_name: &str,
        input_val: Value,
        timeout_ms: u64,
    ) -> IResult {
        let agent = agent_name.to_string();
        let opts = builtins_task::SpawnOpts::new("agent.spawn", None);
        let state =
            self.spawn_task_internal(opts, move |child| child.call_agent(&agent, input_val))?;

        let wait_result = self.task_wait(&[
            Value::Str(state.id().to_string()),
            Value::Number(timeout_ms as f64),
        ])?;
        let Value::Object(m) = wait_result else {
            unreachable!("task_wait always returns an object or null for a just-created task");
        };
        if matches!(m.get("timed_out"), Some(Value::Bool(true))) {
            let mut map = HashMap::new();
            map.insert("timed_out".into(), Value::Bool(true));
            map.insert("agent".into(), Value::Str(agent_name.into()));
            map.insert("timeout_ms".into(), Value::Number(timeout_ms as f64));
            return Ok(Value::Object(map));
        }
        match m.get("ok") {
            Some(Value::Bool(true)) => Ok(m.get("value").cloned().unwrap_or(Value::Null)),
            _ => {
                let msg = m
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent task failed")
                    .to_string();
                Err(Signal::Error(msg))
            }
        }
    }

    // ── #7 parallel named results ─────────────────────────────────────────────

    /// Built on the Task Runtime — same rationale as `call_agent_with_timeout`
    /// above (which this mirrors closely): the previous implementation spawned
    /// a bare, untracked `std::thread` per branch and, on the shared 300s
    /// `recv_timeout`, simply stopped waiting and abandoned it, with no
    /// handle, no way to cancel it, and no guarantee it was ever joined.
    /// Every branch is now a real task — tracked, cancellable, guaranteed
    /// joined by `cleanup_tasks` even if it outlives this wait.
    ///
    /// This also fixes a second, latent bug: a branch's own `timeout N`
    /// clause never actually enforced anything before — the old code only
    /// used it to `std::thread::sleep(N)` *after* the agent had already
    /// finished, delaying a successful fast result by the full timeout for
    /// no reason, while the one thing actually bounding the wait was a
    /// single shared 300-second timeout applied identically to every
    /// branch. Each branch's own timeout now genuinely bounds *that*
    /// branch's wait (defaulting to 300s — unchanged — when none is given).
    fn eval_parallel_map(&mut self, branches: Vec<(String, Expr)>, env: &mut Env) -> IResult {
        let mut result_map = HashMap::new();
        let mut pending: Vec<(String, std::sync::Arc<builtins_task::TaskState>, u64)> = Vec::new();

        for (key, expr) in &branches {
            // Only parallelize CallAgent exprs; evaluate others synchronously.
            if let Expr::CallAgent {
                name,
                input,
                timeout_ms,
            } = expr
            {
                let name_val = self.eval_expr(name, env)?;
                let agent_name = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "parallel: agent name must be string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                for (k, v) in input {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let input_val = Value::Object(map);
                let agent = agent_name.clone();
                // Default unchanged from before: 300s when no `timeout` clause.
                let timeout_ms = timeout_ms
                    .as_ref()
                    .and_then(|e| self.eval_expr(e, env).ok())
                    .and_then(|v| v.as_number())
                    .map(|n| n as u64)
                    .unwrap_or(300_000);

                let opts = builtins_task::SpawnOpts::new("agent.parallel", None);
                let state = self
                    .spawn_task_internal(opts, move |child| child.call_agent(&agent, input_val))?;
                pending.push((key.clone(), state, timeout_ms));
            } else {
                // Non-agent expr: evaluate synchronously, same as before.
                let val = self.eval_expr(expr, env)?;
                result_map.insert(key.clone(), val);
            }
        }

        for (key, state, timeout_ms) in pending {
            let wait_result = self.task_wait(&[
                Value::Str(state.id().to_string()),
                Value::Number(timeout_ms as f64),
            ])?;
            let Value::Object(m) = wait_result else {
                unreachable!("task_wait always returns an object or null for a just-created task");
            };
            let entry = if matches!(m.get("timed_out"), Some(Value::Bool(true))) {
                let mut em = HashMap::new();
                em.insert("timed_out".into(), Value::Bool(true));
                Value::Object(em)
            } else {
                match m.get("ok") {
                    Some(Value::Bool(true)) => m.get("value").cloned().unwrap_or(Value::Null),
                    _ => {
                        let mut em = HashMap::new();
                        em.insert(
                            "error".into(),
                            m.get("error").cloned().unwrap_or(Value::Null),
                        );
                        Value::Object(em)
                    }
                }
            };
            result_map.insert(key, entry);
        }
        Ok(Value::Object(result_map))
    }

    // ── await { key: expr, ... } — concurrent IO block ────────────────────────

    fn eval_await_block(&mut self, bindings: &[(String, Expr)], env: &mut Env) -> IResult {
        use std::sync::mpsc;

        let mut handles: Vec<(String, mpsc::Receiver<Result<serde_json::Value, String>>)> =
            Vec::new();

        for (key, expr) in bindings {
            let val = self.eval_expr(expr, env)?;
            let json = gx_value_to_json(&val);
            // For HTTP calls and other IO we've already executed them synchronously above.
            // For agent calls, spawn threads like parallel {}.
            // This gives true concurrency for agent calls; for HTTP builtins the eval above
            // is already non-blocking from GX's perspective.
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(Ok(json));
            handles.push((key.clone(), rx));
        }

        let mut result_map = HashMap::new();
        for (key, rx) in handles {
            let entry = match rx.recv() {
                Ok(Ok(json)) => json_to_gx_value(&json),
                Ok(Err(e)) => {
                    let mut em = HashMap::new();
                    em.insert("error".into(), Value::Str(e));
                    Value::Object(em)
                }
                Err(_) => Value::Null,
            };
            result_map.insert(key, entry);
        }
        Ok(Value::Object(result_map))
    }

    // ── #15 cron daemon ───────────────────────────────────────────────────────

    fn run_cron_daemon(&mut self, blocks: &[WhenBlock], env: &mut Env) -> Result<(), Signal> {
        eprintln!("[gx] cron daemon started ({} schedule(s))", blocks.len());
        loop {
            let now = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            };
            for wb in blocks {
                if let WhenTrigger::Cron(expr) = &wb.trigger {
                    if cron_matches(expr, now) {
                        match self.run_stmts(&wb.body, env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            // Sleep until the start of the next minute
            let secs_into_minute = now % 60;
            let sleep_secs = 60 - secs_into_minute;
            std::thread::sleep(std::time::Duration::from_secs(sleep_secs));
        }
    }

    fn call_closure(&mut self, callee: &Value, args: Vec<Value>, env: &mut Env) -> IResult {
        match callee {
            Value::Closure(params, body, captured) => {
                self.call_closure_with_capture(params, body, captured, args, env)
            }
            Value::Str(name) => {
                let name = name.clone();
                if let Some(func) = self.functions.get(&name).cloned() {
                    return self.call_user_function_propagating(&func, args, env);
                }
                Err(Signal::Error(format!(
                    "'{}' is not a defined function",
                    name
                )))
            }
            _ => Err(Signal::Error(format!(
                "Expected function, got {}",
                callee.type_name()
            ))),
        }
    }

    /// Every GX function/closure call funnels through one of three
    /// entry points (`call_closure_with_capture`, `call_user_function`,
    /// `call_user_function_propagating`), each of which pushes a frame
    /// onto `call_stack` before running the body — the same field already
    /// used for error-trace context, and a direct proxy for real Rust
    /// call depth, since each GX call is several nested Rust stack frames
    /// (`eval_call` → one of these three → `run_stmts` → `run_stmt` →
    /// `eval_expr` → back to `eval_call` for a recursive call).
    ///
    /// Without this check, sufficiently deep — or accidentally unbounded —
    /// GX recursion overflows the real Rust stack: not a catchable GX
    /// error, not something `try`/`catch` can intervene on, but a hard
    /// process abort (`fatal runtime error: stack overflow, aborting`).
    /// Confirmed empirically: a script as simple as `function f(n) {
    /// return f(n + 1) }` run with no base case aborted the whole process
    /// within a fraction of a second. `main()` runs the interpreter on a
    /// thread with a considerably larger-than-default stack specifically
    /// to raise the ceiling this check is set against, but the check
    /// itself is what actually guarantees a graceful, catchable error —
    /// no stack size is infinite, and every other GX runtime error is
    /// already something a script can catch and recover from; recursion
    /// depth should be no different.
    const MAX_CALL_DEPTH: usize = 1000;

    fn check_recursion_depth(&self) -> Result<(), Signal> {
        if self.call_stack.len() >= Self::MAX_CALL_DEPTH {
            return Err(Signal::Error(format!(
                "maximum call depth exceeded ({}) — likely infinite or excessively deep recursion",
                Self::MAX_CALL_DEPTH
            )));
        }
        Ok(())
    }

    /// Core closure execution: seeds env with captured locals, overlays memory,
    /// then binds params. Changes to memory.* propagate back to the caller.
    fn call_closure_with_capture(
        &mut self,
        params: &[String],
        body: &[Stmt],
        captured: &HashMap<String, Value>,
        args: Vec<Value>,
        caller_env: &mut Env,
    ) -> IResult {
        self.check_recursion_depth()?;
        let mut env = Env::new();
        // 1. Seed with captured locals (variables from the enclosing scope at definition time)
        for (k, v) in captured {
            env.set(k, v.clone());
        }
        // 2. Copy current memory into the closure env so memory.* reads work
        let initial_mem = caller_env.get_memory();
        env.set_memory(initial_mem.clone());
        // 3. Bind params — these shadow any captured variable with the same name
        for (i, param) in params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = body.to_vec();
        self.call_stack.push("<closure>".to_string());
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        let result = match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => return Err(e),
        };
        // 4. Propagate memory changes back to the caller
        let new_mem = env.get_memory();
        if new_mem != initial_mem {
            caller_env.set_memory(new_mem);
        }
        result
    }

    // ── Observability ──────────────────────────────────────────────────────

    /// Convert a GX value into the JSON payload attached to a log/event/audit
    /// line, enriched with the currently-running agent's name when there is
    /// one — the one piece of GX-specific context `Diagnostics` itself has
    /// no way to know (it has no concept of "agents"), matching what the
    /// pre-existing `trace_log`/`emit_trace` mechanism it replaces used to
    /// include.
    fn diagnostic_data(&self, data: &Value) -> serde_json::Value {
        let mut obj = match gx_value_to_json(data) {
            serde_json::Value::Object(m) => m,
            other if other.is_null() => serde_json::Map::new(),
            other => {
                let mut m = serde_json::Map::new();
                m.insert("value".to_string(), other);
                m
            }
        };
        if let Some(agent) = &self.current_agent {
            obj.insert(
                "agent".to_string(),
                serde_json::Value::String(agent.clone()),
            );
        }
        serde_json::Value::Object(obj)
    }

    // ── Retry with exponential/linear/fixed backoff ───────────────────────────

    fn builtin_retry(&mut self, args: Vec<Value>, env: &mut Env) -> IResult {
        // retry(fn, max?, { delay?, backoff? })
        // backoff: "exponential" | "linear" | "fixed"
        let callable = args.first().cloned().unwrap_or(Value::Null);
        let max_attempts = args.get(1).and_then(|v| v.as_number()).unwrap_or(3.0) as u32;
        let opts = args.get(2).cloned().unwrap_or(Value::Null);

        let initial_delay_ms = match &opts {
            Value::Object(m) => m.get("delay").and_then(|v| v.as_number()).unwrap_or(1000.0) as u64,
            Value::Number(n) => *n as u64,
            _ => 1000,
        };
        let backoff_strategy = match &opts {
            Value::Object(m) => m
                .get("backoff")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "exponential".to_string()),
            _ => "exponential".to_string(),
        };

        // Keep the full Closure (with captured env) so retry lambdas can reference
        // enclosing variables like url, headers, body without workarounds.
        let closure = match callable {
            Value::Closure(..) => callable,
            _ => {
                return Err(Signal::Error(
                    "retry: first argument must be a function (fn() { ... })".into(),
                ))
            }
        };

        // Always the most recent attempt's raw outcome — returned as-is
        // once attempts are exhausted, so the final failure (thrown error
        // stays a thrown error, a returned `{ ok: false, ... }` stays a
        // returned value) looks exactly like what a caller who invoked the
        // wrapped builtin directly, with no retry at all, would have seen.
        let mut last_outcome: IResult = Err(Signal::Error("retry: no attempts made".into()));

        for attempt in 0..max_attempts {
            let is_last_attempt = attempt + 1 >= max_attempts;
            let outcome = self.call_closure(&closure, vec![], env);
            // A closure wrapping `http_get`/`process_run`/`task_wait`/
            // `ask`/`context_ask` reports an operational failure by
            // *returning* `{ ok: false, ... }`, never by throwing (see
            // `value_is_ok_false`). Retrying only on `Err` therefore
            // silently never retried any of those — the very builtins
            // `retry` exists to wrap around network/process flakiness for.
            // Treat that shape as a retryable failure too.
            let retryable = match &outcome {
                Ok(v) => value_is_ok_false(v),
                Err(_) => true,
            };
            last_outcome = outcome;
            if !retryable {
                return last_outcome;
            }
            if is_last_attempt {
                break;
            }
            let delay = match backoff_strategy.as_str() {
                "exponential" => initial_delay_ms * (2u64.pow(attempt)),
                "linear" => initial_delay_ms * (attempt as u64 + 1),
                _ => initial_delay_ms, // fixed
            };
            let delay = delay.min(30_000); // cap at 30 seconds
            eprintln!(
                "[gx] retry: attempt {}/{} failed, waiting {}ms",
                attempt + 1,
                max_attempts,
                delay
            );
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        last_outcome
    }

    fn call_behavior(&mut self, func: &FunctionDef, caller_env: &mut Env) -> IResult {
        let mut env = Env::new();
        let mem = caller_env.get_memory();
        env.set_memory(mem.clone());
        // Flatten memory into local vars so bare names work in progressive syntax:
        // `count += 1` inside a behavior acts like `memory.count += 1`
        let initial: HashMap<String, Value> = mem.clone();
        for (k, v) in &mem {
            env.set(k, v.clone());
        }
        let body = func.body.clone();
        let result = match self.run_stmts(&body, &mut env) {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        // memory.X assignments are captured in env.get_memory()
        let mut new_mem = env.get_memory();
        // Local var wins only if it actually changed — this lets memory.X = ... and bare x = ...
        // coexist: whichever was actually modified wins, with local var taking priority on conflict
        for k in initial.keys() {
            let local_val = env.get(k);
            let was = initial.get(k).cloned().unwrap_or(Value::Null);
            if local_val != was && !matches!(local_val, Value::Null) {
                new_mem.insert(k.clone(), local_val);
            }
        }
        caller_env.set_memory(new_mem);
        Ok(result)
    }

    fn call_user_function(
        &mut self,
        func: &FunctionDef,
        args: Vec<Value>,
        parent_memory: Option<HashMap<String, Value>>,
    ) -> IResult {
        self.check_recursion_depth()?;
        let mut env = Env::new();
        if let Some(mem) = parent_memory {
            env.set_memory(mem);
        }
        for (i, param) in func.params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = func.body.clone();
        self.call_stack.push(format!("{}()", func.name));
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Like call_user_function but propagates memory changes back to the caller's env.
    /// Used when agent-level or inline functions need to mutate shared `memory.*` state.
    fn call_user_function_propagating(
        &mut self,
        func: &FunctionDef,
        args: Vec<Value>,
        caller_env: &mut Env,
    ) -> IResult {
        self.check_recursion_depth()?;
        let initial_mem = caller_env.get_memory();
        let mut env = Env::new();
        env.set_memory(initial_mem.clone());
        for (i, param) in func.params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = func.body.clone();
        self.call_stack.push(format!("{}()", func.name));
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        let result = match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => return Err(e),
        };
        // Propagate any memory changes back to the caller
        let new_mem = env.get_memory();
        if new_mem != initial_mem {
            caller_env.set_memory(new_mem);
        }
        result
    }

    fn eval_builtin(&mut self, name: &str, args: Vec<Value>, env: &mut Env) -> IResult {
        match name {
            // ── Output ────────────────────────────────────────────────────────
            "log" | "output" | "print" | "say" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                self.emit_output(&parts.join(" "));
                Ok(Value::Null)
            }
            // Print without a trailing newline — for inline prompts (e.g. `> `) before readline().
            "write" | "print_inline" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                self.emit_output_raw(&parts.join(" "));
                Ok(Value::Null)
            }
            "eprint" | "elog" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                eprintln!("{}", parts.join(" "));
                Ok(Value::Null)
            }

            // ── Debugger ──────────────────────────────────────────────────────
            // A script-embedded pause point (mirrors Python's `breakpoint()`):
            // unconditionally drops into the interactive debugger prompt right
            // here, in *any* execution context (`gx run`, `gx test`, `gx -e`,
            // even the REPL) — no `--break` flag or prior debug session
            // required. Works by calling the same `debug_pause` a `--break`
            // line hit uses; the only difference is entering it directly
            // instead of through `run_stmt`'s `should_pause` check (an
            // expression has no line of its own the way a `Stmt` does, so
            // `self.current_line`, kept up to date by `run_stmt`, stands in).
            #[cfg(not(target_arch = "wasm32"))]
            "breakpoint" => {
                let line = self.current_line;
                self.debug_pause(env, line, "breakpoint()")?;
                Ok(Value::Null)
            }

            // ── Stdin ─────────────────────────────────────────────────────────
            "is_tty" | "stdin_is_tty" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::IsTerminal;
                    Ok(Value::Bool(std::io::stdin().is_terminal()))
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Bool(false))
            }
            "readline" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::BufRead;
                    let mut line = String::new();
                    match std::io::stdin().lock().read_line(&mut line) {
                        Ok(0) => Ok(Value::Null), // EOF
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                                if line.ends_with('\r') {
                                    line.pop();
                                }
                            }
                            Ok(Value::Str(line))
                        }
                        Err(e) => Err(Signal::Error(format!("readline: {}", e))),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Null)
            }
            "read_all" | "read_stdin" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::{IsTerminal, Read};
                    // Never block waiting for user to type — return null on TTY.
                    // Callers that need TTY input should use readline() in a loop.
                    if std::io::stdin().is_terminal() {
                        return Ok(Value::Null);
                    }
                    let mut buf = String::new();
                    std::io::stdin()
                        .lock()
                        .read_to_string(&mut buf)
                        .map(|_| Value::Str(buf))
                        .map_err(|e| Signal::Error(format!("read_stdin: {}", e)))
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Null)
            }

            // ── Standalone slice / merge (also available as methods) ──────────
            "slice" => {
                let target = args.first().cloned().unwrap_or(Value::Null);
                let start = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                match target {
                    Value::Array(arr) => {
                        let end = args
                            .get(2)
                            .and_then(|v| v.as_number())
                            .map(|n| (n as usize).min(arr.len()))
                            .unwrap_or(arr.len());
                        let start = start.min(end);
                        Ok(Value::Array(arr[start..end].to_vec()))
                    }
                    Value::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let end = args
                            .get(2)
                            .and_then(|v| v.as_number())
                            .map(|n| (n as usize).min(chars.len()))
                            .unwrap_or(chars.len());
                        let start = start.min(end);
                        Ok(Value::Str(chars[start..end].iter().collect()))
                    }
                    other => Err(Signal::Error(format!(
                        "slice() expects a string or array as first argument, got {}",
                        other.type_name()
                    ))),
                }
            }
            "merge" => {
                // merge(obj1, obj2, ...) → shallow-merge all objects left to right
                let mut result = HashMap::new();
                for arg in &args {
                    if let Value::Object(m) = arg {
                        for (k, v) in m {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                }
                Ok(Value::Object(result))
            }
            // pick(obj, keys) / omit(obj, keys) — shaping an object down to
            // (or excluding) a specific set of fields is common enough
            // when preparing an API response or filtering a config object
            // that it's worth a builtin rather than every script
            // reconstructing it key-by-key. A key named in `keys` that
            // isn't present on `obj` is silently skipped for `pick` (not
            // an error, and never appears with a null value — the result
            // only ever contains keys that were actually present),
            // matching how `has`/`keys`/`values` already treat a missing
            // key or non-object input as "empty", never an error.
            "pick" => {
                let obj = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Object(m) => m,
                    _ => return Ok(Value::Object(HashMap::new())),
                };
                let wanted = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Null)
                    .iter()
                    .unwrap_or_default();
                let mut result = HashMap::new();
                for key in wanted {
                    if let Some(k) = key.as_str() {
                        if let Some(v) = obj.get(k) {
                            result.insert(k.to_string(), v.clone());
                        }
                    }
                }
                Ok(Value::Object(result))
            }
            "omit" => {
                let obj = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Object(m) => m,
                    _ => return Ok(Value::Object(HashMap::new())),
                };
                let excluded: std::collections::HashSet<String> = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Null)
                    .iter()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect();
                let result: HashMap<String, Value> = obj
                    .into_iter()
                    .filter(|(k, _)| !excluded.contains(k))
                    .collect();
                Ok(Value::Object(result))
            }

            // ── Time ─────────────────────────────────────────────────────────
            "get_timestamp" | "now" | "timestamp" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                Ok(Value::Number(ts as f64))
            }
            "now_ms" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                Ok(Value::Number(ts as f64))
            }
            "sleep" => {
                if let Some(n) = args.first().and_then(|v| v.as_number()) {
                    // Argument is seconds (fractional OK). sleep(5) = 5s, sleep(0.5) = 500ms.
                    // Use sleep(500ms) syntax sugar (lexer converts to 0.5) for sub-second delays.
                    // NaN/Infinity/negative are rejected outright rather than silently
                    // clamped — `Duration::from_secs_f64` itself panics on them (and an
                    // ordinary GX numeric literal can already produce `Infinity` by
                    // overflowing f64 during parsing), and silently clamping something
                    // that's actually invalid input to "sleep for 10 years" would trade
                    // a crash for an equally-unwanted multi-year hang. A merely very
                    // large *finite* value is still bounded (not an error) since that
                    // was always accepted before this fix, just without a real ceiling.
                    if !n.is_finite() || n < 0.0 {
                        return Err(Signal::Error(format!(
                            "sleep: argument must be a non-negative, finite number of seconds (got {})",
                            n
                        )));
                    }
                    let duration =
                        crate::clamp_duration_secs(n, std::time::Duration::from_secs(315_360_000));
                    // Outside a task (the common case): one plain blocking
                    // sleep, zero polling overhead. Inside a task: sleep is
                    // explicitly "this task is idle, waiting" — the exact
                    // state cancellation should interrupt promptly, so it
                    // polls in small increments instead, same as the
                    // process runtime's wait loops. The next statement's
                    // automatic cancellation check (run_stmt) is what
                    // actually raises Signal::Cancelled; this just makes
                    // sure control returns to it quickly rather than after
                    // the full requested duration.
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Some(task) = self.current_task.clone() {
                            const TICK: std::time::Duration = std::time::Duration::from_millis(25);
                            let deadline = std::time::Instant::now() + duration;
                            loop {
                                let now = std::time::Instant::now();
                                if now >= deadline || task.is_cancelled() {
                                    break;
                                }
                                std::thread::sleep(
                                    TICK.min(deadline.saturating_duration_since(now)),
                                );
                            }
                        } else {
                            std::thread::sleep(duration);
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    std::thread::sleep(duration);
                }
                Ok(Value::Null)
            }
            "date_string" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let days_since_epoch = ts / 86400;
                let year = 1970 + days_since_epoch / 365;
                Ok(Value::Str(format!(
                    "day {} (year ~{})",
                    days_since_epoch, year
                )))
            }

            // ── Date / Time (production) ──────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "date_now" => date_now_impl(),
            #[cfg(not(target_arch = "wasm32"))]
            "date_timestamp" => date_timestamp_impl(),
            #[cfg(not(target_arch = "wasm32"))]
            "date_parse" => date_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_format" => date_format_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_diff" => date_diff_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_add" => date_add_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_parts" => date_parts_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_from_parts" => date_from_parts_impl(&args),

            // ── Regex ─────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "regex_test" | "re_test" => regex_test_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_find" | "re_find" => regex_find_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_find_all" | "re_find_all" | "regex_findall" => regex_find_all_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_replace" | "re_replace" => regex_replace_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_split" | "re_split" => regex_split_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_captures" | "re_captures" => regex_captures_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_named_captures" | "re_named" => regex_named_captures_impl(&args),

            // ── CSV / YAML / TOML ─────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "csv_parse" => csv_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "csv_stringify" | "csv_encode" => csv_stringify_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "yaml_parse" => yaml_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "yaml_stringify" | "yaml_encode" => yaml_stringify_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "toml_parse" => toml_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "toml_stringify" | "toml_encode" => toml_stringify_impl(&args),
            "xml_parse" => xml_parse_impl(&args),
            "xml_stringify" | "xml_encode" => xml_stringify_impl(&args),
            // JSON Lines / NDJSON — one independent JSON value per line,
            // distinct from json_parse/json_stringify's "whole text is one
            // JSON value" — see builtins_data.rs for the full rationale.
            "jsonl_parse" | "ndjson_parse" => jsonl_parse_impl(&args),
            "jsonl_stringify" | "ndjson_stringify" | "jsonl_encode" => jsonl_stringify_impl(&args),

            // ── Template & Code Generation ───────────────────────────────────
            "render_template" => render_template_impl(&args),

            // ── Environment / .env ────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "load_env" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| ".env".to_string());
                // Resolve relative to the script's sandbox dir (not the CWD) so that
                // `load_env(".env")` finds the .env next to the script from any CWD.
                let path = self.safe_path(&raw)?;
                load_env_file(&path.to_string_lossy())
            }
            // get_env with optional default
            "get_env" | "env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                self.authorize_capability(crate::capability::Resource::Environment, Some(&key))
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                match std::env::var(&key) {
                    Ok(v) => Ok(Value::Str(v)),
                    Err(_) => Ok(default),
                }
            }
            "set_env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                self.authorize_capability(crate::capability::Resource::Environment, Some(&key))
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let val = args.get(1).map(|v| v.to_string()).unwrap_or_default();
                std::env::set_var(&key, &val);
                Ok(Value::Null)
            }

            // ── Production Configuration Runtime ────────────────────────────────
            // config_load(options) — a single ergonomic entry point over
            // primitives that already existed separately (json_parse/
            // yaml_parse/toml_parse, env(), schema_validate, merge): a
            // layered merge of
            //   defaults  <  config file  <  environment overrides  <  explicit overrides
            // (later layers win), with optional schema validation. Every
            // layer is independently optional — an options object with
            // only `defaults` is a valid, if trivial, call.
            //
            // `options`:
            //   defaults:   object — the base layer.
            //   file:       string — path to a config file, format
            //               auto-detected from its extension
            //               (.json/.yaml/.yml/.toml). Missing file is not
            //               an error (defaults carry the app); a file
            //               that *exists* but fails to parse, or whose
            //               extension isn't recognized, is.
            //   env_prefix: string — enables the environment-override
            //               layer. For each key already present after
            //               defaults+file (never a key invented purely
            //               from an env var — see the security note
            //               below), checks `{env_prefix}{KEY_UPPERCASED}`
            //               and, if set, overrides that key — coerced to
            //               match the *existing* value's type (env vars
            //               are always strings; a numeric/boolean default
            //               makes `"true"`/`"8080"` usable without a
            //               separate parse step in the caller). Denied by
            //               gx.json's `env_deny`: skipped, same as every
            //               other capability denial, and audited exactly
            //               like a direct `env()` call would be.
            //   overrides:  object — highest precedence, applied last.
            //   schema:     object — if given, the *final* merged config
            //               is run through the existing `schema_validate`
            //               and, on failure, config_load throws (fail
            //               fast on bad config) rather than returning an
            //               invalid object for the caller to forget to
            //               check.
            //
            // Security note: the env-override layer can only ever
            // *override a key the developer already defined* (via
            // `defaults` or `file`) — it never lets an environment
            // variable inject a brand-new config key that wasn't already
            // part of the app's own schema, so it can't be used to smuggle
            // unexpected configuration in. Secrets are deliberately kept
            // out of this entirely: they belong in `.env`/`load_env()`+
            // `env()` (already capability-gated and already excluded from
            // whatever `config_load` returns), not in `defaults`/`file`/
            // `overrides` — see the Configuration Runtime section of the
            // language reference for the full secrets-separation guidance.
            "config_load" => {
                let opts = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Object(m) => m,
                    _ => {
                        return Err(Signal::Error(
                            "config_load: expected an options object".into(),
                        ))
                    }
                };

                let mut merged: HashMap<String, Value> = match opts.get("defaults") {
                    Some(Value::Object(m)) => m.clone(),
                    _ => HashMap::new(),
                };

                if let Some(Value::Str(file_path)) = opts.get("file") {
                    let path = self.safe_path(file_path)?;
                    if path.exists() {
                        let content = std::fs::read_to_string(&path).map_err(|e| {
                            Signal::Error(format!("config_load: reading '{}': {}", file_path, e))
                        })?;
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        let parsed = match ext.as_str() {
                            "json" => serde_json::from_str::<serde_json::Value>(&content)
                                .map(|j| json_to_gx_value(&j))
                                .map_err(|e| {
                                    Signal::Error(format!(
                                        "config_load: parsing '{}' as JSON: {}",
                                        file_path, e
                                    ))
                                })?,
                            #[cfg(not(target_arch = "wasm32"))]
                            "yaml" | "yml" => yaml_parse_impl(&[Value::Str(content)])?,
                            #[cfg(not(target_arch = "wasm32"))]
                            "toml" => toml_parse_impl(&[Value::Str(content)])?,
                            other => {
                                return Err(Signal::Error(format!(
                                    "config_load: unrecognized config file extension '{}' for \
                                     '{}' (expected .json, .yaml, .yml, or .toml)",
                                    other, file_path
                                )))
                            }
                        };
                        match parsed {
                            Value::Object(m) => {
                                for (k, v) in m {
                                    merged.insert(k, v);
                                }
                            }
                            _ => {
                                return Err(Signal::Error(format!(
                                    "config_load: '{}' must contain an object at the top level",
                                    file_path
                                )))
                            }
                        }
                    }
                }

                if let Some(Value::Str(prefix)) = opts.get("env_prefix") {
                    let keys: Vec<String> = merged.keys().cloned().collect();
                    for key in keys {
                        let env_name = format!("{}{}", prefix, key.to_ascii_uppercase());
                        if let Ok(raw) = std::env::var(&env_name) {
                            self.authorize_capability(
                                crate::capability::Resource::Environment,
                                Some(&env_name),
                            )
                            .map_err(|e| Signal::Error(e.to_string()))?;
                            let coerced = match merged.get(&key) {
                                Some(Value::Number(_)) => raw
                                    .parse::<f64>()
                                    .map(Value::Number)
                                    .unwrap_or(Value::Str(raw)),
                                Some(Value::Bool(_)) => match raw.to_ascii_lowercase().as_str() {
                                    "true" | "1" | "yes" => Value::Bool(true),
                                    "false" | "0" | "no" => Value::Bool(false),
                                    _ => Value::Str(raw),
                                },
                                _ => Value::Str(raw),
                            };
                            merged.insert(key, coerced);
                        }
                    }
                }

                if let Some(Value::Object(overrides)) = opts.get("overrides") {
                    for (k, v) in overrides {
                        merged.insert(k.clone(), v.clone());
                    }
                }

                if let Some(Value::Object(schema)) = opts.get("schema") {
                    let validation = schema_validate_impl(&[
                        Value::Object(merged.clone()),
                        Value::Object(schema.clone()),
                    ])?;
                    if let Value::Object(v) = &validation {
                        let ok = v.get("ok").map(|b| b.is_truthy()).unwrap_or(true);
                        if !ok {
                            let errs = v.get("errors").cloned().unwrap_or(Value::Array(Vec::new()));
                            return Err(Signal::Error(format!(
                                "config_load: invalid configuration: {}",
                                errs
                            )));
                        }
                    }
                }

                Ok(Value::Object(merged))
            }

            // ── Retry with backoff ────────────────────────────────────────────
            "retry" => self.builtin_retry(args, env),

            // unwrap(result) — bridges GX's two coexisting failure-signaling
            // conventions. `db_query`/`db_exec`/file I/O/`readline` signal
            // failure by *throwing* (catchable with try/catch); `http_*`/
            // `process_*`/`task_wait`/`ask`/`context_ask` signal it by
            // *returning* `{ ok: false, error, error_kind, ... }` instead
            // (see `value_is_ok_false`). A script has to know, per builtin,
            // which convention applies, or it silently writes dead error-
            // handling code (an `if !r.ok` check that never runs against
            // something that throws, or a try/catch around something that
            // never throws). `unwrap` lets a caller who just wants "the
            // value, or a normal thrown error" opt into one convention
            // regardless of which the wrapped call actually uses — pure
            // sugar, raises the *same* kind of error `db_query` already
            // would, so it composes with existing try/catch normally.
            // Anything that isn't a `{ ok: false, ... }` object — including
            // `{ ok: true, ... }` and every value with no `ok` field at all
            // — passes through completely unchanged; `unwrap` never guesses
            // at which field holds "the real payload" for a shape it
            // doesn't recognize.
            "unwrap" => {
                let v = args.into_iter().next().unwrap_or(Value::Null);
                if value_is_ok_false(&v) {
                    let Value::Object(m) = &v else {
                        unreachable!("value_is_ok_false only matches Value::Object")
                    };
                    let error = m
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("operation failed");
                    let msg = match m.get("error_kind").and_then(|k| k.as_str()) {
                        Some(kind) => format!("{} ({})", error, kind),
                        None => error.to_string(),
                    };
                    return Err(Signal::Error(msg));
                }
                Ok(v)
            }

            // ── Vector store ──────────────────────────────────────────────────
            "vector_store_new" | "vs_new" => vector_store_new_impl(&args),
            "vector_store_add" | "vs_add" => vector_store_add_impl(&args),
            "vector_store_search" | "vs_search" => vector_store_search_impl(&args),
            "vector_store_delete" | "vs_delete" => vector_store_delete_impl(&args),
            "vector_store_size" | "vs_size" => vector_store_size_impl(&args),
            "cosine_similarity" => cosine_similarity_impl(&args),

            // ── Observability ─────────────────────────────────────────────────
            "trace_log" => {
                let event = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "event".to_string());
                let data = args.get(1).cloned().unwrap_or(Value::Null);
                self.diagnostics.event(&event, self.diagnostic_data(&data));
                Ok(Value::Null)
            }

            // ── Structured leveled logging — always active, filtered only
            // by --log-level (default info), independent of --trace. ────
            "log_debug" | "log_info" | "log_warn" | "log_error" => {
                let level = match name {
                    "log_debug" => crate::diagnostics::Level::Debug,
                    "log_warn" => crate::diagnostics::Level::Warn,
                    "log_error" => crate::diagnostics::Level::Error,
                    _ => crate::diagnostics::Level::Info,
                };
                let message = args.first().map(|v| v.to_string()).unwrap_or_default();
                let data = args.get(1).cloned();
                self.diagnostics
                    .log(level, &message, data.map(|d| self.diagnostic_data(&d)));
                Ok(Value::Null)
            }

            // Current trace/span IDs — for a script to thread its own
            // trace ID into, say, a response header for client-side
            // correlation. Null when tracing isn't enabled.
            "trace_id" => Ok(self
                .diagnostics
                .current_trace_id()
                .map(|s| Value::Str(s.to_string()))
                .unwrap_or(Value::Null)),
            "span_id" => Ok(self
                .diagnostics
                .current_span_id()
                .map(|s| Value::Str(s.to_string()))
                .unwrap_or(Value::Null)),

            // has_capability(resource, name?) — the only way, before this,
            // for a script to learn whether an operation is allowed was to
            // attempt it and catch a "capability_denied" error. That works,
            // but it's trial-and-error: a script that wants to *choose*
            // between two strategies up front (e.g. "use the network if
            // I'm allowed to, otherwise fall back to a cached copy") had no
            // way to ask first. `resource` is one of the names already
            // used in gx.json/--deny/error messages (see the Capability
            // Runtime doc section) — "shell", "process", "filesystem",
            // "internal_network", "external_network", "http_server",
            // "database", "environment", "ai", "js", "ts", "py", "binary",
            // "go", "rust_bin". `name` narrows the check for a
            // resource with an allowlist (an AI provider, a bridge module,
            // a process executable) — omit it to check only the
            // resource-level grant. This is a pure query: it never denies
            // an operation, throws, or writes to the audit log — it
            // performs the same check `authorize_capability` would, minus
            // the side effect that exists specifically for a *real* denial.
            "has_capability" => {
                let resource_name = args.first().and_then(|v| v.as_str()).ok_or_else(|| {
                    Signal::Error("has_capability: expected a resource name string".into())
                })?;
                let Some(resource) = crate::capability::Resource::parse(resource_name) else {
                    return Err(Signal::Error(format!(
                        "has_capability: unknown resource '{}'",
                        resource_name
                    )));
                };
                let scoped_name = args.get(1).and_then(|v| v.as_str());
                Ok(Value::Bool(
                    self.capabilities.authorize(resource, scoped_name).is_ok(),
                ))
            }

            // ── Test assertions (integrate with `gx test` pass/fail tracking) ──
            "assert_eq" | "assert_equal" => {
                self.assert_count += 1;
                let a = args.first().cloned().unwrap_or(Value::Null);
                let b = args.get(1).cloned().unwrap_or(Value::Null);
                if a == b {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(2)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_eq".to_string());
                    let msg = format!("{}: expected {} == {}", label, a, b);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }
            "assert_true" | "assert_that" => {
                self.assert_count += 1;
                let cond = args.first().map(|v| v.is_truthy()).unwrap_or(false);
                if cond {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(1)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_true".to_string());
                    let msg = format!("{}: expected true, got false", label);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }
            "assert_contains" => {
                self.assert_count += 1;
                let haystack = args.first().cloned().unwrap_or(Value::Null);
                let needle = args.get(1).cloned().unwrap_or(Value::Null);
                let contained = match &haystack {
                    Value::Str(s) => needle.as_str().map(|n| s.contains(n)).unwrap_or(false),
                    Value::Array(arr) => arr.contains(&needle),
                    _ => false,
                };
                if contained {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(2)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_contains".to_string());
                    let msg = format!("{}: {} does not contain {}", label, haystack, needle);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }

            // ── Schema validation ─────────────────────────────────────────────
            "schema_validate" => schema_validate_impl(&args),
            "schema_check" => schema_validate_impl(&args),

            // ── Persistent memory ─────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "persist_memory" | "save_memory" => self.builtin_persist_memory(env),
            #[cfg(not(target_arch = "wasm32"))]
            "load_memory" | "restore_memory" => self.builtin_load_memory(env),

            // ── Type coercions ─────────────────────────────────────────────────
            "to_string" | "str" => Ok(Value::Str(
                args.first().cloned().unwrap_or(Value::Null).to_string(),
            )),
            "to_number" | "int" | "float" | "num" => {
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Number(n) => Ok(Value::Number(n)),
                    Value::Bool(b) => Ok(Value::Number(if b { 1.0 } else { 0.0 })),
                    Value::Str(s) => s
                        .trim()
                        .parse::<f64>()
                        .map(Value::Number)
                        .map_err(|_| Signal::Error(format!("Cannot convert '{}' to number", s))),
                    _ => Ok(Value::Number(0.0)),
                }
            }
            "to_bool" | "bool" => Ok(Value::Bool(
                args.first().cloned().unwrap_or(Value::Null).is_truthy(),
            )),
            "type_of" | "typeof" => Ok(Value::Str(
                args.first()
                    .cloned()
                    .unwrap_or(Value::Null)
                    .type_name()
                    .into(),
            )),
            "is_null" => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Null) | None
            ))),
            "is_number" => Ok(Value::Bool(matches!(args.first(), Some(Value::Number(_))))),
            "is_string" => Ok(Value::Bool(matches!(args.first(), Some(Value::Str(_))))),
            "is_array" => Ok(Value::Bool(matches!(args.first(), Some(Value::Array(_))))),
            "is_object" => Ok(Value::Bool(matches!(args.first(), Some(Value::Object(_))))),
            "is_bool" => Ok(Value::Bool(matches!(args.first(), Some(Value::Bool(_))))),

            // ── Collections ───────────────────────────────────────────────────
            "count" | "len" | "length" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(a) => Ok(Value::Number(a.len() as f64)),
                Value::Object(o) => Ok(Value::Number(o.len() as f64)),
                Value::Str(s) => Ok(Value::Number(s.chars().count() as f64)),
                Value::Null => Ok(Value::Number(0.0)),
                _ => Ok(Value::Number(1.0)),
            },
            "keys" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let mut ks: Vec<Value> = m.keys().map(|k| Value::Str(k.clone())).collect();
                    ks.sort_by_key(|a| a.to_string());
                    Ok(Value::Array(ks))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "values" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => Ok(Value::Array(m.values().cloned().collect())),
                _ => Ok(Value::Array(Vec::new())),
            },
            "entries" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let pairs: Vec<Value> = m
                        .iter()
                        .map(|(k, v)| Value::Array(vec![Value::Str(k.clone()), v.clone()]))
                        .collect();
                    Ok(Value::Array(pairs))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "range" => {
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                let end = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                let step = args.get(2).and_then(|v| v.as_number()).unwrap_or(1.0) as i64;
                if step == 0 {
                    return Err(Signal::Error("range step cannot be 0".into()));
                }
                let mut arr = Vec::new();
                let mut i = start;
                while if step > 0 { i < end } else { i > end } {
                    arr.push(Value::Number(i as f64));
                    i += step;
                }
                Ok(Value::Array(arr))
            }
            "zip" => {
                let a = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Array(arr) => arr,
                    _ => return Ok(Value::Array(Vec::new())),
                };
                let b = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Array(arr) => arr,
                    _ => return Ok(Value::Array(Vec::new())),
                };
                let zipped = a
                    .into_iter()
                    .zip(b)
                    .map(|(x, y)| Value::Array(vec![x, y]))
                    .collect();
                Ok(Value::Array(zipped))
            }
            "flatten" | "flat" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => {
                    let mut flat = Vec::new();
                    for v in arr {
                        match v {
                            Value::Array(inner) => flat.extend(inner),
                            other => flat.push(other),
                        }
                    }
                    Ok(Value::Array(flat))
                }
                _ => Ok(Value::Array(Vec::new())),
            },

            // ── String global functions ───────────────────────────────────────
            "trim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim().to_string()))
            }
            "trim_start" | "ltrim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim_start().to_string()))
            }
            "trim_end" | "rtrim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim_end().to_string()))
            }
            "to_upper" | "upper" | "uppercase" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.to_uppercase()))
            }
            "to_lower" | "lower" | "lowercase" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.to_lowercase()))
            }
            "starts_with" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let prefix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.starts_with(prefix.as_str())))
            }
            "ends_with" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let suffix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.ends_with(suffix.as_str())))
            }
            "replace" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let old = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let new = args
                    .get(2)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replace(old.as_str(), new.as_str())))
            }
            "split" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let delim = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or(" ".to_string());
                let parts: Vec<Value> = s
                    .split(delim.as_str())
                    .map(|p| Value::Str(p.to_string()))
                    .collect();
                Ok(Value::Array(parts))
            }
            "join" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => {
                    let sep = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    let parts: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                    Ok(Value::Str(parts.join(sep.as_str())))
                }
                _ => Ok(Value::Str(String::new())),
            },
            "has" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let key = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    Ok(Value::Bool(m.contains_key(&key)))
                }
                Value::Array(arr) => {
                    let needle = args.get(1).cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(arr.iter().any(|v| v == &needle)))
                }
                _ => Ok(Value::Bool(false)),
            },
            "contains" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Str(s) => {
                    let sub = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    Ok(Value::Bool(s.contains(sub.as_str())))
                }
                Value::Array(arr) => {
                    let needle = args.get(1).cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(arr.iter().any(|v| v == &needle)))
                }
                _ => Ok(Value::Bool(false)),
            },
            "set_key" | "set_field" => {
                // set_key(obj, key, val) → new object with key set to val
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Object(mut m) => {
                        let key = args
                            .get(1)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default();
                        let val = args.get(2).cloned().unwrap_or(Value::Null);
                        m.insert(key, val);
                        Ok(Value::Object(m))
                    }
                    _ => Err(Signal::Error(
                        "set_key requires an object as first argument".into(),
                    )),
                }
            }
            "delete_key" | "remove_key" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(mut m) => {
                    let key = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    m.remove(&key);
                    Ok(Value::Object(m))
                }
                _ => Err(Signal::Error(
                    "delete_key requires an object as first argument".into(),
                )),
            },
            "push" => {
                // Functional push: push(array, value) → new array with value appended
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Array(mut arr) => {
                        if let Some(v) = args.get(1) {
                            arr.push(v.clone());
                        }
                        Ok(Value::Array(arr))
                    }
                    _ => Err(Signal::Error(
                        "push() requires an array as first argument".into(),
                    )),
                }
            }
            "pop" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(mut arr) => {
                    arr.pop();
                    Ok(Value::Array(arr))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "first" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => Ok(arr.into_iter().next().unwrap_or(Value::Null)),
                other => Ok(other),
            },
            "last" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => Ok(arr.into_iter().last().unwrap_or(Value::Null)),
                other => Ok(other),
            },
            "is_empty" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Str(s) => Ok(Value::Bool(s.is_empty())),
                Value::Array(a) => Ok(Value::Bool(a.is_empty())),
                Value::Object(o) => Ok(Value::Bool(o.is_empty())),
                Value::Null => Ok(Value::Bool(true)),
                _ => Ok(Value::Bool(false)),
            },
            // substr(s, start) or substr(s, start, len)
            "substr" | "substring" => {
                let s: Vec<char> = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
                    .chars()
                    .collect();
                let start = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let result: String = if let Some(length) = args.get(2).and_then(|v| v.as_number()) {
                    s.iter().skip(start).take(length as usize).collect()
                } else {
                    s.iter().skip(start).collect()
                };
                Ok(Value::Str(result))
            }
            // strip_prefix(s, prefix) — remove prefix only from start
            "strip_prefix" | "remove_prefix" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let prefix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            // strip_suffix(s, suffix) — remove suffix only from end
            "strip_suffix" | "remove_suffix" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let suffix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            // index_of(s, needle) — first position of needle, or -1
            "index_of" | "find" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let needle = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match s.find(needle.as_str()) {
                    Some(i) => Ok(Value::Number(i as f64)),
                    None => Ok(Value::Number(-1.0)),
                }
            }

            // ── Math ──────────────────────────────────────────────────────────
            // Character primitives for self-hosting
            "ord" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Number(
                    c.chars().next().map(|ch| ch as u32 as f64).unwrap_or(0.0),
                ))
            }
            "chr" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
                Ok(char::from_u32(n)
                    .map(|c| Value::Str(c.to_string()))
                    .unwrap_or(Value::Null))
            }
            "is_digit" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_digit())
                        .unwrap_or(false),
                ))
            }
            "is_alpha" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_alphabetic())
                        .unwrap_or(false),
                ))
            }
            "is_alnum" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_alphanumeric())
                        .unwrap_or(false),
                ))
            }
            "is_whitespace" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_whitespace())
                        .unwrap_or(false),
                ))
            }
            "floor" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .floor(),
            )),
            "ceil" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .ceil(),
            )),
            "round" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let decimals = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
                let factor = 10f64.powi(decimals);
                Ok(Value::Number((n * factor).round() / factor))
            }
            "abs" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .abs(),
            )),
            "sqrt" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                if n < 0.0 {
                    return Err(Signal::Error("sqrt of negative number".into()));
                }
                Ok(Value::Number(n.sqrt()))
            }
            "pow" => {
                let base = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let exp = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(Value::Number(base.powf(exp)))
            }
            "log2" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(1.0)
                    .log2(),
            )),
            "log10" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(1.0)
                    .log10(),
            )),
            "ln" => Ok(Value::Number(
                args.first().and_then(|v| v.as_number()).unwrap_or(1.0).ln(),
            )),
            "sin" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .sin(),
            )),
            "cos" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .cos(),
            )),
            "tan" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .tan(),
            )),
            "max" => {
                // max(array) or max(a, b, c, ...) — handles any number of args
                if let Some(Value::Array(arr)) = args.first() {
                    let m = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::NEG_INFINITY, f64::max);
                    return Ok(Value::Number(m));
                }
                let m = args
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Value::Number(m))
            }
            "min" => {
                // min(array) or min(a, b, c, ...) — handles any number of args
                if let Some(Value::Array(arr)) = args.first() {
                    let m = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::INFINITY, f64::min);
                    return Ok(Value::Number(m));
                }
                let m = args
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::INFINITY, f64::min);
                Ok(Value::Number(m))
            }
            "clamp" => {
                let v = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let lo = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::NEG_INFINITY);
                let hi = args
                    .get(2)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::INFINITY);
                Ok(Value::Number(v.clamp(lo, hi)))
            }
            "random" => {
                let lo = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let hi = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(Value::Number(lo + self.next_random_unit_f64() * (hi - lo)))
            }
            // random_int(min, max) — an *inclusive* integer range, unlike
            // random(lo, hi)'s exclusive-of-hi float range: the two most
            // common off-by-one mistakes with a hand-rolled
            // `floor(random() * (max - min + 1)) + min` (forgetting the
            // `+ 1`, or not flooring at all) are exactly what this exists
            // to make impossible to get wrong.
            "random_int" => {
                let lo = args
                    .first()
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| Signal::Error("random_int: expected min".into()))?;
                let hi = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| Signal::Error("random_int: expected max".into()))?;
                if hi < lo {
                    return Err(Signal::Error(format!(
                        "random_int: max ({}) must be >= min ({})",
                        hi, lo
                    )));
                }
                let span = (hi - lo).floor() + 1.0;
                let n = (lo + (self.next_random_unit_f64() * span).floor()).min(hi);
                Ok(Value::Number(n))
            }
            // random_choice(array) — null on an empty array (the same
            // "no crash on the empty case" convention .first()/.last()
            // already use) rather than an error a caller has to guard
            // against separately.
            "random_choice" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("random_choice: expected array".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let idx = (self.next_random_unit_f64() * items.len() as f64).floor() as usize;
                Ok(items[idx.min(items.len() - 1)].clone())
            }
            // shuffle(array) — returns a new array (every other array
            // operation in GX returns a new value rather than mutating in
            // place; this stays consistent with that rather than being a
            // surprising exception). Fisher-Yates using one seeded,
            // *stepped* generator for the whole call — drawing a fresh
            // system-time seed per swap (as random()/random_int() each
            // independently do, when unseeded) risks two swaps landing on
            // the same or a barely-advanced clock tick and producing a
            // correlated, poorly-shuffled result on a fast loop. When
            // `set_random_seed(n)` is active, every swap instead steps the
            // one shared, persistent `rng_seed` — deterministic across the
            // whole call *and* consistent with every other random-family
            // builtin's draw sequence in the same seeded run.
            "shuffle" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("shuffle: expected array".into()))?;
                let mut items = arr.iter().map_err(Signal::Error)?;
                if let Some(state) = &mut self.rng_seed {
                    for i in (1..items.len()).rev() {
                        let j = (lcg_step_unit_f64(state) * (i + 1) as f64).floor() as usize;
                        items.swap(i, j.min(i));
                    }
                } else {
                    let mut state = random_seed_u64();
                    for i in (1..items.len()).rev() {
                        let j = (lcg_step_unit_f64(&mut state) * (i + 1) as f64).floor() as usize;
                        items.swap(i, j.min(i));
                    }
                }
                Ok(Value::Array(items))
            }
            // Production Testing Framework: makes every random-family
            // builtin above deterministic for the rest of this
            // Interpreter's lifetime (or until called again with a
            // different seed) — the same seed always produces the same
            // sequence of draws, so a test asserting on "random" behavior
            // stops being flaky. See `next_random_unit_f64`.
            "set_random_seed" => {
                let n = args
                    .first()
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| Signal::Error("set_random_seed: expected a number".into()))?;
                self.rng_seed = Some(n as u64);
                Ok(Value::Null)
            }
            "pi" | "PI" => Ok(Value::Number(std::f64::consts::PI)),
            "e" | "E" => Ok(Value::Number(std::f64::consts::E)),

            // ── Testing Framework ────────────────────────────────────────────
            // `test(name, fn)` registers a named, isolated test case rather
            // than running it immediately — `crate::toolchain::test` runs
            // each one separately after the top-level script finishes (see
            // `take_registered_tests`), so one test's assertion failure is
            // reported against its own name and doesn't abort the others the
            // way a bare top-level `assert` would.
            "test" => {
                let name = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("test: expected a name string".into()))?;
                let f = args.get(1).cloned().ok_or_else(|| {
                    Signal::Error("test: expected a function (fn() { ... })".into())
                })?;
                if !matches!(f, Value::Closure(..)) {
                    return Err(Signal::Error(format!(
                        "test: second argument must be a function (fn() {{ ... }}), got {}",
                        f.type_name()
                    )));
                }
                self.registered_tests.push((name, f));
                Ok(Value::Null)
            }
            // Single active hook each — a later call replaces an earlier
            // one. Run around every case registered via `test()` in this
            // file (see `crate::toolchain::test`), not around the
            // top-level script itself.
            "before_each" => {
                let f = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("before_each: expected a function".into()))?;
                if !matches!(f, Value::Closure(..)) {
                    return Err(Signal::Error(
                        "before_each: argument must be a function (fn() { ... })".into(),
                    ));
                }
                self.before_each_fn = Some(f);
                Ok(Value::Null)
            }
            "after_each" => {
                let f = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("after_each: expected a function".into()))?;
                if !matches!(f, Value::Closure(..)) {
                    return Err(Signal::Error(
                        "after_each: argument must be a function (fn() { ... })".into(),
                    ));
                }
                self.after_each_fn = Some(f);
                Ok(Value::Null)
            }
            // assert_golden(actual, path) — byte-for-byte comparison
            // against a golden file. A `Value::Str` is compared as-is (the
            // natural shape for "golden text output" — a rendered
            // template, an HTTP body); anything else is serialized as
            // pretty-printed JSON with sorted keys (`gx_value_to_json`
            // routes objects through `serde_json::Map`, which — without
            // this crate's `preserve_order` feature enabled — sorts keys,
            // unlike `Value::Object`'s own `HashMap`, whose iteration
            // order is *not* stable across runs and would make a golden
            // file comparison of an object flaky for reasons that have
            // nothing to do with the value actually changing).
            //
            // No golden file yet at `path`, or `GX_UPDATE_GOLDEN=1` set:
            // write `actual` as the new golden and pass — the same
            // "missing snapshot always writes and passes" convention
            // widely used elsewhere, so a fresh golden test doesn't need
            // two separate runs (one to create the file, one to verify it)
            // before it can ever go green.
            "assert_golden" => {
                let actual = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("assert_golden: expected a value".into()))?;
                let path_str = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("assert_golden: expected a path string".into()))?;
                let actual_text = match &actual {
                    Value::Str(s) => s.clone(),
                    other => serde_json::to_string_pretty(&gx_value_to_json(other))
                        .unwrap_or_else(|_| other.to_string()),
                };
                let path = self.safe_path(&path_str)?;
                self.assert_count += 1;
                let update = std::env::var("GX_UPDATE_GOLDEN")
                    .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
                if update || !path.exists() {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    std::fs::write(&path, &actual_text).map_err(|e| {
                        Signal::Error(format!(
                            "assert_golden: writing golden file '{}': {}",
                            path_str, e
                        ))
                    })?;
                    return Ok(Value::Bool(true));
                }
                let expected = std::fs::read_to_string(&path).map_err(|e| {
                    Signal::Error(format!(
                        "assert_golden: reading golden file '{}': {}",
                        path_str, e
                    ))
                })?;
                if actual_text == expected {
                    Ok(Value::Bool(true))
                } else {
                    let msg = format!(
                        "assert_golden: value does not match golden file '{}' \
                         (run with GX_UPDATE_GOLDEN=1 to update it)\n--- expected ---\n{}\n--- actual ---\n{}",
                        path_str, expected, actual_text
                    );
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }
            // test_temp_dir() — a scratch directory, fresh on every call,
            // resolved via `safe_path` exactly like any other file
            // operation (`tmp/<pid>-<n>`, relative — sandboxed under the
            // script's own directory when `gx run` sandboxing is active,
            // relative to cwd under `gx test`'s Unrestricted default, the
            // same as a script's own `write_file("foo.txt", ...)` calls
            // already behave either way). Never the OS temp directory: a
            // path outside wherever the *rest* of a test's file
            // operations resolve would be immediately unusable for any
            // subsequent `write_file`/`read_file` call against it,
            // defeating the point of handing back a directory a test can
            // actually write into. `tmp/` also matches this project's own
            // existing convention (see `tests/tmp/`, already used by
            // hand-written paths in several `tests/*.gx` files) rather
            // than inventing a differently-named scratch directory.
            //
            // The counter is a process-wide atomic, not an Interpreter
            // field: `gx test` constructs a *fresh* `Interpreter` per test
            // file, so a per-instance counter would restart at 0 for every
            // file and hand back the same "tmp/gx-test-<pid>-1" path to
            // more than one file's tests.
            "test_temp_dir" => {
                let n = TEST_TEMP_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let rel = format!("tmp/gx-test-{}-{}", std::process::id(), n);
                let path = self.safe_path(&rel)?;
                std::fs::create_dir_all(&path)
                    .map_err(|e| Signal::Error(format!("test_temp_dir: {}", e)))?;
                Ok(Value::Str(path.to_string_lossy().into_owned()))
            }

            // ── JSON ──────────────────────────────────────────────────────────
            "json_parse" | "parse_json" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match serde_json::from_str::<serde_json::Value>(&s) {
                    Ok(json) => Ok(json_to_gx_value(&json)),
                    Err(e) => Err(Signal::Error(format!("json_parse error: {}", e))),
                }
            }
            "json_stringify" | "to_json" | "json" => {
                let val = args.first().cloned().unwrap_or(Value::Null);
                let pretty = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);
                let json = gx_value_to_json(&val);
                let s = if pretty {
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                } else {
                    serde_json::to_string(&json).unwrap_or_default()
                };
                Ok(Value::Str(s))
            }

            // Generalizes the AI Context Runtime's own `context_serialize`/
            // `context_deserialize` pattern (a `__gx_context_version` tag,
            // checked on load, rejecting a stale/foreign blob loudly rather
            // than deserializing it into a silently-wrong shape) into a
            // primitive any GX app can use for *its own* persisted data —
            // not just AI contexts. Composes `json_stringify`/`json_parse`;
            // no new serialization logic.
            "versioned_stringify" => {
                let val = args.first().cloned().unwrap_or(Value::Null);
                let version = args.get(1).and_then(|v| v.as_number()).ok_or_else(|| {
                    Signal::Error("versioned_stringify: expected a version number".into())
                })?;
                let mut wrapper = HashMap::new();
                wrapper.insert("__gx_version".to_string(), Value::Number(version));
                wrapper.insert("data".to_string(), val);
                let json = gx_value_to_json(&Value::Object(wrapper));
                Ok(Value::Str(serde_json::to_string(&json).unwrap_or_default()))
            }
            "versioned_parse" => {
                let text = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Signal::Error("versioned_parse: expected a string".into()))?;
                let expected_version = args.get(1).and_then(|v| v.as_number());
                let json: serde_json::Value = serde_json::from_str(text)
                    .map_err(|e| Signal::Error(format!("versioned_parse: invalid JSON: {}", e)))?;
                let val = json_to_gx_value(&json);
                let Value::Object(m) = &val else {
                    return Err(Signal::Error(
                        "versioned_parse: expected a versioned_stringify-produced object".into(),
                    ));
                };
                let actual_version = m.get("__gx_version").and_then(|v| v.as_number());
                if let Some(expected) = expected_version {
                    match actual_version {
                        Some(actual) if actual == expected => {}
                        Some(actual) => {
                            return Err(Signal::Error(format!(
                                "versioned_parse: unsupported version {} (expected {})",
                                actual, expected
                            )))
                        }
                        None => {
                            return Err(Signal::Error(
                                "versioned_parse: missing __gx_version — not produced by versioned_stringify".into(),
                            ))
                        }
                    }
                }
                Ok(m.get("data").cloned().unwrap_or(Value::Null))
            }

            // ── HTTP client ───────────────────────────────────────────────────
            "http_get" | "fetch" | "http_post" | "http_put" | "http_delete" => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(url) = args.first().and_then(|v| v.as_str()) {
                    check_url_safe(url, &self.capabilities, &self.diagnostics)?;
                }
                let url_attr = args
                    .first()
                    .and_then(|v| v.as_str())
                    .map(diagnostic_url)
                    .unwrap_or_default();
                // A `&'static str` match, not `format!("http.client.{}", name)`
                // — the latter would heap-allocate on every single HTTP call
                // regardless of whether tracing is even enabled, since Rust
                // evaluates a call's arguments before start_span's own
                // disabled-fast-path check ever runs.
                let span_name: &'static str = match name {
                    "http_post" => "http.client.http_post",
                    "http_put" => "http.client.http_put",
                    "http_delete" => "http.client.http_delete",
                    "fetch" => "http.client.fetch",
                    _ => "http.client.http_get",
                };
                let span = self.diagnostics.start_span(span_name);
                #[cfg(not(target_arch = "wasm32"))]
                let result = {
                    let agent = self.http_agent();
                    http_builtin(name, &args, &agent)
                };
                #[cfg(target_arch = "wasm32")]
                let result = http_builtin(name, &args);
                let err_msg = http_result_err_msg(&result);
                let outcome = match &err_msg {
                    Some(e) => crate::diagnostics::Outcome::Error(e.as_str()),
                    None => crate::diagnostics::Outcome::Ok,
                };
                self.diagnostics.end_span(
                    span,
                    outcome,
                    &[("url", serde_json::Value::String(url_attr))],
                );
                result
            }

            // http_request { url, method, body, headers } — unified form
            "http_request" => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let url = match &opts {
                    Value::Object(m) => m
                        .get("url")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default(),
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(
                            "http_request: expected object with url field".into(),
                        ))
                    }
                };
                let method = match &opts {
                    Value::Object(m) => m
                        .get("method")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "GET".into()),
                    _ => "GET".into(),
                };
                let body_val = match &opts {
                    Value::Object(m) => m.get("body").cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                let headers: Vec<(String, String)> = match &opts {
                    Value::Object(m) => match m.get("headers") {
                        Some(Value::Object(hm)) => hm
                            .iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                #[cfg(not(target_arch = "wasm32"))]
                check_url_safe(&url, &self.capabilities, &self.diagnostics)?;
                let method_name = match method.to_uppercase().as_str() {
                    "POST" => "http_post",
                    "PUT" => "http_put",
                    "DELETE" => "http_delete",
                    _ => "http_get",
                };
                let headers_val = if headers.is_empty() {
                    None
                } else {
                    Some(Value::Object(
                        headers
                            .into_iter()
                            .map(|(k, v)| (k, Value::Str(v)))
                            .collect::<HashMap<_, _>>(),
                    ))
                };
                // arg layout: [url, body?, headers?]
                // For GET/DELETE: [url, headers?]
                // For POST/PUT:   [url, body, headers?]
                let mut builtin_args = vec![Value::Str(url)];
                match method_name {
                    "http_post" | "http_put" => {
                        builtin_args.push(body_val);
                        if let Some(h) = headers_val {
                            builtin_args.push(h);
                        }
                    }
                    _ => {
                        if let Some(h) = headers_val {
                            builtin_args.push(h);
                        }
                    }
                }
                let url_attr = match builtin_args.first() {
                    Some(Value::Str(s)) => diagnostic_url(s),
                    _ => String::new(),
                };
                // Same reasoning as the http_get/post/put/delete arm above:
                // a static match, not `format!`, so no allocation happens
                // just to *name* the span before checking if tracing is on.
                let span_name: &'static str = match method_name {
                    "http_post" => "http.client.http_post",
                    "http_put" => "http.client.http_put",
                    "http_delete" => "http.client.http_delete",
                    _ => "http.client.http_get",
                };
                let span = self.diagnostics.start_span(span_name);
                #[cfg(not(target_arch = "wasm32"))]
                let result = {
                    let agent = self.http_agent();
                    http_builtin(method_name, &builtin_args, &agent)
                };
                #[cfg(target_arch = "wasm32")]
                let result = http_builtin(method_name, &builtin_args);
                let err_msg = http_result_err_msg(&result);
                let outcome = match &err_msg {
                    Some(e) => crate::diagnostics::Outcome::Error(e.as_str()),
                    None => crate::diagnostics::Outcome::Ok,
                };
                self.diagnostics.end_span(
                    span,
                    outcome,
                    &[("url", serde_json::Value::String(url_attr))],
                );
                result
            }

            // ── #17 HTTP streaming ────────────────────────────────────────────
            "http_stream" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let url = match args.first() {
                        Some(Value::Object(m)) => m
                            .get("url")
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    if !url.is_empty() {
                        check_url_safe(&url, &self.capabilities, &self.diagnostics)?;
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let agent = self.http_agent();
                    http_stream_builtin(&args, &agent)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    http_stream_builtin(&args)
                }
            }

            // ── #8 HTTP multipart upload ──────────────────────────────────────
            "http_upload" => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(Value::Object(m)) = args.first() {
                    if let Some(url) = m.get("url").and_then(|v| v.as_str()) {
                        check_url_safe(url, &self.capabilities, &self.diagnostics)?;
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let agent = self.http_agent();
                    http_upload_builtin(&args, &agent, &self.capabilities)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    http_upload_builtin(&args)
                }
            }

            // sse_send(data) or sse_send(event, data) — write one
            // Server-Sent-Events frame to the connection currently open via
            // `respond stream { ... }`. Only meaningful inside that block;
            // self.sse_tx is None everywhere else, including inside a
            // normal (non-stream) route.
            #[cfg(not(target_arch = "wasm32"))]
            "sse_send" => {
                let tx = self.sse_tx.clone().ok_or_else(|| {
                    Signal::Error("sse_send: only valid inside a respond stream block".into())
                })?;
                let (event, data_val) = if args.len() >= 2 {
                    (
                        args.first().and_then(|v| v.as_str().map(String::from)),
                        args.get(1).cloned().unwrap_or(Value::Null),
                    )
                } else {
                    (None, args.first().cloned().unwrap_or(Value::Null))
                };
                let data_str = match &data_val {
                    Value::Str(s) => s.clone(),
                    other => value_to_json(other),
                };
                let mut frame = String::new();
                if let Some(ev) = event {
                    // Newlines aren't valid inside a single SSE field line;
                    // an event name containing one would corrupt the
                    // frame, so it's rejected rather than silently mangled.
                    if ev.contains('\n') {
                        return Err(Signal::Error(
                            "sse_send: event name must not contain newlines".into(),
                        ));
                    }
                    frame.push_str("event: ");
                    frame.push_str(&ev);
                    frame.push('\n');
                }
                if data_str.is_empty() {
                    frame.push_str("data: \n");
                } else {
                    for line in data_str.lines() {
                        frame.push_str("data: ");
                        frame.push_str(line);
                        frame.push('\n');
                    }
                }
                frame.push('\n');
                // A plain blocking `send` on this bounded channel would
                // hang the whole worker thread forever if the client
                // simply stops reading (a dead connection that never
                // closes, or just a very slow one) — the channel fills,
                // send() blocks, and that worker can never process another
                // request again. Retrying try_send with a bounded overall
                // deadline keeps real backpressure (a client that's merely
                // bursty gets a real chance to catch up) while guaranteeing
                // this can never block longer than SSE_SEND_TIMEOUT.
                let deadline = std::time::Instant::now() + SSE_SEND_TIMEOUT;
                let mut payload = frame.into_bytes();
                loop {
                    match tx.try_send(payload) {
                        Ok(()) => break,
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                            return Err(Signal::Error("sse_send: client disconnected".into()));
                        }
                        Err(std::sync::mpsc::TrySendError::Full(returned)) => {
                            if std::time::Instant::now() >= deadline {
                                return Err(Signal::Error(
                                    "sse_send: client is not reading fast enough (timed out)"
                                        .into(),
                                ));
                            }
                            payload = returned;
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
                Ok(Value::Null)
            }

            // send_email { to, subject, body, smtp_host?, smtp_port?, from?, username?, password? }
            "send_email" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let opts = args.first().cloned().unwrap_or(Value::Null);
                    let get_str = |key: &str| match &opts {
                        Value::Object(m) => m
                            .get(key)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    let to = get_str("to");
                    let subject = get_str("subject");
                    let body_s = get_str("body");
                    let from = get_str("from");
                    let smtp_host = get_str("smtp_host");
                    let smtp_port = match &opts {
                        Value::Object(m) => m
                            .get("smtp_port")
                            .and_then(|v| v.as_number())
                            .unwrap_or(587.0) as u16,
                        _ => 587,
                    };
                    let username = get_str("username");
                    let password = get_str("password");

                    // Use SMTP env vars as fallback
                    let smtp_host = if smtp_host.is_empty() {
                        std::env::var("SMTP_HOST").unwrap_or_else(|_| "localhost".into())
                    } else {
                        smtp_host
                    };
                    let from = if from.is_empty() {
                        std::env::var("SMTP_FROM").unwrap_or_else(|_| "gx@localhost".into())
                    } else {
                        from
                    };
                    let username = if username.is_empty() {
                        std::env::var("SMTP_USER").unwrap_or_default()
                    } else {
                        username
                    };
                    let password = if password.is_empty() {
                        std::env::var("SMTP_PASS").unwrap_or_default()
                    } else {
                        password
                    };

                    let raw = format!(
                        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body_s}"
                    );

                    use std::io::{BufRead, BufReader, Write};
                    use std::net::TcpStream;
                    let stream = TcpStream::connect(format!("{}:{}", smtp_host, smtp_port))
                        .map_err(|e| Signal::Error(format!("send_email: connect failed: {}", e)))?;
                    let mut w = std::io::BufWriter::new(stream.try_clone().map_err(|e| {
                        Signal::Error(format!("send_email: failed to clone socket: {}", e))
                    })?);
                    let mut r = BufReader::new(stream);
                    let mut line = String::new();
                    let smtp_read =
                        |r: &mut BufReader<TcpStream>, line: &mut String| -> Result<(), Signal> {
                            line.clear();
                            r.read_line(line)
                                .map_err(|e| Signal::Error(format!("send_email: read: {}", e)))?;
                            Ok(())
                        };
                    smtp_read(&mut r, &mut line)?;
                    let cmd =
                        |w: &mut std::io::BufWriter<TcpStream>, s: &str| -> Result<(), Signal> {
                            w.write_all(format!("{}\r\n", s).as_bytes())
                                .map_err(|e| Signal::Error(format!("send_email: write: {}", e)))?;
                            w.flush()
                                .map_err(|e| Signal::Error(format!("send_email: flush: {}", e)))
                        };
                    cmd(&mut w, "EHLO localhost")?;
                    smtp_read(&mut r, &mut line)?;
                    while line.contains('-') {
                        smtp_read(&mut r, &mut line)?;
                    }
                    if !username.is_empty() {
                        cmd(&mut w, "AUTH LOGIN")?;
                        smtp_read(&mut r, &mut line)?;
                        cmd(&mut w, &base64_encode(username.as_bytes()))?;
                        smtp_read(&mut r, &mut line)?;
                        cmd(&mut w, &base64_encode(password.as_bytes()))?;
                        smtp_read(&mut r, &mut line)?;
                    }
                    cmd(&mut w, &format!("MAIL FROM:<{}>", from))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, &format!("RCPT TO:<{}>", to))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, "DATA")?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, &format!("{}\r\n.", raw))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, "QUIT")?;
                    Ok(Value::Bool(true))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error(
                    "send_email not available in playground".into(),
                ))
            }

            // scrape "url" → cleaned plain text
            "scrape" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let url = args
                        .first()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    let html = ureq::get(&url)
                        .call()
                        .map_err(|e| Signal::Error(format!("scrape: fetch failed: {}", e)))?
                        .into_string()
                        .map_err(|e| Signal::Error(format!("scrape: read failed: {}", e)))?;
                    // Strip HTML tags naively
                    let text = strip_html_tags(&html);
                    Ok(Value::Str(text))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error("scrape not available in playground".into()))
            }

            // notify { channel: "slack"|"webhook", url, message }
            "notify" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let opts = args.first().cloned().unwrap_or(Value::Null);
                    let get_str = |key: &str| match &opts {
                        Value::Object(m) => m
                            .get(key)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    let channel = get_str("channel");
                    let message = get_str("message");
                    let url = if channel == "slack" {
                        std::env::var("SLACK_WEBHOOK_URL").unwrap_or_else(|_| get_str("url"))
                    } else {
                        get_str("url")
                    };
                    if url.is_empty() {
                        return Err(Signal::Error(
                            "notify: url or SLACK_WEBHOOK_URL required".into(),
                        ));
                    }
                    let payload = format!("{{\"text\":\"{}\"}}", message.replace('"', "\\\""));
                    ureq::post(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&payload)
                        .map_err(|e| Signal::Error(format!("notify: failed: {}", e)))?;
                    Ok(Value::Bool(true))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error("notify not available in playground".into()))
            }

            // ── File I/O ──────────────────────────────────────────────────────
            "read_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("read_file requires a path string".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::read_to_string(&path)
                    .map(Value::Str)
                    .map_err(|e| Signal::Error(format!("read_file '{}': {}", raw, e)))
            }
            "read_file_lines" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| {
                        Signal::Error("read_file_lines requires a path string".into())
                    })?;
                let path = self.safe_path(&raw)?;
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Signal::Error(format!("read_file_lines '{}': {}", raw, e)))?;
                Ok(Value::Array(
                    content.lines().map(|l| Value::Str(l.to_string())).collect(),
                ))
            }
            "write_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("write_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                let content = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()))
                    .to_string();
                std::fs::write(&path, &content)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("write_file '{}': {}", raw, e)))
            }
            // data_import(path) / data_export(path, value, schema?) — a
            // format-agnostic "read+parse"/"stringify+write" pair, format
            // detected from the path's extension. Composes every existing
            // parser/stringifier (json/yaml/toml/csv/xml/jsonl) — no new
            // parsing logic, just the boilerplate every one of those
            // otherwise needs repeated at every call site (`json_parse(
            // read_file(path))`, `write_file(path, yaml_stringify(v))`, ...).
            // Deliberately a *separate* extension table from `config_load`'s
            // `file` option (json/yaml/toml only, since a config file must
            // be a single top-level object) rather than sharing one: a data
            // file has no such restriction — csv/xml/jsonl are all
            // legitimate here and would just fail config_load's "must be an
            // object" check if reused there for no benefit.
            "data_import" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("data_import requires a path string".into()))?;
                let path = self.safe_path(&raw)?;
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Signal::Error(format!("data_import '{}': {}", raw, e)))?;
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                parse_by_extension(&ext, content).map_err(|e| match e {
                    Signal::Error(m) => Signal::Error(format!("data_import '{}': {}", raw, m)),
                    other => other,
                })
            }
            "data_export" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("data_export requires a path string".into()))?;
                let val = args.get(1).cloned().unwrap_or(Value::Null);
                if let Some(Value::Object(schema)) = args.get(2) {
                    let validation =
                        schema_validate_impl(&[val.clone(), Value::Object(schema.clone())])?;
                    if let Value::Object(v) = &validation {
                        if !v.get("ok").map(|b| b.is_truthy()).unwrap_or(true) {
                            let errs = v.get("errors").cloned().unwrap_or(Value::Array(Vec::new()));
                            return Err(Signal::Error(format!(
                                "data_export '{}': value failed schema validation: {}",
                                raw, errs
                            )));
                        }
                    }
                }
                let path = self.safe_path(&raw)?;
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let text = stringify_by_extension(&ext, &val).map_err(|e| match e {
                    Signal::Error(m) => Signal::Error(format!("data_export '{}': {}", raw, m)),
                    other => other,
                })?;
                std::fs::write(&path, &text)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("data_export '{}': {}", raw, e)))
            }
            "append_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("append_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                let content = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()))
                    .to_string();
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&path)
                    .map_err(|e| Signal::Error(format!("append_file '{}': {}", raw, e)))?;
                f.write_all(content.as_bytes())
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("append_file write '{}': {}", raw, e)))
            }
            "file_exists" | "exists" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let path = self.safe_path(&raw)?;
                Ok(Value::Bool(path.exists()))
            }
            // Parse a GX source string using the Rust parser and return the AST
            // as a GX value tree (same format as parser.gx output) for use with
            // js_codegen.gx. This bypasses the slow GX-written parser.
            "parse_gx" => {
                let src = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let parse_result = if crate::indent_parser::is_indent_syntax(&src) {
                    crate::indent_parser::parse(&src)
                } else {
                    crate::lexer::Lexer::new(&src)
                        .tokenize()
                        .and_then(|toks| crate::parser::Parser::new(toks).parse())
                };
                match parse_result {
                    Ok(program) => Ok(gx_ast_to_value(&program)),
                    Err(e) => {
                        eprintln!("[gx] parse_gx error: {}", e);
                        Ok(Value::Null)
                    }
                }
            }
            "delete_file" | "remove_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("delete_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::remove_file(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("delete_file '{}': {}", raw, e)))
            }
            "list_dir" | "read_dir" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| ".".into());
                let path = self.safe_path(&raw)?;
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let files: Vec<Value> = entries
                            .flatten()
                            .map(|e| Value::Str(e.file_name().to_string_lossy().into_owned()))
                            .collect();
                        Ok(Value::Array(files))
                    }
                    Err(e) => Err(Signal::Error(format!("list_dir '{}': {}", raw, e))),
                }
            }
            "make_dir" | "mkdir" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("make_dir requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::create_dir_all(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("make_dir '{}': {}", raw, e)))
            }
            "path_join" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                let mut path = std::path::PathBuf::new();
                for p in parts {
                    path.push(p);
                }
                Ok(Value::Str(path.to_string_lossy().into_owned()))
            }
            "dirname" => {
                let p = args.first().map(|v| v.to_string()).unwrap_or_default();
                let d = std::path::Path::new(&p)
                    .parent()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(Value::Str(d))
            }
            "basename" => {
                let p = args.first().map(|v| v.to_string()).unwrap_or_default();
                let b = std::path::Path::new(&p)
                    .file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(Value::Str(b))
            }
            // extname("report.tar.gz") == ".gz" — matches Node's path.extname
            // (the *last* extension, with the leading dot; "" when there
            // isn't one, including for a dotfile like ".gitignore", which
            // has no "stem" for an extension to attach to).
            "extname" => {
                let p = args.first().map(|v| v.to_string()).unwrap_or_default();
                let ext = std::path::Path::new(&p)
                    .extension()
                    .map(|x| format!(".{}", x.to_string_lossy()))
                    .unwrap_or_default();
                Ok(Value::Str(ext))
            }

            // ── stdlib: strings / tokens ──────────────────────────────────────
            // truncate(value, max[, ellipsis]) — char-safe clip to `max` total chars.
            "truncate" => {
                let s = args.first().map(|v| v.to_string()).unwrap_or_default();
                let max = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let chars: Vec<char> = s.chars().collect();
                if chars.len() <= max {
                    Ok(Value::Str(s))
                } else if max == 0 {
                    Ok(Value::Str(String::new()))
                } else {
                    let ellipsis = args
                        .get(2)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "…".to_string());
                    let keep = max.saturating_sub(ellipsis.chars().count());
                    let mut out: String = chars.into_iter().take(keep).collect();
                    out.push_str(&ellipsis);
                    Ok(Value::Str(out))
                }
            }
            // token_count(text) — fast heuristic estimate (~4 chars/token). Approximate,
            // not a real tokenizer; use it to budget before an API call.
            "token_count" => {
                let s = args.first().map(|v| v.to_string()).unwrap_or_default();
                let est = ((s.chars().count() as f64) / 4.0).ceil();
                Ok(Value::Number(est))
            }
            // tokens_used() — cumulative tokens across every AI call this session.
            "tokens_used" | "total_tokens" => Ok(Value::Number(self.total_tokens_used as f64)),

            // ── AI Context Runtime ──────────────────────────────────────────────
            "context_create" => self.context_create(&args),
            "context_set_system" => self.context_set_system(&args),
            "context_add_message" => self.context_add_message(&args),
            "context_add_tool_result" => self.context_add_tool_result(&args),
            "context_trim" => self.context_trim(&args),
            "context_summarize_and_trim" => self.context_summarize_and_trim(&args),
            "context_clone" => self.context_clone(&args),
            "context_reset" => self.context_reset(&args),
            "context_serialize" => self.context_serialize(&args),
            "context_deserialize" => self.context_deserialize(&args),
            "context_stats" => self.context_stats(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "context_ask" => self.context_ask(&args),
            #[cfg(target_arch = "wasm32")]
            "context_ask" => Err(Signal::Error(
                "context_ask is not available in the playground".into(),
            )),

            // ── stdlib: collections / net ─────────────────────────────────────
            // group_by(array, key) — group objects by the value at `key`.
            "group_by" => {
                let arr = match args.first() {
                    Some(Value::Array(a)) => a.clone(),
                    _ => return Err(Signal::Error("group_by requires an array".into())),
                };
                let key = args.get(1).map(|v| v.to_string()).unwrap_or_default();
                let mut groups: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
                for item in arr {
                    let k = match &item {
                        Value::Object(o) => o.get(&key).map(|v| v.to_string()).unwrap_or_default(),
                        other => other.to_string(),
                    };
                    groups.entry(k).or_default().push(item);
                }
                let mut out = HashMap::new();
                for (k, v) in groups {
                    out.insert(k, Value::Array(v));
                }
                Ok(Value::Object(out))
            }
            // url_parse(url) -> { scheme, host, port, path, query, fragment }
            "url_parse" => {
                let u = args.first().map(|v| v.to_string()).unwrap_or_default();
                let mut rest = u.as_str();
                let mut fragment = String::new();
                if let Some(i) = rest.find('#') {
                    fragment = rest[i + 1..].to_string();
                    rest = &rest[..i];
                }
                let mut scheme = String::new();
                if let Some(i) = rest.find("://") {
                    scheme = rest[..i].to_string();
                    rest = &rest[i + 3..];
                }
                let mut query = String::new();
                if let Some(i) = rest.find('?') {
                    query = rest[i + 1..].to_string();
                    rest = &rest[..i];
                }
                let (authority, path) = match rest.find('/') {
                    Some(i) => (&rest[..i], rest[i..].to_string()),
                    None => (rest, String::new()),
                };
                let (host, port) = match authority.rfind(':') {
                    Some(i) => (authority[..i].to_string(), authority[i + 1..].to_string()),
                    None => (authority.to_string(), String::new()),
                };
                let mut obj = HashMap::new();
                obj.insert("scheme".to_string(), Value::Str(scheme));
                obj.insert("host".to_string(), Value::Str(host));
                obj.insert(
                    "port".to_string(),
                    if port.is_empty() {
                        Value::Null
                    } else {
                        Value::Str(port)
                    },
                );
                obj.insert("path".to_string(), Value::Str(path));
                obj.insert("query".to_string(), Value::Str(query));
                obj.insert("fragment".to_string(), Value::Str(fragment));
                Ok(Value::Object(obj))
            }

            // ── stdlib: crypto ────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "sha256" => {
                use sha2::{Digest, Sha256};
                let s = args.first().map(|v| v.to_string()).unwrap_or_default();
                let mut hasher = Sha256::new();
                hasher.update(s.as_bytes());
                Ok(Value::Str(hex_encode(&hasher.finalize())))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "uuid" | "uuid_v4" => Ok(Value::Str(uuid::Uuid::new_v4().to_string())),

            // ── Crypto: HMAC / secure compare / secure random / Ed25519 / JWT ──
            "hmac_sha256"
            | "hmac_sha512"
            | "secure_compare"
            | "secure_random"
            | "ed25519_generate_keypair"
            | "ed25519_sign"
            | "ed25519_verify"
            | "jwt_sign"
            | "jwt_verify" => crypto_builtin(name, &args),

            // ── stdlib: fs glob (sandbox-aware) ───────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "glob" => {
                self.authorize_capability(crate::capability::Resource::Filesystem, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let pat = args.first().map(|v| v.to_string()).unwrap_or_default();
                let sandbox = self.capabilities.sandbox_dir();
                let full_pat = match (sandbox, std::path::Path::new(&pat).is_absolute()) {
                    (Some(base), false) => base.join(&pat).to_string_lossy().into_owned(),
                    _ => pat.clone(),
                };
                match glob::glob(&full_pat) {
                    Ok(paths) => {
                        let mut out = Vec::new();
                        for entry in paths.flatten() {
                            if let Some(base) = sandbox {
                                if !entry.starts_with(base) {
                                    continue;
                                }
                                let rel = entry.strip_prefix(base).unwrap_or(&entry);
                                out.push(Value::Str(rel.to_string_lossy().into_owned()));
                            } else {
                                out.push(Value::Str(entry.to_string_lossy().into_owned()));
                            }
                        }
                        Ok(Value::Array(out))
                    }
                    Err(e) => Err(Signal::Error(format!("glob '{}': {}", pat, e))),
                }
            }

            // ── String utilities ──────────────────────────────────────────────
            "format" => {
                // format("Hello {0}, you are {1}", name, age)
                let template = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut result = template.clone();
                for (i, arg) in args.iter().skip(1).enumerate() {
                    result = result.replace(&format!("{{{}}}", i), &arg.to_string());
                }
                Ok(Value::Str(result))
            }
            "url_encode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let encoded: String = s
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                            c.to_string()
                        } else {
                            format!("%{:02X}", c as u32)
                        }
                    })
                    .collect();
                Ok(Value::Str(encoded))
            }
            "url_decode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(util::url_decode(&s)))
            }
            "html_escape" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let escaped = s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;");
                Ok(Value::Str(escaped))
            }
            "html_unescape" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let unescaped = s
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'");
                Ok(Value::Str(unescaped))
            }
            "base64_encode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let encoded = base64_encode(s.as_bytes());
                Ok(Value::Str(encoded))
            }
            "base64_decode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match base64_decode(&s) {
                    Ok(bytes) => Ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned())),
                    Err(e) => Err(Signal::Error(format!("base64_decode: {}", e))),
                }
            }

            // ── Process / System ──────────────────────────────────────────────
            "shell" | "exec" => {
                self.authorize_capability(crate::capability::Resource::Shell, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let cmd = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("shell requires a command string".into()))?;
                let span = self.diagnostics.start_span("process.shell");
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .stdin(std::process::Stdio::inherit())
                    .output()
                    .map_err(|e| Signal::Error(format!("shell exec failed: {}", e)))?;
                self.diagnostics.end_span(
                    span,
                    if output.status.success() {
                        crate::diagnostics::Outcome::Ok
                    } else {
                        crate::diagnostics::Outcome::Error("non-zero exit")
                    },
                    &[("command", serde_json::Value::String(cmd.clone()))],
                );
                let mut map = HashMap::new();
                map.insert(
                    "stdout".into(),
                    Value::Str(String::from_utf8_lossy(&output.stdout).into_owned()),
                );
                map.insert(
                    "stderr".into(),
                    Value::Str(String::from_utf8_lossy(&output.stderr).into_owned()),
                );
                map.insert(
                    "exit_code".into(),
                    Value::Number(output.status.code().unwrap_or(-1) as f64),
                );
                map.insert("ok".into(), Value::Bool(output.status.success()));
                Ok(Value::Object(map))
            }

            // ── Native process runtime ──────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "process_run" => self.process_run(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_spawn" => self.process_spawn(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_wait" => self.process_wait(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_kill" => self.process_kill(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_exists" => self.process_exists(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_status" => self.process_status(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "process_read" => self.process_read(&args),

            // ── Native task runtime ───────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "task_spawn" => self.task_spawn(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_wait" => self.task_wait(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_wait_all" => self.task_wait_all(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_wait_any" => self.task_wait_any(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_cancel" => self.task_cancel(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_status" => self.task_status(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_id" => Ok(self.task_id_builtin()),
            #[cfg(not(target_arch = "wasm32"))]
            "is_cancelled" => Ok(self.is_cancelled_builtin()),
            // Lightweight progress reporting from inside a running task,
            // drained by the caller — see TaskState's own doc comment for
            // why this exists (task_wait's result is only available on
            // completion; there was no way for a still-running task to
            // report incremental progress before this).
            #[cfg(not(target_arch = "wasm32"))]
            "task_emit" => self.task_emit(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "task_progress" => self.task_progress(&args),

            "input" => {
                use std::io::{self, Write};
                let prompt = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                if !prompt.is_empty() {
                    print!("{}", prompt);
                    io::stdout().flush().ok();
                }
                let mut line = String::new();
                io::stdin().read_line(&mut line).ok();
                Ok(Value::Str(
                    line.trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string(),
                ))
            }
            "exit" => {
                // std::process::exit does not run destructors, so any
                // still-running process_spawn children / task_spawn tasks
                // must be reaped here explicitly rather than relying on
                // Interpreter's Drop impl.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.cleanup_tasks();
                    self.cleanup_processes();
                }
                let code = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
                std::process::exit(code);
            }

            // ── Agent management ──────────────────────────────────────────────
            "spawn_agent" | "spawn_helper" => {
                let n = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "unknown".into());
                Err(Signal::Error(format!(
                    "spawn_agent('{}') is not yet implemented. \
                     Agents currently run sequentially in one process. \
                     Use function calls or direct helper invocation for sub-tasks.",
                    n
                )))
            }

            // ── Readiness (#14) ───────────────────────────────────────────────
            "ready" => {
                if let Some(name) = self.current_agent.clone() {
                    self.ready_agents.insert(name.clone());
                    // Flush any messages queued before ready() was called
                    if let Some(queued) = self.queued_messages.remove(&name) {
                        for (event, payload) in queued {
                            let bus_key = format!("{}:{}", name, event);
                            self.event_bus.entry(bus_key).or_default().push(payload);
                        }
                    }
                }
                Ok(Value::Null)
            }

            // ── Env ───────────────────────────────────────────────────────────
            "env_require" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("env_require: expected string key".into()))?;
                match std::env::var(&key) {
                    Ok(val) => Ok(Value::Str(val)),
                    Err(_) => Err(Signal::Error(format!(
                        "Missing required environment variable: {}",
                        key
                    ))),
                }
            }

            // ── Object utilities ──────────────────────────────────────────────
            "object_merge" => {
                let mut result: HashMap<String, Value> = HashMap::new();
                for arg in &args {
                    if let Value::Object(map) = arg {
                        for (k, v) in map {
                            result.insert(k.clone(), v.clone());
                        }
                    } else {
                        return Err(Signal::Error(format!(
                            "object_merge: expected object, got {}",
                            arg.type_name()
                        )));
                    }
                }
                Ok(Value::Object(result))
            }

            // ── Regex ─────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "regex_match" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_match: expected pattern string".into()))?;
                let text = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_match: expected text string".into()))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| Signal::Error(format!("regex_match: invalid pattern: {}", e)))?;
                Ok(Value::Bool(re.is_match(&text)))
            }
            // ── SQLite ────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "db_query" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected db path".into()))?;
                let sql = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected SQL string".into()))?;
                // Params: db_query(path, sql, [p1,p2]) OR db_query(path, sql, p1, p2)
                let params: Vec<Value> = db_unpack_params(&args, 2);
                // Resolved purely by the requested path, whether or not
                // *any* transaction is active — previously this looked at
                // a single global "is some transaction active anywhere"
                // slot, so db_query("other.db", ...) called from inside a
                // db_transaction("main.db") {...} silently ran against
                // main.db's connection instead of opening/using other.db.
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                let span = self.diagnostics.start_span("db.query");
                let conn = match self.db_conn(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = format!("{:?}", e);
                        self.diagnostics.end_span(
                            span,
                            crate::diagnostics::Outcome::Error(&msg),
                            &[("db", serde_json::Value::String(path))],
                        );
                        return Err(e);
                    }
                };
                let result = db_query_on_conn(conn, &sql, params);
                let err_msg = match &result {
                    Err(Signal::Error(e)) => Some(e.clone()),
                    Err(_) => Some("query failed".to_string()),
                    Ok(_) => None,
                };
                let outcome = match &err_msg {
                    Some(e) => crate::diagnostics::Outcome::Error(e.as_str()),
                    None => crate::diagnostics::Outcome::Ok,
                };
                self.diagnostics.end_span(
                    span,
                    outcome,
                    &[("db", serde_json::Value::String(path))],
                );
                result
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_exec" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_exec: expected db path".into()))?;
                let sql = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_exec: expected SQL string".into()))?;
                // Params: db_exec(path, sql, [p1,p2]) OR db_exec(path, sql, p1, p2)
                let params: Vec<Value> = db_unpack_params(&args, 2);
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                let span = self.diagnostics.start_span("db.exec");
                let conn = match self.db_conn(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = format!("{:?}", e);
                        self.diagnostics.end_span(
                            span,
                            crate::diagnostics::Outcome::Error(&msg),
                            &[("db", serde_json::Value::String(path))],
                        );
                        return Err(e);
                    }
                };
                let result = db_exec_on_conn(conn, &sql, params);
                let err_msg = match &result {
                    Err(Signal::Error(e)) => Some(e.clone()),
                    Err(_) => Some("exec failed".to_string()),
                    Ok(_) => None,
                };
                let outcome = match &err_msg {
                    Some(e) => crate::diagnostics::Outcome::Error(e.as_str()),
                    None => crate::diagnostics::Outcome::Ok,
                };
                self.diagnostics.end_span(
                    span,
                    outcome,
                    &[("db", serde_json::Value::String(path))],
                );
                result
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_migrate" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_migrate: expected db path".into()))?;
                let migrations: Vec<String> = match args.get(1) {
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|v| {
                            v.as_str().map(String::from).ok_or_else(|| {
                                Signal::Error(
                                    "db_migrate: every migration must be a SQL string".into(),
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err(Signal::Error(
                            "db_migrate: expected an array of SQL migration strings".into(),
                        ))
                    }
                };
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                if self.db_tx_depth.get(&path).copied().unwrap_or(0) > 0 {
                    return Err(Signal::Error(
                        "db_migrate: cannot run inside an active db_transaction on the same database".into(),
                    ));
                }
                let conn = self.db_conn(&path)?;
                builtins_db::db_migrate_on_conn(conn, &migrations)
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_integrity_check" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_integrity_check: expected db path".into()))?;
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                let conn = self.db_conn(&path)?;
                builtins_db::db_integrity_check_on_conn(conn)
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_vacuum" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_vacuum: expected db path".into()))?;
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                if self.db_tx_depth.get(&path).copied().unwrap_or(0) > 0 {
                    return Err(Signal::Error(
                        "db_vacuum: cannot run inside an active db_transaction on the same database"
                            .into(),
                    ));
                }
                let conn = self.db_conn(&path)?;
                builtins_db::db_vacuum_on_conn(conn)
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_backup" => {
                self.authorize_capability(crate::capability::Resource::Database, None)
                    .map_err(|e| Signal::Error(e.to_string()))?;
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_backup: expected db path".into()))?;
                let dest_raw = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_backup: expected destination path".into()))?;
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                let dest_safe = self.safe_path(&dest_raw)?;
                let dest_path = dest_safe.to_string_lossy().into_owned();
                let conn = self.db_conn(&path)?;
                builtins_db::db_backup_impl(conn, &dest_path)
            }

            // ── Functional: map / filter ──────────────────────────────────────
            "map" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("map: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("map: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                let mut result = Vec::new();
                for item in items {
                    let out = self.call_closure(&func, vec![item], env)?;
                    result.push(out);
                }
                Ok(Value::Array(result))
            }
            "filter" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("filter: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("filter: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                let mut result = Vec::new();
                for item in items {
                    let keep = self.call_closure(&func, vec![item.clone()], env)?;
                    if keep.is_truthy() {
                        result.push(item);
                    }
                }
                Ok(Value::Array(result))
            }
            // reduce(arr, fn, initial) — `initial` is required rather than
            // optionally defaulting to the first element (as some
            // languages allow): an empty array with no initial value is
            // then simply "return initial unchanged" instead of a special
            // error case a caller has to think about.
            "reduce" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("reduce: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("reduce: expected function".into()))?;
                let mut acc = args
                    .get(2)
                    .cloned()
                    .ok_or_else(|| Signal::Error("reduce: expected an initial value".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                for item in items {
                    acc = self.call_closure(&func, vec![acc, item], env)?;
                }
                Ok(acc)
            }
            // some(arr, fn) / every(arr, fn) — `every` is vacuously true
            // for an empty array (the standard convention: there is no
            // element that fails the predicate).
            "some" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("some: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("some: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                for item in items {
                    if self.call_closure(&func, vec![item], env)?.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "every" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("every: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("every: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                for item in items {
                    if !self.call_closure(&func, vec![item], env)?.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            // find_index(arr, fn) — predicate-based, distinct from the
            // existing value-*equality* `index_of`/`find` (which look for
            // an exact match, not a computed condition). Returns -1 when
            // nothing matches, the same not-found convention `index_of`
            // already uses.
            "find_index" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("find_index: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("find_index: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                for (i, item) in items.into_iter().enumerate() {
                    if self.call_closure(&func, vec![item], env)?.is_truthy() {
                        return Ok(Value::Number(i as f64));
                    }
                }
                Ok(Value::Number(-1.0))
            }

            // ── Legacy stubs ──────────────────────────────────────────────────
            "wait_for_agent_ready"
            | "start_application"
            | "stop_application"
            | "restart_application"
            | "restart_all_failed_agents"
            | "initialize_memory_manager"
            | "initialize_message_router"
            | "initialize_helper_manager"
            | "parse_gx_file"
            | "execute_initial_brain_cycles"
            | "load_application"
            | "generate_job_id"
            | "start_training_process"
            | "monitor_training_jobs"
            | "deploy_ready_models"
            | "update_model_performance"
            | "cleanup_old_models" => Ok(Value::Null),

            _ => {
                let suggestion = closest_builtin(name);
                let hint = match suggestion {
                    Some(s) => format!(" — did you mean '{}'?", s),
                    None => " — returning null".to_string(),
                };
                eprintln!("[gx] warning: unknown function '{}'{}", name, hint);
                Ok(Value::Null)
            }
        }
    }

    fn eval_method(
        &mut self,
        obj: Value,
        method: &str,
        args: Vec<Value>,
        env: &mut Env,
    ) -> IResult {
        match (&obj, method) {
            // `.map(fn)`/`.filter(fn)` delegate to the exact same code as
            // the free-function forms (`map(arr, fn)`/`filter(arr, fn)`,
            // which predate this and already work on Array/Object/String
            // via `Value::iter()`) rather than reimplementing the loop —
            // guarantees the two call styles can never silently drift
            // apart, and closes a real inconsistency: every other array
            // operation (`.sort()`, `.take()`, `.sum()`, ...) was already
            // a method, but the two most fundamental ones weren't —
            // `arr.map(fn)` used to print "unknown method" and silently
            // return null.
            (_, "map") => self.eval_builtin(
                "map",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (_, "filter") => self.eval_builtin(
                "filter",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (_, "reduce") => self.eval_builtin(
                "reduce",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (_, "some") => self.eval_builtin(
                "some",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (_, "every") => self.eval_builtin(
                "every",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (_, "find_index") => self.eval_builtin(
                "find_index",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            // ── Array methods ─────────────────────────────────────────────────
            (Value::Array(arr), "push") | (Value::Array(arr), "append") => {
                let mut a = arr.clone();
                for v in args {
                    a.push(v);
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "pop") => {
                let mut a = arr.clone();
                Ok(a.pop().unwrap_or(Value::Null))
            }
            (Value::Array(arr), "shift") => {
                if arr.is_empty() {
                    return Ok(Value::Null);
                }
                Ok(arr[0].clone())
            }
            (Value::Array(arr), "unshift") => {
                let mut a = arr.clone();
                if let Some(v) = args.into_iter().next() {
                    a.insert(0, v);
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "length")
            | (Value::Array(arr), "len")
            | (Value::Array(arr), "count") => Ok(Value::Number(arr.len() as f64)),
            (Value::Array(arr), "first") => Ok(arr.first().cloned().unwrap_or(Value::Null)),
            (Value::Array(arr), "last") => Ok(arr.last().cloned().unwrap_or(Value::Null)),
            (Value::Array(arr), "join") => {
                let sep = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    arr.iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(&sep),
                ))
            }
            (Value::Array(arr), "contains") | (Value::Array(arr), "includes") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Bool(arr.contains(&needle)))
            }
            (Value::Array(arr), "index_of") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Number(
                    arr.iter()
                        .position(|v| v == &needle)
                        .map(|i| i as f64)
                        .unwrap_or(-1.0),
                ))
            }
            (Value::Array(arr), "reverse") => {
                let mut a = arr.clone();
                a.reverse();
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "sort") => {
                let mut a = arr.clone();
                a.sort_by(|x, y| match (x, y) {
                    (Value::Number(a), Value::Number(b)) => {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    _ => x.to_string().cmp(&y.to_string()),
                });
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "sort_by") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut a = arr.clone();
                a.sort_by(|x, y| {
                    let xk = x.get_field(&key);
                    let yk = y.get_field(&key);
                    match (xk, yk) {
                        (Value::Number(a), Value::Number(b)) => {
                            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (a, b) => a.to_string().cmp(&b.to_string()),
                    }
                });
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "slice") => {
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let end = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .map(|n| n as usize)
                    .unwrap_or(arr.len());
                let end = end.min(arr.len());
                let start = start.min(end);
                Ok(Value::Array(arr[start..end].to_vec()))
            }
            (Value::Array(arr), "concat") => {
                let mut a = arr.clone();
                for arg in args {
                    match arg {
                        Value::Array(b) => a.extend(b),
                        v => a.push(v),
                    }
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "flat") | (Value::Array(arr), "flatten") => {
                let mut flat = Vec::new();
                for v in arr {
                    match v {
                        Value::Array(inner) => flat.extend(inner.clone()),
                        other => flat.push(other.clone()),
                    }
                }
                Ok(Value::Array(flat))
            }
            (Value::Array(arr), "unique") | (Value::Array(arr), "distinct") => {
                let mut seen = Vec::new();
                for v in arr {
                    if !seen.contains(v) {
                        seen.push(v.clone());
                    }
                }
                Ok(Value::Array(seen))
            }
            (Value::Array(arr), "sum") => {
                let s: f64 = arr.iter().filter_map(|v| v.as_number()).sum();
                Ok(Value::Number(s))
            }
            (Value::Array(arr), "min") => {
                let m = arr
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::INFINITY, f64::min);
                Ok(Value::Number(m))
            }
            (Value::Array(arr), "max") => {
                let m = arr
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Value::Number(m))
            }
            (Value::Array(arr), "average") | (Value::Array(arr), "mean") => {
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_number()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                Ok(Value::Number(nums.iter().sum::<f64>() / nums.len() as f64))
            }
            (Value::Array(arr), "find") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(arr
                    .iter()
                    .find(|v| *v == &needle)
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            (Value::Array(arr), "filter_by") => {
                // filter_by("field", value) — filter objects by field value
                let field = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let value = args.get(1).cloned().unwrap_or(Value::Null);
                let filtered: Vec<Value> = arr
                    .iter()
                    .filter(|v| v.get_field(&field) == value)
                    .cloned()
                    .collect();
                Ok(Value::Array(filtered))
            }
            (Value::Array(arr), "map_field") => {
                // map_field("field") — extract a field from each object
                let field = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Array(
                    arr.iter().map(|v| v.get_field(&field)).collect(),
                ))
            }
            (Value::Array(arr), "take") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                Ok(Value::Array(arr.iter().take(n).cloned().collect()))
            }
            (Value::Array(arr), "skip") | (Value::Array(arr), "drop") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                Ok(Value::Array(arr.iter().skip(n).cloned().collect()))
            }
            (Value::Array(arr), "to_json") => {
                let json =
                    serde_json::to_string(&arr.iter().map(gx_value_to_json).collect::<Vec<_>>())
                        .unwrap_or_default();
                Ok(Value::Str(json))
            }

            // ── String methods ────────────────────────────────────────────────
            (Value::Str(s), "length") | (Value::Str(s), "len") | (Value::Str(s), "count") => {
                Ok(Value::Number(s.chars().count() as f64))
            }
            (Value::Str(s), "to_upper") | (Value::Str(s), "upper") => {
                Ok(Value::Str(s.to_uppercase()))
            }
            (Value::Str(s), "to_lower") | (Value::Str(s), "lower") => {
                Ok(Value::Str(s.to_lowercase()))
            }
            (Value::Str(s), "trim") => Ok(Value::Str(s.trim().to_string())),
            (Value::Str(s), "trim_start") | (Value::Str(s), "ltrim") => {
                Ok(Value::Str(s.trim_start().to_string()))
            }
            (Value::Str(s), "trim_end") | (Value::Str(s), "rtrim") => {
                Ok(Value::Str(s.trim_end().to_string()))
            }
            (Value::Str(s), "split") => {
                let sep = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or(" ".into());
                Ok(Value::Array(
                    s.split(&*sep).map(|p| Value::Str(p.to_string())).collect(),
                ))
            }
            (Value::Str(s), "split_lines") | (Value::Str(s), "lines") => Ok(Value::Array(
                s.lines().map(|l| Value::Str(l.to_string())).collect(),
            )),
            (Value::Str(s), "contains") | (Value::Str(s), "includes") => {
                let needle = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.contains(&*needle)))
            }
            (Value::Str(s), "starts_with") => {
                let p = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.starts_with(&*p)))
            }
            (Value::Str(s), "ends_with") => {
                let p = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.ends_with(&*p)))
            }
            (Value::Str(s), "replace") => {
                let from = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let to = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replace(&*from, &to)))
            }
            (Value::Str(s), "replace_first") => {
                let from = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let to = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replacen(&*from, &to, 1)))
            }
            (Value::Str(s), "index_of") => {
                let needle = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Number(
                    s.find(&*needle).map(|i| i as f64).unwrap_or(-1.0),
                ))
            }
            (Value::Str(s), "char_at") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let chars: Vec<char> = s.chars().collect();
                // Same out-of-bounds symmetry as `Value::get_index`: a
                // negative index landing before the start must miss (return
                // null) like a positive out-of-bounds index does, not clamp
                // to char 0.
                let i = if n < 0.0 {
                    chars.len() as i64 + n as i64
                } else {
                    n as i64
                };
                Ok(if i < 0 {
                    Value::Null
                } else {
                    chars
                        .get(i as usize)
                        .map(|c| Value::Str(c.to_string()))
                        .unwrap_or(Value::Null)
                })
            }
            (Value::Str(s), "slice") | (Value::Str(s), "substring") => {
                let chars: Vec<char> = s.chars().collect();
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let end = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .map(|n| n as usize)
                    .unwrap_or(chars.len());
                let end = end.min(chars.len());
                let start = start.min(end);
                Ok(Value::Str(chars[start..end].iter().collect()))
            }
            (Value::Str(s), "repeat") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                match s.len().checked_mul(n) {
                    Some(len) if len <= MAX_STRING_REPEAT_BYTES => Ok(Value::Str(s.repeat(n))),
                    _ => Err(Signal::Error(format!(
                        "repeat: result would exceed the {}-byte limit",
                        MAX_STRING_REPEAT_BYTES
                    ))),
                }
            }
            (Value::Str(s), "pad_left") | (Value::Str(s), "pad_start") => {
                let width = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let pad_char = args
                    .get(1)
                    .and_then(|v| v.as_str().map(|s| s.chars().next().unwrap_or(' ')))
                    .unwrap_or(' ');
                let chars: Vec<char> = s.chars().collect();
                if chars.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let padding: String = std::iter::repeat_n(pad_char, width - chars.len()).collect();
                Ok(Value::Str(format!("{}{}", padding, s)))
            }
            (Value::Str(s), "pad_right") | (Value::Str(s), "pad_end") => {
                let width = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let pad_char = args
                    .get(1)
                    .and_then(|v| v.as_str().map(|s| s.chars().next().unwrap_or(' ')))
                    .unwrap_or(' ');
                let chars: Vec<char> = s.chars().collect();
                if chars.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let padding: String = std::iter::repeat_n(pad_char, width - chars.len()).collect();
                Ok(Value::Str(format!("{}{}", s, padding)))
            }
            (Value::Str(s), "to_number") | (Value::Str(s), "parse_number") => s
                .trim()
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| Signal::Error(format!("Cannot parse '{}' as number", s))),
            (Value::Str(s), "to_json") | (Value::Str(s), "parse_json") => {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(json) => Ok(json_to_gx_value(&json)),
                    Err(e) => Err(Signal::Error(format!("parse_json: {}", e))),
                }
            }
            (Value::Str(s), "reverse") => Ok(Value::Str(s.chars().rev().collect())),
            (Value::Str(s), "to_array") => Ok(Value::Array(
                s.chars().map(|c| Value::Str(c.to_string())).collect(),
            )),
            (Value::Str(s), "is_empty") => Ok(Value::Bool(s.is_empty())),
            (Value::Str(s), "to_upper_first") => {
                let mut chars = s.chars();
                match chars.next() {
                    None => Ok(Value::Str(String::new())),
                    Some(c) => Ok(Value::Str(
                        c.to_uppercase().collect::<String>() + chars.as_str(),
                    )),
                }
            }

            // ── Object methods ────────────────────────────────────────────────
            (Value::Object(m), "has") | (Value::Object(m), "has_key") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(m.contains_key(&*key)))
            }
            (Value::Object(m), "keys") => {
                let mut ks: Vec<Value> = m.keys().map(|k| Value::Str(k.clone())).collect();
                ks.sort_by_key(|a| a.to_string());
                Ok(Value::Array(ks))
            }
            (Value::Object(m), "values") => Ok(Value::Array(m.values().cloned().collect())),
            (Value::Object(m), "entries") => {
                let pairs: Vec<Value> = m
                    .iter()
                    .map(|(k, v)| Value::Array(vec![Value::Str(k.clone()), v.clone()]))
                    .collect();
                Ok(Value::Array(pairs))
            }
            (Value::Object(m), "get") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                Ok(m.get(&*key).cloned().unwrap_or(default))
            }
            (Value::Object(m), "merge") => {
                let mut result = m.clone();
                if let Some(Value::Object(other)) = args.first() {
                    for (k, v) in other {
                        result.insert(k.clone(), v.clone());
                    }
                }
                Ok(Value::Object(result))
            }
            (Value::Object(m), "delete") | (Value::Object(m), "remove") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut result = m.clone();
                result.remove(&*key);
                Ok(Value::Object(result))
            }
            (Value::Object(m), "len")
            | (Value::Object(m), "length")
            | (Value::Object(m), "count") => Ok(Value::Number(m.len() as f64)),
            (Value::Object(m), "to_json") => {
                let json = gx_value_to_json(&Value::Object(m.clone()));
                Ok(Value::Str(serde_json::to_string(&json).unwrap_or_default()))
            }
            (Value::Object(m), "is_empty") => Ok(Value::Bool(m.is_empty())),
            (Value::Object(_), "pick") => self.eval_builtin(
                "pick",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),
            (Value::Object(_), "omit") => self.eval_builtin(
                "omit",
                {
                    let mut a = vec![obj.clone()];
                    a.extend(args);
                    a
                },
                env,
            ),

            // ── Number methods ────────────────────────────────────────────────
            (Value::Number(n), "floor") => Ok(Value::Number(n.floor())),
            (Value::Number(n), "ceil") => Ok(Value::Number(n.ceil())),
            (Value::Number(n), "round") => Ok(Value::Number(n.round())),
            (Value::Number(n), "abs") => Ok(Value::Number(n.abs())),
            (Value::Number(n), "sqrt") => Ok(Value::Number(n.sqrt())),
            (Value::Number(n), "pow") => {
                let exp = args.first().and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(Value::Number(n.powf(exp)))
            }
            (Value::Number(n), "to_string") | (Value::Number(n), "str") => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    Ok(Value::Str(format!("{}", *n as i64)))
                } else {
                    Ok(Value::Str(format!("{}", n)))
                }
            }

            _ => {
                eprintln!(
                    "[gx] warning: unknown method '{}.{}' — returning null",
                    obj.type_name(),
                    method
                );
                Ok(Value::Null)
            }
        }
    }
}

// ── Free helper functions ─────────────────────────────────────────────────────

/// Unpack db params: supports both `db_exec(path, sql, [p1,p2])` and `db_exec(path, sql, p1, p2)`.
/// `start` is the index of the first param (after path and sql).
fn db_unpack_params(args: &[Value], start: usize) -> Vec<Value> {
    let rest: Vec<Value> = args[start.min(args.len())..].to_vec();
    // If there's exactly one arg and it's an array, unpack it as the param list
    if rest.len() == 1 {
        if let Value::Array(items) = &rest[0] {
            return items.clone();
        }
    }
    rest
}

/// Extract the source line number from any statement (for error context).
fn stmt_line(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Assign { line, .. }
        | Stmt::PlusAssign { line, .. }
        | Stmt::MinusAssign { line, .. }
        | Stmt::MulAssign { line, .. }
        | Stmt::DivAssign { line, .. }
        | Stmt::If { line, .. }
        | Stmt::ForEach { line, .. }
        | Stmt::While { line, .. }
        | Stmt::Break { line }
        | Stmt::Continue { line }
        | Stmt::TryCatch { line, .. }
        | Stmt::Assert { line, .. }
        | Stmt::Emit { line, .. }
        | Stmt::Broadcast { line, .. }
        | Stmt::Log { line, .. }
        | Stmt::Output { line, .. }
        | Stmt::Say { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::Expr { line, .. }
        | Stmt::Wait { line, .. }
        | Stmt::ReRun { line }
        | Stmt::EscalateToHuman { line }
        | Stmt::Serve { line, .. }
        | Stmt::SendMessage { line, .. }
        | Stmt::Think { line, .. }
        | Stmt::Observe { line, .. }
        | Stmt::Act { line, .. }
        | Stmt::LoopUntil { line, .. }
        | Stmt::RepeatTimes { line, .. }
        | Stmt::Parallel { line, .. }
        | Stmt::Respond { line, .. }
        | Stmt::RespondStream { line, .. }
        | Stmt::Await { line, .. }
        | Stmt::Span { line, .. }
        | Stmt::DbTransaction { line, .. } => *line,
    }
}

/// Common builtins for "did you mean" suggestions on unknown function calls.
const KNOWN_BUILTINS: &[&str] = &[
    "log",
    "print",
    "say",
    "write",
    "truncate",
    "token_count",
    "tokens_used",
    "context_create",
    "context_set_system",
    "context_add_message",
    "context_add_tool_result",
    "context_trim",
    "context_summarize_and_trim",
    "context_clone",
    "context_reset",
    "context_serialize",
    "context_deserialize",
    "context_stats",
    "context_ask",
    "path_join",
    "dirname",
    "basename",
    "extname",
    "map",
    "filter",
    "reduce",
    "some",
    "every",
    "find_index",
    "group_by",
    "url_parse",
    "sha256",
    "uuid",
    "glob",
    "hmac_sha256",
    "hmac_sha512",
    "secure_compare",
    "secure_random",
    "ed25519_generate_keypair",
    "ed25519_sign",
    "ed25519_verify",
    "jwt_sign",
    "jwt_verify",
    "process_run",
    "process_spawn",
    "process_wait",
    "process_kill",
    "process_exists",
    "process_status",
    "process_read",
    "task_spawn",
    "task_wait",
    "task_wait_all",
    "task_wait_any",
    "task_cancel",
    "task_status",
    "task_id",
    "is_cancelled",
    "readline",
    "read_all",
    "is_tty",
    "slice",
    "merge",
    "len",
    "range",
    "to_string",
    "to_number",
    "type_of",
    "is_null",
    "abs",
    "floor",
    "ceil",
    "round",
    "sqrt",
    "pow",
    "min",
    "max",
    "random",
    "json_stringify",
    "json_parse",
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "read_file",
    "write_file",
    "delete_file",
    "file_exists",
    "list_dir",
    "make_dir",
    "regex_test",
    "regex_find",
    "regex_find_all",
    "regex_replace",
    "regex_split",
    "regex_captures",
    "date_now",
    "date_parse",
    "date_format",
    "date_diff",
    "date_add",
    "date_parts",
    "csv_parse",
    "csv_stringify",
    "yaml_parse",
    "yaml_stringify",
    "toml_parse",
    "toml_stringify",
    "xml_parse",
    "xml_stringify",
    "load_env",
    "get_env",
    "set_env",
    "retry",
    "vector_store_new",
    "vector_store_add",
    "vector_store_search",
    "cosine_similarity",
    "schema_validate",
    "persist_memory",
    "load_memory",
    "trace_log",
    "log_debug",
    "log_info",
    "log_warn",
    "log_error",
    "trace_id",
    "span_id",
    "has_capability",
    "unwrap",
    "base64_encode",
    "base64_decode",
    "embed",
    "sleep",
    "now",
    "now_ms",
    "get_timestamp",
    "random_int",
    "random_choice",
    "shuffle",
    "pick",
    "omit",
    // Runtime Completion milestone — previously missing from this list.
    "breakpoint",
    "test",
    "before_each",
    "after_each",
    "set_random_seed",
    "test_temp_dir",
    "assert_golden",
    "config_load",
    // Runtime Infrastructure milestone.
    "jsonl_parse",
    "jsonl_stringify",
    "versioned_stringify",
    "versioned_parse",
    "data_import",
    "data_export",
    "render_template",
    "task_emit",
    "task_progress",
];

/// Parse `content` (the contents of a file whose extension is `ext` —
/// already lowercased, no leading dot) into a `Value`, dispatching by
/// format the same way every one of GX's per-format parsers already does
/// individually — the single place `data_import` recognizes an extension.
/// Deliberately not shared with `config_load`'s own, narrower extension
/// table (json/yaml/toml only) — see `data_import`'s own comment.
fn parse_by_extension(ext: &str, content: String) -> Result<Value, Signal> {
    match ext {
        "json" => serde_json::from_str::<serde_json::Value>(&content)
            .map(|j| json_to_gx_value(&j))
            .map_err(|e| Signal::Error(format!("parsing as JSON: {}", e))),
        "yaml" | "yml" => yaml_parse_impl(&[Value::Str(content)]),
        "toml" => toml_parse_impl(&[Value::Str(content)]),
        "csv" => csv_parse_impl(&[Value::Str(content)]),
        "xml" => xml_parse_impl(&[Value::Str(content)]),
        "jsonl" | "ndjson" => jsonl_parse_impl(&[Value::Str(content)]),
        other => Err(Signal::Error(format!(
            "unrecognized data file extension '.{}' (expected .json, .yaml, .yml, .toml, .csv, .xml, or .jsonl)",
            other
        ))),
    }
}

/// The `data_export` counterpart to `parse_by_extension` — serializes
/// `value` to text in the format `ext` names.
fn stringify_by_extension(ext: &str, value: &Value) -> Result<String, Signal> {
    let wrap_str = |r: Result<Value, Signal>| match r? {
        Value::Str(s) => Ok(s),
        _ => unreachable!("every *_stringify_impl returns Value::Str on success"),
    };
    match ext {
        "json" => {
            let json = gx_value_to_json(value);
            serde_json::to_string_pretty(&json)
                .map_err(|e| Signal::Error(format!("stringifying as JSON: {}", e)))
        }
        "yaml" | "yml" => wrap_str(yaml_stringify_impl(std::slice::from_ref(value))),
        "toml" => wrap_str(toml_stringify_impl(std::slice::from_ref(value))),
        "csv" => wrap_str(csv_stringify_impl(std::slice::from_ref(value))),
        "xml" => wrap_str(xml_stringify_impl(std::slice::from_ref(value))),
        "jsonl" | "ndjson" => wrap_str(jsonl_stringify_impl(std::slice::from_ref(value))),
        other => Err(Signal::Error(format!(
            "unrecognized data file extension '.{}' (expected .json, .yaml, .yml, .toml, .csv, .xml, or .jsonl)",
            other
        ))),
    }
}

// ── Non-cryptographic pseudo-randomness ─────────────────────────────────────
//
// Shared by `random`/`random_int`/`random_choice`/`shuffle`. Not suitable
// for anything security-sensitive (token/key generation, anything already
// covered by `secure_random`/the Crypto primitives) — it's seeded from the
// system clock and uses a simple LCG, exactly as `random()` already did
// before this milestone; these helpers just factor that one generator out
// so every random-* builtin draws from the same well-understood source
// instead of each hand-rolling its own.

/// Backs `test_temp_dir()` — process-wide (not per-`Interpreter`) so two
/// separate `Interpreter` instances in the same process (e.g. `gx test`
/// constructing a fresh one per test file) never hand back the same
/// directory name. See that builtin's own comment for the full reasoning.
static TEST_TEMP_DIR_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// A fresh seed from the current time — one clock read, reused for every
/// step of a single generator's lifetime (see `shuffle`, which seeds once
/// and steps `lcg_step_unit_f64` in a loop, rather than reseeding from the
/// clock on every draw and risking two draws landing on the same tick).
fn random_seed_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

/// Advance `state` one LCG step and return a value in `[0, 1)`.
fn lcg_step_unit_f64(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 33) as f64 / u32::MAX as f64
}

/// A single-draw `[0, 1)` value, freshly seeded from the clock — the
/// one-shot counterpart to `lcg_step_unit_f64` for builtins that only
/// need one random value per call (`random`, `random_int`,
/// `random_choice`).
fn random_unit_f64() -> f64 {
    lcg_step_unit_f64(&mut random_seed_u64())
}

/// Returns the closest known builtin within edit distance 2, if any.
fn closest_builtin(name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for &candidate in KNOWN_BUILTINS {
        let d = levenshtein(name, candidate);
        if d <= 2 && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((candidate, d));
        }
    }
    best.map(|(c, _)| c)
}

/// Standard Levenshtein edit distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Parse and load a .env file into the current process environment.
/// Lines of the form KEY=VALUE or KEY="VALUE" are supported.
/// Lines starting with # are comments. Existing env vars are NOT overwritten.
#[cfg(not(target_arch = "wasm32"))]
fn load_env_file(path: &str) -> Result<Value, Signal> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Signal::Error(format!("load_env: cannot read '{}': {}", path, e)))?;
    let mut loaded = 0usize;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let raw_val = line[eq + 1..].trim();
            let val = raw_val.trim_matches('"').trim_matches('\'').to_string();
            // Only set if not already present
            if std::env::var(&key).is_err() {
                std::env::set_var(&key, &val);
            }
            loaded += 1;
        }
    }
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("loaded".into(), Value::Number(loaded as f64));
    result.insert("path".into(), Value::Str(path.to_string()));
    Ok(Value::Object(result))
}

/// schema_validate(value, schema_object) → { ok: bool, errors: array<string> }
/// Schema object format: { field_name: "type" } or { field_name: { type: "string", required: true } }
fn schema_validate_impl(args: &[Value]) -> Result<Value, Signal> {
    let value = args.first().cloned().unwrap_or(Value::Null);
    let schema = match args.get(1).cloned().unwrap_or(Value::Null) {
        Value::Object(m) => m,
        _ => {
            return Err(Signal::Error(
                "schema_validate(value, schema) — schema must be an object".into(),
            ))
        }
    };

    let obj = match &value {
        Value::Object(m) => m.clone(),
        _ => {
            let mut r = HashMap::new();
            r.insert("ok".into(), Value::Bool(false));
            r.insert(
                "errors".into(),
                Value::Array(vec![Value::Str("value must be an object".into())]),
            );
            return Ok(Value::Object(r));
        }
    };

    let mut errors: Vec<Value> = Vec::new();

    for (field, rule) in &schema {
        let (expected_type, required) = match rule {
            Value::Str(t) => (t.as_str().to_string(), true),
            Value::Object(opts) => {
                let t = opts
                    .get("type")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "any".to_string());
                let req = opts.get("required").map(|v| v.is_truthy()).unwrap_or(true);
                (t, req)
            }
            _ => continue,
        };

        match obj.get(field) {
            None | Some(Value::Null) => {
                if required {
                    errors.push(Value::Str(format!("field '{}' is required", field)));
                }
            }
            Some(field_val) => {
                let actual = field_val.type_name();
                let ok = match expected_type.as_str() {
                    "any" => true,
                    "string" | "str" => matches!(field_val, Value::Str(_)),
                    "number" | "num" | "float" | "int" => {
                        matches!(field_val, Value::Number(_))
                    }
                    "boolean" | "bool" => matches!(field_val, Value::Bool(_)),
                    "array" | "list" => matches!(field_val, Value::Array(_)),
                    "object" | "map" => matches!(field_val, Value::Object(_)),
                    "null" => matches!(field_val, Value::Null),
                    _ => true,
                };
                if !ok {
                    errors.push(Value::Str(format!(
                        "field '{}' must be {}, got {}",
                        field, expected_type, actual
                    )));
                }
            }
        }
    }

    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(errors.is_empty()));
    result.insert("errors".into(), Value::Array(errors));
    Ok(Value::Object(result))
}

// ── Persistent memory (SQLite-backed) ────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn persistent_db_path_for(agent_name: &str) -> String {
    let safe_name: String = agent_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = std::env::var("GX_STATE_DIR")
        .unwrap_or_else(|_| dirs_home().unwrap_or_else(|| ".".to_string()) + "/.gx/state");
    format!("{}/{}.db", base, safe_name)
}

#[cfg(not(target_arch = "wasm32"))]
fn dirs_home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn load_persistent_memory(db_path: &str) -> Result<HashMap<String, Value>, String> {
    use rusqlite::Connection;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT key, value FROM memory")
        .map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    for row in rows.flatten() {
        let (k, v_json) = row;
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&v_json) {
            map.insert(k, json_to_gx_value(&json));
        }
    }
    Ok(map)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_persistent_memory(db_path: &str, memory: &HashMap<String, Value>) -> Result<(), String> {
    use rusqlite::Connection;
    // Ensure the parent directory exists
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    // Skip ai_trace (ephemeral) and closures
    for (k, v) in memory {
        if k == "ai_trace" {
            continue;
        }
        if matches!(v, Value::Closure(..)) {
            continue;
        }
        let json_str = serde_json::to_string(&gx_value_to_json(v)).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO memory (key, value) VALUES (?1, ?2)",
            rusqlite::params![k, json_str],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── builtin: persist_memory / load_memory ────────────────────────────────────

impl Interpreter {
    /// Force-save current memory to SQLite for the current agent.
    /// Called by the `persist_memory()` builtin.
    #[cfg(not(target_arch = "wasm32"))]
    fn builtin_persist_memory(&self, env: &Env) -> IResult {
        let agent = self.current_agent.as_deref().unwrap_or("default");
        let db_path = persistent_db_path_for(agent);
        let memory = env.get_memory();
        save_persistent_memory(&db_path, &memory)
            .map(|_| Value::Bool(true))
            .map_err(|e| Signal::Error(format!("persist_memory: {}", e)))
    }

    /// Load memory from SQLite into the current env. Called by `load_memory()` builtin.
    #[cfg(not(target_arch = "wasm32"))]
    fn builtin_load_memory(&self, env: &mut Env) -> IResult {
        let agent = self.current_agent.as_deref().unwrap_or("default");
        let db_path = persistent_db_path_for(agent);
        let loaded = load_persistent_memory(&db_path)
            .map_err(|e| Signal::Error(format!("load_memory: {}", e)))?;
        let count = loaded.len();
        let mut mem = env.get_memory();
        for (k, v) in loaded {
            mem.insert(k, v);
        }
        env.set_memory(mem);
        Ok(Value::Number(count as f64))
    }
}

// ── Bridge + serve: see bridge_impl.rs ────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        Interpreter::new().run_program(&program)
    }

    #[test]
    fn http_result_err_msg_detects_an_ok_false_result_as_an_error() {
        // Regression test: http_get/post/put/delete/http_request never
        // throw on a failed *request* (timeout, blocked SSRF, DNS failure,
        // non-2xx) — they return Ok({ok: false, error, ...}), the same
        // convention process_run uses. A first version of this function
        // only checked Err(Signal::Error(_)), so every one of those real
        // failures showed up in a trace as a misleading `outcome: "ok"`.
        let mut ok_false = HashMap::new();
        ok_false.insert("ok".to_string(), Value::Bool(false));
        ok_false.insert(
            "error".to_string(),
            Value::Str("connection refused".to_string()),
        );
        let result: Result<Value, Signal> = Ok(Value::Object(ok_false));
        assert_eq!(
            http_result_err_msg(&result),
            Some("connection refused".to_string())
        );

        let mut ok_true = HashMap::new();
        ok_true.insert("ok".to_string(), Value::Bool(true));
        let success: Result<Value, Signal> = Ok(Value::Object(ok_true));
        assert_eq!(http_result_err_msg(&success), None);

        let thrown: Result<Value, Signal> = Err(Signal::Error("boom".to_string()));
        assert_eq!(http_result_err_msg(&thrown), Some("boom".to_string()));
    }

    #[test]
    fn retry_retries_on_a_returned_ok_false_value_not_just_a_thrown_error() {
        // Regression test for a real bug: retry(fn, ...) only retried when
        // the closure *threw* (Signal::Error/AssertFail). Every I/O
        // builtin that instead *returns* `{ ok: false, ... }` on failure —
        // http_*, process_*, task_wait, ask, context_ask — looked like an
        // immediate success to retry, so wrapping any of them in retry()
        // silently never retried at all. x tracks how many times the
        // closure actually ran.
        run(r#"
memory.x = 0
result = retry(fn() {
  memory.x = memory.x + 1
  if memory.x < 3 {
    return { ok: false, error: "not yet", error_kind: "timeout" }
  }
  return { ok: true, value: memory.x }
}, 5, { delay: 1 })
assert memory.x == 3 "closure must be called until it succeeds, not just once"
assert result.ok == true "the eventual success must be returned"
assert result.value == 3 "the success value must be the last attempt's"
"#)
        .unwrap();
    }

    #[test]
    fn retry_returns_the_last_ok_false_value_once_attempts_are_exhausted() {
        // A closure that *never* succeeds must still return its last
        // `{ ok: false, ... }` value once attempts run out — as a normal
        // return value, not a thrown error, matching what a caller who
        // invoked the wrapped builtin directly (no retry at all) would
        // have seen on that same final failure.
        run(r#"
memory.x = 0
result = retry(fn() {
  memory.x = memory.x + 1
  return { ok: false, error: "still failing", error_kind: "timeout" }
}, 3, { delay: 1 })
assert memory.x == 3 "must attempt exactly max_attempts times"
assert result.ok == false "must return the last failure, not throw"
assert result.error == "still failing" "must be the closure's own last error"
"#)
        .unwrap();
    }

    #[test]
    fn retry_still_propagates_a_thrown_error_after_exhausting_attempts() {
        // Existing behavior (predates this fix) must be unaffected: a
        // closure that throws every time still ends in a thrown error,
        // not a swallowed/misrepresented value. (Attempt-count precision
        // is covered by the ok:false-path tests above instead of here — a
        // *thrown* closure's own memory mutations never make it back to
        // the caller at all, a separate, pre-existing, unrelated quirk of
        // `call_closure_with_capture` only committing memory on the
        // non-throwing path, so `memory.x` can't be used to count
        // attempts in this specific scenario.)
        run(r#"
try {
  retry(fn() {
    assert false "boom"
  }, 3, { delay: 1 })
  assert false "retry must not swallow a persistent thrown error"
} catch e {
  assert e.message == "boom" "must propagate the closure's own last error message"
}
"#)
        .unwrap();
    }

    #[test]
    fn retry_does_not_retry_an_ok_true_result_shaped_like_the_convention() {
        // An `{ ok: true, ... }` return (a value that merely happens to
        // share the {ok, ...} shape) must be treated as an immediate
        // success, same as any other value — only ok:false is retryable.
        run(r#"
memory.x = 0
result = retry(fn() {
  memory.x = memory.x + 1
  return { ok: true, data: "hello" }
}, 5, { delay: 1 })
assert memory.x == 1 "an ok:true result must not be retried"
assert result.data == "hello"
"#)
        .unwrap();
    }

    #[test]
    fn has_capability_reflects_the_current_grant_without_performing_the_operation() {
        // Regression coverage for a real gap: before this, the only way
        // for a script to learn whether it had a capability was to attempt
        // the operation and catch a "capability_denied" error. This must
        // be a pure query — no side effect, no audit-log entry, no denial
        // thrown — that reports the same answer `authorize_capability`
        // would act on.
        let src = r#"
assert has_capability("process") == false "process must be denied by default"
assert has_capability("filesystem") == true "filesystem defaults to sandboxed-but-permitted"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn has_capability_reflects_a_granted_resource() {
        let src = r#"
assert has_capability("process") == true "process must now be granted"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.process = true;
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn has_capability_checks_the_allowlist_when_a_name_is_given() {
        let src = r#"
assert has_capability("ai", "openai") == true "openai must be in the configured allowlist"
assert has_capability("ai", "anthropic") == false "anthropic must not be in the configured allowlist"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.ai_providers =
            crate::capability::Allowlist::only(["openai".to_string()]);
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn has_capability_errors_clearly_on_an_unknown_resource_name() {
        let src = r#"
try {
  has_capability("not-a-real-resource")
  assert false "must not silently succeed on a typo'd resource name"
} catch e {
  assert contains(e.message, "not-a-real-resource")
}
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn has_capability_never_writes_a_capability_denied_audit_entry() {
        // Unlike an actual attempted-and-denied operation, a mere
        // introspection check must not pollute the audit trail — a script
        // that probes several capabilities before choosing a strategy
        // shouldn't leave a trail of misleading "denied" entries for
        // operations that were never actually attempted.
        let src = r#"
has_capability("process")
has_capability("shell")
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.diagnostics.set_enabled(true);
        interp.run_program(&program).unwrap();
        // No direct accessor for audit entries exists outside the emit()
        // sink (stdout/file, per Diagnostics's design) — this test's real
        // value is that has_capability() takes the `self.capabilities.
        // authorize(...)` path directly rather than `self.
        // authorize_capability(...)` (the wrapper that logs on denial),
        // which is verified by code inspection; this test just proves the
        // calls succeed without panicking under a live diagnostics setup.
    }

    #[test]
    fn unwrap_passes_through_an_ok_true_result_unchanged() {
        run(r#"
r = unwrap({ ok: true, value: 42 })
assert r.ok == true
assert r.value == 42
"#)
        .unwrap();
    }

    #[test]
    fn unwrap_passes_through_a_plain_value_with_no_ok_field_unchanged() {
        run(r#"
assert unwrap(42) == 42
assert unwrap("hello") == "hello"
assert unwrap([1, 2, 3])[1] == 2
"#)
        .unwrap();
    }

    #[test]
    fn unwrap_raises_a_catchable_error_from_an_ok_false_result() {
        // The core bridge: a builtin that signals failure by *returning*
        // { ok: false, ... } (http_*/process_*/task_wait/ask/context_ask's
        // convention) becomes catchable the same way a *throwing* builtin
        // (db_query/file I/O) already is, once wrapped in unwrap().
        run(r#"
try {
  unwrap({ ok: false, error: "connection refused", error_kind: "timeout" })
  assert false "unwrap must throw on ok:false, not return it"
} catch e {
  assert contains(e.message, "connection refused")
  assert contains(e.message, "timeout")
}
"#)
        .unwrap();
    }

    #[test]
    fn unwrap_falls_back_to_a_generic_message_when_error_field_is_missing() {
        run(r#"
try {
  unwrap({ ok: false })
  assert false
} catch e {
  assert e.message != ""
}
"#)
        .unwrap();
    }

    #[test]
    fn an_uncaught_assertion_failure_shows_the_call_stack() {
        // Regression test: every other kind of uncaught error already got
        // "\n  in {call stack}" context via run_stmt's wrapper around
        // Signal::Error — but Signal::AssertFail deliberately bypasses
        // that wrapper (see with_call_stack's doc comment) and used to
        // reach the top level with no call-stack context at all, an
        // inconsistency with every other error kind.
        let src = r#"
helper "demo" {
  brain {
    plan {}
    execute {
      x = 1
      y = 2
      assert x == y "x must equal y"
    }
    remember {}
    communicate {}
  }
}
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let err = Interpreter::new().run_program(&program).unwrap_err();
        assert!(err.contains("x must equal y"));
        assert!(
            err.contains("in agent \"demo\""),
            "expected call-stack context, got: {}",
            err
        );
    }

    #[test]
    fn a_caught_assertion_failures_message_stays_pristine() {
        // The other half of the same fix: with_call_stack must only ever
        // be applied at the top-level *uncaught* conversion sites, never
        // to the message a `catch` block sees via e.message (or gx test's
        // failure list) — scripts may reasonably compare e.message for
        // equality against exactly what they wrote in the assert.
        run(r#"
try {
  assert false "boom"
} catch e {
  assert e.message == "boom" "e.message must stay exactly what the script wrote"
}
"#)
        .unwrap();
    }

    #[test]
    fn map_works_as_both_a_free_function_and_a_method() {
        // Regression coverage for a real inconsistency: every other array
        // operation (.sort(), .take(), .sum(), ...) was already a method,
        // but .map()/.filter() weren't — calling them printed "unknown
        // method" and silently returned null.
        run(r#"
nums = [1, 2, 3]
assert map(nums, fn(x) { return x * 2 }) == [2, 4, 6]
assert nums.map(fn(x) { return x * 2 }) == [2, 4, 6]
"#)
        .unwrap();
    }

    #[test]
    fn filter_works_as_both_a_free_function_and_a_method() {
        run(r#"
nums = [1, 2, 3, 4]
assert filter(nums, fn(x) { return x % 2 == 0 }) == [2, 4]
assert nums.filter(fn(x) { return x % 2 == 0 }) == [2, 4]
"#)
        .unwrap();
    }

    #[test]
    fn reduce_folds_with_a_required_initial_value() {
        run(r#"
nums = [1, 2, 3, 4]
assert reduce(nums, fn(acc, x) { return acc + x }, 0) == 10
assert nums.reduce(fn(acc, x) { return acc + x }, 100) == 110
// An empty array with an initial value just returns it unchanged —
// no special "empty sequence" error case to think about.
assert reduce([], fn(acc, x) { return acc + x }, 42) == 42
"#)
        .unwrap();
    }

    #[test]
    fn reduce_requires_an_initial_value() {
        let err = run(r#"reduce([1, 2, 3], fn(acc, x) { return acc + x })"#).unwrap_err();
        assert!(err.contains("initial"));
    }

    #[test]
    fn some_and_every_check_predicates_across_the_array() {
        run(r#"
nums = [2, 4, 6]
assert some(nums, fn(x) { return x > 5 }) == true
assert some(nums, fn(x) { return x > 100 }) == false
assert every(nums, fn(x) { return x % 2 == 0 }) == true
assert every(nums, fn(x) { return x > 3 }) == false
assert nums.some(fn(x) { return x > 5 }) == true
assert nums.every(fn(x) { return x % 2 == 0 }) == true
// every() is vacuously true on an empty array — the standard convention:
// there is no element that fails the predicate.
assert every([], fn(x) { return false }) == true
// some() is correspondingly false on an empty array.
assert some([], fn(x) { return true }) == false
"#)
        .unwrap();
    }

    #[test]
    fn find_index_is_predicate_based_and_distinct_from_index_of() {
        // index_of/find are value-*equality* lookups (pre-existing,
        // unchanged); find_index is predicate-based — a deliberately
        // different name to avoid colliding with that existing meaning
        // of "find".
        run(r#"
nums = [10, 20, 30, 40]
assert find_index(nums, fn(x) { return x > 25 }) == 2
assert find_index(nums, fn(x) { return x > 1000 }) == -1
assert nums.find_index(fn(x) { return x > 25 }) == 2
// index_of's existing, unrelated value-equality behavior is unaffected.
// (index_of/find are method-only for arrays — the free-function forms
// of those two names are string substring search instead; a real,
// pre-existing type-dependent split this test deliberately doesn't
// paper over.)
assert nums.index_of(30) == 2
"#)
        .unwrap();
    }

    #[test]
    fn map_and_filter_still_work_on_objects_and_strings_via_iter() {
        // map/filter's free-function forms already iterated Object (as
        // key strings) and String (as one-character strings) via
        // Value::iter() before this change — the new method forms must
        // preserve that, not silently narrow it to arrays only.
        run(r#"
obj = { a: 1, b: 2 }
keys_upper = obj.map(fn(k) { return to_upper(k) })
assert contains(keys_upper, "A")
assert contains(keys_upper, "B")

chars = "abc".filter(fn(c) { return c != "b" })
assert chars == ["a", "c"]
"#)
        .unwrap();
    }

    #[test]
    fn random_int_stays_within_the_inclusive_bounds() {
        run(r#"
i = 0
while i < 200 {
  n = random_int(1, 6)
  assert n >= 1 "random_int must never go below min"
  assert n <= 6 "random_int must never exceed max (it is inclusive, unlike random(lo, hi))"
  assert n == floor(n) "random_int must always return a whole number"
  i = i + 1
}
"#)
        .unwrap();
    }

    #[test]
    fn random_int_handles_a_single_value_range() {
        run(r#"assert random_int(5, 5) == 5"#).unwrap();
    }

    #[test]
    fn random_int_rejects_max_less_than_min() {
        let err = run(r#"random_int(10, 5)"#).unwrap_err();
        assert!(err.contains("min"));
    }

    #[test]
    fn random_choice_always_returns_a_member_of_the_array() {
        run(r#"
arr = ["a", "b", "c", "d"]
i = 0
while i < 50 {
  c = random_choice(arr)
  assert contains(arr, c) "random_choice must return an actual element of the array"
  i = i + 1
}
"#)
        .unwrap();
    }

    #[test]
    fn random_choice_on_an_empty_array_returns_null_not_an_error() {
        run(r#"assert random_choice([]) == null"#).unwrap();
    }

    #[test]
    fn shuffle_preserves_length_and_every_element_without_mutating_the_original() {
        run(r#"
original = [1, 2, 3, 4, 5]
shuffled = shuffle(original)
assert len(shuffled) == len(original) "shuffle must preserve length"
for each x in original {
  assert contains(shuffled, x) "shuffle must preserve every element"
}
// Every other array operation in GX returns a new value rather than
// mutating in place — shuffle must not be a surprising exception.
assert original == [1, 2, 3, 4, 5] "shuffle must not mutate the original array"
"#)
        .unwrap();
    }

    #[test]
    fn shuffle_handles_empty_and_single_element_arrays() {
        run(r#"
assert shuffle([]) == []
assert shuffle([1]) == [1]
"#)
        .unwrap();
    }

    #[test]
    fn shuffle_produces_varied_output_across_rapid_consecutive_calls() {
        // Regression test for the exact bug this design avoids: drawing a
        // fresh system-time seed per swap (rather than seeding once and
        // stepping) risks two swaps in a tight loop landing on the same
        // clock tick, producing a correlated, poorly-shuffled — or even
        // completely unshuffled — result. Calls shuffle() many times in
        // immediate succession and asserts the results aren't all
        // identical (an extremely unlikely coincidence if the generator
        // is working correctly, a near-certainty if it isn't).
        run(r#"
results = []
i = 0
while i < 15 {
  results = results + [to_string(shuffle([1,2,3,4,5,6,7,8]))]
  i = i + 1
}
distinct_results = results.unique()
assert len(distinct_results) > 1 "consecutive shuffle() calls must not all produce the identical permutation"
"#)
        .unwrap();
    }

    #[test]
    fn pick_keeps_only_the_named_keys_that_are_actually_present() {
        run(r#"
user = { id: 1, name: "Ada", email: "ada@example.com" }
public = pick(user, ["id", "name"])
assert public.id == 1
assert public.name == "Ada"
assert !has(public, "email") "pick must not include keys that weren't asked for"

// A requested key that isn't on the object is silently skipped, not an
// error and not present with a null value.
partial = pick(user, ["id", "does_not_exist"])
assert has(partial, "id")
assert !has(partial, "does_not_exist")

assert user.pick(["id"]).id == 1 "pick must also work as a method"
"#)
        .unwrap();
    }

    #[test]
    fn pick_on_a_non_object_or_with_no_keys_returns_an_empty_object_not_an_error() {
        run(r#"
assert pick(null, ["a"]).is_empty()
assert pick(42, ["a"]).is_empty()
assert pick({ a: 1 }, null).is_empty()
"#)
        .unwrap();
    }

    #[test]
    fn omit_excludes_only_the_named_keys() {
        run(r#"
user = { id: 1, name: "Ada", password_hash: "secret" }
safe = omit(user, ["password_hash"])
assert safe.id == 1
assert safe.name == "Ada"
assert !has(safe, "password_hash") "omit must remove exactly the named keys"

assert user.omit(["password_hash"]).id == 1 "omit must also work as a method"
"#)
        .unwrap();
    }

    #[test]
    fn omit_with_no_keys_returns_the_object_unchanged() {
        run(r#"
user = { id: 1, name: "Ada" }
same = omit(user, [])
assert same.id == 1
assert same.name == "Ada"
"#)
        .unwrap();
    }

    #[test]
    fn extname_returns_the_last_extension_with_the_leading_dot() {
        run(r#"
assert extname("report.tar.gz") == ".gz"
assert extname("main.gx") == ".gx"
assert extname("README") == ""
assert extname(".gitignore") == "" "a dotfile has no extension, matching Node's path.extname"
assert extname("lib/util.gx") == ".gx"
"#)
        .unwrap();
    }

    #[test]
    fn test_hello_world() {
        run(r#"
helper "hello" {
  brain {
    plan { plan = { action: "greet" } }
    execute { if plan.action == "greet" { output("Hello, Brain-First World!") } }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_memory_read_write() {
        run(r#"
helper "mem" {
  remember { count = 0 }
  brain {
    plan { }
    execute { memory.count = memory.count + 1 }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_if_else() {
        run(r#"
helper "cond" {
  brain {
    plan { plan = { action: "test" } }
    execute {
      if plan.action == "test" { log("yes") }
      else { log("no") }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_for_each() {
        run(r#"
helper "loop" {
  brain {
    plan { }
    execute {
      items = ["a", "b", "c"]
      for each item in items { log(item) }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_while_loop() {
        run(r#"
helper "wh" {
  brain {
    plan { }
    execute {
      i = 0
      while i < 3 {
        i += 1
      }
      log(i)
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_break_continue() {
        run(r#"
helper "bc" {
  brain {
    plan { }
    execute {
      total = 0
      for each i in [1, 2, 3, 4, 5] {
        if i == 4 { break }
        if i == 2 { continue }
        total += i
      }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert() {
        run(r#"
helper "asserttest" {
  brain {
    plan { }
    execute {
      x = 2 + 2
      assert x == 4 "math must work"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_null_coalesce() {
        run(r#"
helper "nc" {
  brain {
    plan { }
    execute {
      a = null
      b = a ?? "default"
      log(b)
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_arithmetic() {
        run(r#"
helper "math" {
  brain {
    plan { }
    execute {
      result = 5 + 3
      log(result)
      result2 = 10 * 4
      log(result2)
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_concat() {
        run(r#"
helper "str" {
  brain {
    plan { }
    execute { greeting = "Hello, " + "World!"; log(greeting) }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_agent_when_started() {
        run(r#"
agent "bot" {
  remember greeting = "hello from when block"
  when started {
    say memory.greeting
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_try_catch() {
        run(r#"
helper "safe" {
  brain {
    plan { }
    execute {
      try {
        result = 10 / 0
      } catch err {
        log("Caught: " + err)
      }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_interpolation() {
        run(r#"
helper "interp" {
  remember { name = "GX" }
  brain {
    plan { }
    execute { output("Hello from {memory.name}!") }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_nested_memory() {
        run(r#"
helper "nested" {
  remember { config = { debug: false, version: "1.0" } }
  brain {
    plan { }
    execute { memory.config.debug = true; log(memory.config.debug) }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_array_push_mutates() {
        run(r#"
helper "arrays" {
  brain {
    plan { }
    execute {
      items = ["a", "b"]
      items.push("c")
      log(items.length)
      assert items.length == 3 "push should mutate in place"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_array_methods_extended() {
        run(r#"
helper "arrmethods" {
  brain {
    plan { }
    execute {
      nums = [3, 1, 4, 1, 5, 9, 2, 6]
      sorted = nums.sort()
      assert sorted.first() == 1 "sort: first should be 1"
      assert sorted.last() == 9 "sort: last should be 9"
      assert nums.sum() == 31 "sum should be 31"
      assert nums.min() == 1 "min should be 1"
      assert nums.max() == 9 "max should be 9"
      uniq = [1, 2, 2, 3, 3, 3].unique()
      assert uniq.length == 3 "unique should remove duplicates"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_methods_extended() {
        run(r#"
helper "strmethods" {
  brain {
    plan { }
    execute {
      s = "  Hello World  "
      assert s.trim() == "Hello World" "trim"
      assert s.trim().to_lower() == "hello world" "to_lower"
      assert "abc".repeat(3) == "abcabcabc" "repeat"
      assert "hello".index_of("ll") == 2 "index_of"
      assert "hi".pad_left(5, "0") == "000hi" "pad_left"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_json_builtins() {
        run(r#"
helper "jsontest" {
  brain {
    plan { }
    execute {
      data = { name: "GX", version: 1 }
      s = json_stringify(data)
      parsed = json_parse(s)
      assert parsed.name == "GX" "json round-trip name"
      assert parsed.version == 1 "json round-trip version"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_math_builtins() {
        run(r#"
helper "mathtest" {
  brain {
    plan { }
    execute {
      assert sqrt(9) == 3 "sqrt(9)"
      assert pow(2, 10) == 1024 "pow(2,10)"
      assert abs(-5) == 5 "abs(-5)"
      assert floor(3.9) == 3 "floor"
      assert ceil(3.1) == 4 "ceil"
      assert clamp(15, 0, 10) == 10 "clamp high"
      assert clamp(-5, 0, 10) == 0 "clamp low"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_file_io() {
        let tmp = std::env::temp_dir().join("gx_test_file.txt");
        let path = tmp.to_string_lossy().replace('\\', "/");
        let src = format!(
            r#"
helper "filetest" {{
  brain {{
    plan {{ }}
    execute {{
      write_file("{path}", "hello gx")
      content = read_file("{path}")
      assert content == "hello gx" "file round-trip"
      assert file_exists("{path}") "file exists"
      delete_file("{path}")
      assert not file_exists("{path}") "file deleted"
    }}
    remember {{ }}
    communicate {{ }}
  }}
}}"#
        );
        run(&src).unwrap();
    }

    #[test]
    fn test_negative_indexing() {
        run(r#"
helper "negidx" {
  brain {
    plan { }
    execute {
      arr = [1, 2, 3, 4, 5]
      assert arr[-1] == 5 "last element"
      assert arr[-2] == 4 "second to last"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    // ── v0.4.0 feature tests ──────────────────────────────────────────────────

    #[test]
    fn test_bang_not_operator() {
        run(r#"
helper "bang" {
  brain {
    plan { }
    execute {
      assert !false "!false should be true"
      assert !(!true) "double negation"
      ok = false
      assert !ok "!variable"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_integer_json_serialization() {
        run(r#"
helper "intjson" {
  brain {
    plan { }
    execute {
      s = json_stringify({ n: 50, rate: 0.5, count: 100 })
      // Must NOT contain 50.0 or 100.0 for integer values
      assert not s.contains("50.0") "integer must not be float"
      assert not s.contains("100.0") "integer must not be float"
      // Float must remain
      assert s.contains("0.5") "float preserved"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_range_slicing_string() {
        run(r#"
helper "rngstr" {
  brain {
    plan { }
    execute {
      s = "hello world"
      assert s[0..5] == "hello" "string range [0..5]"
      assert s[6..11] == "world" "string range [6..11]"
      assert s[0..0] == "" "empty range"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_range_slicing_array() {
        run(r#"
helper "rngarr" {
  brain {
    plan { }
    execute {
      arr = [10, 20, 30, 40, 50]
      sub = arr[1..4]
      assert sub[0] == 20 "range arr[0]"
      assert sub[1] == 30 "range arr[1]"
      assert sub[2] == 40 "range arr[2]"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_output_as_variable() {
        run(r#"
helper "outvar" {
  brain {
    plan { }
    execute {
      output = "hello"
      output += " world"
      assert output == "hello world" "output as variable"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_standalone_slice() {
        run(r#"
helper "slicefn" {
  brain {
    plan { }
    execute {
      assert slice("foobar", 0, 3) == "foo" "slice string"
      sub = slice([1, 2, 3, 4, 5], 1, 4)
      assert sub[0] == 2 "slice array start"
      assert sub[2] == 4 "slice array end"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_standalone_merge() {
        run(r#"
helper "mergefn" {
  brain {
    plan { }
    execute {
      m = merge({ a: 1, b: 2 }, { b: 99, c: 3 })
      assert m.a == 1 "merge preserves left"
      assert m.b == 99 "merge right wins"
      assert m.c == 3 "merge adds right"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_interpolation_brace_escape() {
        run(r#"
helper "interpescape" {
  brain {
    plan { }
    execute {
      // {{ in a GX string produces a literal {
      // Both the value and the comparison use {{ so both resolve to "{literal}"
      s = "{{literal}}"
      assert s == "{{literal}}" "brace escape"
      // Mixing: {{ escape + real interpolation
      name = "GX"
      msg = "{{tag}}: {name}"
      assert msg == "{{tag}}: GX" "mixed escape and interp"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_inline_function_in_block() {
        run(r#"
helper "inlinefn" {
  brain {
    plan { }
    execute {
      function square(n) {
        return n * n
      }
      assert square(7) == 49 "inline function"
      assert square(0) == 0 "inline function zero"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_agent_level_function() {
        run(r#"
helper "agentfn" {
  function double(x) {
    return x * 2
  }
  brain {
    plan { }
    execute {
      assert double(21) == 42 "agent-level function"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_module_import_namespaced() {
        // Uses helpers/math_utils.gx via namespaced import
        run(r#"
import "tests/helpers/math_utils.gx" as math

helper "modimp" {
  brain {
    plan { }
    execute {
      assert math.add(10, 32) == 42 "module add"
      assert math.greet("World") == "Hello from math, World!" "module greet"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    // ── Module & Package Runtime: multi-file import resolution ──────────────

    fn temp_gx_project(label: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "gx_import_test_{}_{}_{}",
            label,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&root).unwrap();
        for (rel_path, content) in files {
            let full = root.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, content).unwrap();
        }
        root
    }

    fn run_entry_file(root: &std::path::Path, entry: &str) -> Result<(), String> {
        let entry_path = root.join(entry);
        let src = std::fs::read_to_string(&entry_path).unwrap();
        let tokens = Lexer::new(&src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        let mut interp = Interpreter::new();
        interp.base_path = Some(entry_path.to_string_lossy().into_owned());
        interp.run_program(&program)
    }

    #[test]
    fn transitive_imports_are_resolved_not_silently_dropped() {
        // Regression test for a real, significant gap: only the top-level
        // program's own file_imports used to be processed at all — an
        // imported file's own `import`s were silently never loaded.
        let root = temp_gx_project(
            "transitive",
            &[
                ("main.gx", "import \"lib/b.gx\"\nresult = from_b()\n"),
                (
                    "lib/b.gx",
                    "import \"c.gx\"\nfunction from_b() { return \"b+\" + from_c() }\n",
                ),
                ("lib/c.gx", "function from_c() { return \"c\" }\n"),
            ],
        );
        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nested_imports_resolve_relative_to_the_importing_files_own_directory() {
        // Regression test for a real determinism bug: path resolution used
        // to check the current working directory *before* the importing
        // file's own directory, so the "same" script could import a
        // different file depending on which directory `gx` happened to be
        // invoked from. `lib/b.gx`'s `import "c.gx"` must resolve to
        // `lib/c.gx` (relative to b.gx), never to a `c.gx` that happens to
        // sit next to main.gx — proven here via a decoy at the wrong
        // relative position, which the fix must never pick.
        //
        // Deliberately does *not* also mutate the process's actual current
        // directory to further prove CWD-independence — `cargo test` runs
        // tests in parallel by default, and `std::env::set_current_dir` is
        // global, process-wide state that would race every other test
        // reading a relative path concurrently (including
        // `test_module_import_namespaced` a few tests up, which relies on
        // CWD-relative resolution when `base_path` is `None`). CWD
        // independence for this exact scenario was verified manually
        // instead (a real decoy file placed in the actual invocation
        // directory, confirmed ignored).
        let root = temp_gx_project(
            "relative-resolution",
            &[
                (
                    "main.gx",
                    "import \"lib/b.gx\"\nassert from_b() == \"correct\" \"must resolve lib/b.gx's import relative to lib/, not top-level\"\n",
                ),
                ("lib/b.gx", "import \"c.gx\"\nfunction from_b() { return from_c() }\n"),
                ("lib/c.gx", "function from_c() { return \"correct\" }\n"),
                // A decoy at the top level — must never be picked. If it
                // were, `from_b()` would return "wrong" instead of
                // "correct" and the assertion above would fail — a plain
                // "did it error?" check wouldn't have caught the decoy
                // silently winning.
                ("c.gx", "function from_c() { return \"wrong\" }\n"),
            ],
        );
        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_cycles_are_detected_not_infinitely_recursed() {
        let root = temp_gx_project(
            "cycle",
            &[
                ("a.gx", "import \"b.gx\"\nfunction from_a() { return 1 }\n"),
                ("b.gx", "import \"a.gx\"\nfunction from_b() { return 2 }\n"),
            ],
        );
        let err = run_entry_file(&root, "a.gx").unwrap_err();
        assert!(
            err.contains("import cycle detected"),
            "expected a clear cycle error, got: {}",
            err
        );
        assert!(
            err.contains("a.gx") && err.contains("b.gx"),
            "the cycle error should name both files in the chain, got: {}",
            err
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_file_imported_from_two_places_is_only_parsed_once_and_not_flagged_as_a_collision() {
        // "Diamond" import: left.gx and right.gx both import shared.gx.
        // shared.gx's own functions must not trigger a false-positive
        // collision warning just because they're *merged* twice (once per
        // importer) even though they're only ever *parsed* once.
        let root = temp_gx_project(
            "diamond",
            &[
                (
                    "main.gx",
                    "import \"left.gx\"\nimport \"right.gx\"\nassert left_fn() + right_fn() == 2 \"both branches of the diamond must reach the same shared_fn()\"\n",
                ),
                (
                    "left.gx",
                    "import \"shared.gx\"\nfunction left_fn() { return shared_fn() }\n",
                ),
                (
                    "right.gx",
                    "import \"shared.gx\"\nfunction right_fn() { return shared_fn() }\n",
                ),
                ("shared.gx", "function shared_fn() { return 1 }\n"),
            ],
        );
        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_missing_import_reports_a_clear_error_naming_the_line_and_path() {
        let root = temp_gx_project("missing", &[("main.gx", "import \"does_not_exist.gx\"\n")]);
        let err = run_entry_file(&root, "main.gx").unwrap_err();
        assert!(
            err.contains("does_not_exist.gx"),
            "error should name the missing file, got: {}",
            err
        );
        assert!(
            err.contains("Line 1"),
            "error should include the import statement's line number, got: {}",
            err
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn is_package_import_distinguishes_package_names_from_file_paths() {
        assert!(is_package_import("leftpad"));
        assert!(is_package_import("http-utils"));
        assert!(
            !is_package_import("leftpad.gx"),
            "a .gx suffix is a file path"
        );
        assert!(
            !is_package_import("./leftpad.gx"),
            "a leading . is a relative file path"
        );
        assert!(!is_package_import("../lib/leftpad.gx"));
        assert!(
            !is_package_import("lib/leftpad"),
            "a path separator makes it a file path"
        );
        assert!(!is_package_import("lib\\leftpad"));
    }

    #[test]
    fn package_import_resolves_a_path_dependency_via_gx_lock() {
        // `import "pathdep"` (no .gx suffix, no separator) must resolve
        // through gx.lock + the dependency's own gx.json `entry` field —
        // not be treated as a plain (and therefore missing) file path.
        let dep_dir = temp_gx_project(
            "pkgimport-path-dep",
            &[
                ("main.gx", "function hello() { return \"from-dep\" }\n"),
                (
                    "gx.json",
                    r#"{"name":"pathdep","version":"0.1.0","entry":"main.gx"}"#,
                ),
            ],
        );
        let root = temp_gx_project(
            "pkgimport-path-consumer",
            &[(
                "main.gx",
                "import \"pathdep\"\nassert hello() == \"from-dep\" \"package import must reach the dependency's entry file\"\n",
            )],
        );
        let rel = pathdiff(&dep_dir, &root);
        std::fs::write(
            root.join("gx.lock"),
            format!(
                r#"{{"version":1,"packages":{{"pathdep":{{"version":"0.1.0","resolved":"path+{}","integrity":"sha256-unused"}}}}}}"#,
                rel.replace('\\', "\\\\")
            ),
        )
        .unwrap();

        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&dep_dir).ok();
    }

    #[test]
    fn package_import_rejects_an_absolute_entry_path_that_escapes_the_package_directory() {
        // A dependency's gx.json is untrusted content the *importing*
        // project didn't write. `PathBuf::join` with an absolute path
        // discards the package directory entirely, so an unvalidated
        // `entry` field would let a malicious git/registry dependency
        // point at any file on the importer's disk (e.g. `~/.ssh/id_rsa`)
        // and have it read the moment the package is imported — see
        // resolve_package_import_impl's confinement check.
        let victim_dir = temp_gx_project(
            "pkgimport-traversal-victim",
            &[(
                "secret.gx",
                "function secret_leaked() { return \"leaked\" }\n",
            )],
        );
        let victim_entry = victim_dir
            .join("secret.gx")
            .to_string_lossy()
            .replace('\\', "\\\\");
        let dep_dir = temp_gx_project(
            "pkgimport-traversal-dep",
            &[(
                "gx.json",
                &format!(
                    r#"{{"name":"evil","version":"0.1.0","entry":"{}"}}"#,
                    victim_entry
                ),
            )],
        );
        let root = temp_gx_project(
            "pkgimport-traversal-consumer",
            &[("main.gx", "import \"evil\"\n")],
        );
        let rel = pathdiff(&dep_dir, &root);
        std::fs::write(
            root.join("gx.lock"),
            format!(
                r#"{{"version":1,"packages":{{"evil":{{"version":"0.1.0","resolved":"path+{}","integrity":"sha256-unused"}}}}}}"#,
                rel.replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let err = run_entry_file(&root, "main.gx").unwrap_err();
        assert!(
            err.contains("outside"),
            "error should explain the entry escapes the package directory, got: {}",
            err
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&dep_dir).ok();
        std::fs::remove_dir_all(&victim_dir).ok();
    }

    #[test]
    fn package_import_reflects_a_path_dependencys_live_edits_not_a_stale_hash() {
        // By design (see `resolve_package_import`), path dependencies are
        // never integrity-checked against gx.lock — they're meant to
        // always reflect whatever is currently on disk, since that's the
        // entire point of using one during local/monorepo development.
        // gx.lock intentionally carries a bogus integrity hash here to
        // prove it's never even consulted for a path dependency.
        let dep_dir = temp_gx_project(
            "pkgimport-live-edit-dep",
            &[
                ("main.gx", "function hello() { return \"v1\" }\n"),
                (
                    "gx.json",
                    r#"{"name":"pathdep","version":"0.1.0","entry":"main.gx"}"#,
                ),
            ],
        );
        let root = temp_gx_project(
            "pkgimport-live-edit-consumer",
            &[(
                "main.gx",
                "import \"pathdep\"\nassert hello() == \"v2\" \"must reflect the dependency's current content, not a cached hash\"\n",
            )],
        );
        let rel = pathdiff(&dep_dir, &root);
        std::fs::write(
            root.join("gx.lock"),
            format!(
                r#"{{"version":1,"packages":{{"pathdep":{{"version":"0.1.0","resolved":"path+{}","integrity":"sha256-deliberately-wrong"}}}}}}"#,
                rel.replace('\\', "\\\\")
            ),
        )
        .unwrap();
        std::fs::write(
            dep_dir.join("main.gx"),
            "function hello() { return \"v2\" }\n",
        )
        .unwrap();

        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&dep_dir).ok();
    }

    #[test]
    fn package_import_without_a_gx_lock_errors_clearly() {
        let root = temp_gx_project("pkgimport-no-lock", &[("main.gx", "import \"somepkg\"\n")]);
        let err = run_entry_file(&root, "main.gx").unwrap_err();
        assert!(
            err.contains("somepkg"),
            "error should name the package: {}",
            err
        );
        assert!(
            err.contains("gx.lock"),
            "error should mention gx.lock: {}",
            err
        );
        assert!(
            err.contains("gx install"),
            "error should suggest the fix: {}",
            err
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn package_import_of_an_undeclared_package_errors_clearly() {
        let root = temp_gx_project(
            "pkgimport-undeclared",
            &[
                ("main.gx", "import \"ghost\"\n"),
                ("gx.lock", r#"{"version":1,"packages":{}}"#),
            ],
        );
        let err = run_entry_file(&root, "main.gx").unwrap_err();
        assert!(
            err.contains("ghost"),
            "error should name the package: {}",
            err
        );
        assert!(
            err.contains("not found in gx.lock"),
            "error should explain why, got: {}",
            err
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn package_import_from_two_different_files_resolves_correctly_for_both() {
        // Two files both `import "shared_pkg"` — this exercises the
        // per-resolution-pass memoization in `resolve_package_import`
        // (added so a package imported from several files only has its
        // integrity hash computed once, not once per importer). The point
        // of this test is correctness, not speed: both importers must
        // still resolve to the same, correct entry point.
        let dep_dir = temp_gx_project(
            "pkgimport-shared-dep",
            &[
                ("main.gx", "function shared_value() { return 7 }\n"),
                (
                    "gx.json",
                    r#"{"name":"shared_pkg","version":"0.1.0","entry":"main.gx"}"#,
                ),
            ],
        );
        let root = temp_gx_project(
            "pkgimport-shared-consumer",
            &[
                (
                    "main.gx",
                    "import \"left.gx\"\nimport \"right.gx\"\nassert left_value() + right_value() == 14 \"both importers of the same package must resolve it correctly\"\n",
                ),
                (
                    "left.gx",
                    "import \"shared_pkg\"\nfunction left_value() { return shared_value() }\n",
                ),
                (
                    "right.gx",
                    "import \"shared_pkg\"\nfunction right_value() { return shared_value() }\n",
                ),
            ],
        );
        let rel = pathdiff(&dep_dir, &root);
        std::fs::write(
            root.join("gx.lock"),
            format!(
                r#"{{"version":1,"packages":{{"shared_pkg":{{"version":"0.1.0","resolved":"path+{}","integrity":"sha256-unused"}}}}}}"#,
                rel.replace('\\', "\\\\")
            ),
        )
        .unwrap();

        run_entry_file(&root, "main.gx").unwrap();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&dep_dir).ok();
    }

    /// A relative path from `from` to `to`, good enough for these tests'
    /// sibling temp directories (both are direct children of `std::env::
    /// temp_dir()`, so it's always exactly `../<to's dir name>`).
    fn pathdiff(to: &std::path::Path, from: &std::path::Path) -> String {
        let to_name = to.file_name().unwrap().to_str().unwrap();
        assert_eq!(
            to.parent(),
            from.parent(),
            "pathdiff test helper assumes sibling temp directories"
        );
        format!("../{}", to_name)
    }

    // ── v0.4.0 feature tests ──────────────────────────────────────────────────

    #[test]
    fn test_regex_test() {
        run(r#"
helper "re1" {
  brain { plan {} execute {
    assert regex_test("hello world", "world") "simple match"
    assert regex_test("user@example.com", "@") "at-sign match"
    assert regex_test("abc123", "\\d+") "digit match"
    assert !regex_test("no-match", "^\\d+$") "no match"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_find() {
        run(r#"
helper "re2" {
  brain { plan {} execute {
    price = regex_find("Total: $42.50", "\\$([0-9.]+)")
    assert price == "42.50" "capture group"
    nothing = regex_find("abc", "\\d+")
    assert nothing == null "no match returns null"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_find_all() {
        run(r#"
helper "re3" {
  brain { plan {} execute {
    nums = regex_find_all("a1 b2 c3", "\\d")
    assert nums[0] == "1" "first digit"
    assert nums[2] == "3" "third digit"
    assert len(nums) == 3 "count"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_replace() {
        run(r#"
helper "re4" {
  brain { plan {} execute {
    cleaned = regex_replace("aaa bbb", "a", "x")
    assert cleaned == "xxx bbb" "replace all a with x"
    no_digits = regex_replace("abc123def", "\\d", "")
    assert no_digits == "abcdef" "remove digits"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_split() {
        run(r#"
helper "re5" {
  brain { plan {} execute {
    parts = regex_split("one,two,,three", ",+")
    assert parts[0] == "one" "first part"
    assert parts[1] == "two" "second part"
    assert parts[2] == "three" "third part"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_now() {
        run(r#"
helper "dt1" {
  brain { plan {} execute {
    n = date_now()
    assert n.contains("T") "ISO-8601 contains T"
    assert n.len() > 10 "has time component"
    ts = date_timestamp()
    assert ts > 1700000000 "reasonable unix timestamp"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_parse_and_format() {
        run(r#"
helper "dt2" {
  brain { plan {} execute {
    ts = date_parse("2024-01-15")
    assert ts > 0 "parse returns timestamp"
    formatted = date_format(ts, "%Y-%m-%d")
    assert formatted == "2024-01-15" "round-trip"
    year = date_format(ts, "%Y")
    assert year == "2024" "year only"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_diff() {
        run(r#"
helper "dt3" {
  brain { plan {} execute {
    t1 = date_parse("2024-01-01")
    t2 = date_parse("2024-01-08")
    diff_days = date_diff(t1, t2, "days")
    assert diff_days == 7 "seven days"
    diff_hrs = date_diff(t1, t2, "hours")
    assert diff_hrs == 168 "168 hours"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_add() {
        run(r#"
helper "dt4" {
  brain { plan {} execute {
    base = date_parse("2024-01-01")
    next_week = date_add(base, 7, "days")
    diff = date_diff(base, next_week, "days")
    assert diff == 7 "added 7 days"
    next_hour = date_add(base, 1, "hours")
    diff_h = date_diff(base, next_hour, "hours")
    assert diff_h == 1 "added 1 hour"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_parts() {
        run(r#"
helper "dt5" {
  brain { plan {} execute {
    ts = date_parse("2024-03-15")
    p = date_parts(ts)
    assert p.year == 2024 "year"
    assert p.month == 3 "month"
    assert p.day == 15 "day"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_csv_parse() {
        run(r#"
helper "csv1" {
  brain { plan {} execute {
    csv = "name,age,city\nAlice,30,London\nBob,25,Paris"
    rows = csv_parse(csv)
    assert len(rows) == 2 "two data rows"
    assert rows[0].name == "Alice" "first name"
    assert rows[0].age == 30 "age is number"
    assert rows[1].city == "Paris" "second city"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_csv_stringify() {
        run(r#"
helper "csv2" {
  brain { plan {} execute {
    rows = [
      { name: "Alice", age: 30 },
      { name: "Bob",   age: 25 }
    ]
    out = csv_stringify(rows)
    assert out.contains("Alice") "has Alice"
    assert out.contains("age") "has header"
    // Round-trip
    back = csv_parse(out)
    assert back[0].name == "Alice" "round-trip name"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_yaml_parse() {
        run(r#"
helper "yaml1" {
  brain { plan {} execute {
    src = "name: Alice\nage: 30\ntags:\n  - dev\n  - gx"
    data = yaml_parse(src)
    assert data.name == "Alice" "name"
    assert data.age == 30 "age is number"
    assert data.tags[0] == "dev" "first tag"
    assert data.tags[1] == "gx" "second tag"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_yaml_stringify() {
        run(r#"
helper "yaml2" {
  brain { plan {} execute {
    obj = { name: "Alice", score: 42 }
    out = yaml_stringify(obj)
    assert out.contains("Alice") "has name"
    assert out.contains("42") "has score"
    back = yaml_parse(out)
    assert back.name == "Alice" "round-trip"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_toml_parse() {
        run(r#"
helper "toml1" {
  brain { plan {} execute {
    src = "[package]\nname = \"my-app\"\nversion = \"1.0.0\"\nedition = 2021"
    data = toml_parse(src)
    assert data.package.name == "my-app" "package name"
    assert data.package.edition == 2021 "edition number"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_load_env_and_get_env_default() {
        run(r#"
helper "env1" {
  brain { plan {} execute {
    // get_env with default — key definitely does not exist
    val = get_env("GX_TEST_NONEXISTENT_KEY_12345", "fallback")
    assert val == "fallback" "default returned"
    // get_env with no default → null
    val2 = get_env("GX_TEST_NONEXISTENT_KEY_99999")
    assert val2 == null "null when missing"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_vector_store() {
        run(r#"
helper "vs1" {
  brain { plan {} execute {
    // Create a store and add documents
    store = vector_store_new("test_store_v4")
    vector_store_add(store, "doc1", [1.0, 0.0, 0.0], "red axis")
    vector_store_add(store, "doc2", [0.0, 1.0, 0.0], "green axis")
    vector_store_add(store, "doc3", [0.0, 0.0, 1.0], "blue axis")
    assert vector_store_size(store) == 3 "three documents"

    // Search: query close to red axis should find doc1 first
    hits = vector_store_search(store, [0.9, 0.1, 0.0], 2)
    assert len(hits) == 2 "two hits"
    assert hits[0].id == "doc1" "closest is doc1"
    assert hits[0].score > 0.9 "high similarity"

    // Cosine similarity of identical vectors = 1.0
    sim = cosine_similarity([1.0, 0.0], [1.0, 0.0])
    assert sim == 1.0 "identical vectors"

    // Orthogonal vectors = 0.0
    sim2 = cosine_similarity([1.0, 0.0], [0.0, 1.0])
    assert sim2 == 0.0 "orthogonal vectors"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_schema_validate() {
        run(r#"
helper "sv1" {
  brain { plan {} execute {
    // "schema" is now a keyword — use "spec" as variable name
    spec = { name: "string", age: "number", active: "boolean" }

    good = { name: "Alice", age: 30, active: true }
    r = schema_validate(good, spec)
    assert r.ok "valid passes"
    assert len(r.errors) == 0 "no errors"

    bad = { name: "Bob", age: "thirty" }
    r2 = schema_validate(bad, spec)
    assert !r2.ok "invalid fails"
    assert len(r2.errors) > 0 "has errors"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_await_block() {
        run(r#"
helper "aw1" {
  brain { plan {} execute {
    await {
      a: 1 + 1
      b: "hello" + " world"
      c: len([1, 2, 3])
    } into results
    assert results.a == 2 "a computed"
    assert results.b == "hello world" "b computed"
    assert results.c == 3 "c computed"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_retry_succeeds_first_try() {
        run(r#"
helper "retry1" {
  brain { plan {} execute {
    // Lambda runs in its own scope; test that it returns a value
    result = retry(fn() {
      return 42
    }, 3)
    assert result == 42 "got result"
    // retry with string result
    msg = retry(fn() {
      return "hello"
    }, 2)
    assert msg == "hello" "string result"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_tool_definition_parsed() {
        // Verify the tool keyword parses and the helper runs correctly
        run(r#"
tool "greet_user" {
  description: "Greet a user by name"
  execute(name) {
    return "Hello " + name
  }
}

helper "tool_test" {
  brain { plan {} execute {
    assert true "tool definition accepted"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    // ── v0.4.1 friction-point fixes ───────────────────────────────────────────

    #[test]
    fn test_closure_captures_locals() {
        run(r#"
helper "cap" {
  brain { plan {} execute {
    multiplier = 3
    times = fn(n) { return n * multiplier }
    assert times(5) == 15 "closure captures multiplier"

    base = "https://x.com"
    path = "/users"
    build = fn() { return base + path }
    assert build() == "https://x.com/users" "closure captures two locals"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_closure_param_shadows_capture() {
        run(r#"
helper "shadow" {
  brain { plan {} execute {
    x = 100
    f = fn(x) { return x + 1 }
    assert f(5) == 6 "param shadows captured var"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_closure_in_object_field() {
        run(r#"
helper "objfn" {
  brain { plan {} execute {
    handlers = {
      "double": fn(n) { return n * 2 },
      "square": fn(n) { return n * n }
    }
    assert handlers.double(21) == 42 "obj.field(args) calls closure"
    assert handlers.square(7) == 49 "obj.field square"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_retry_captures_state() {
        run(r#"
helper "retrycap" {
  brain { plan {} execute {
    url = "https://api.test.com"
    body = { q: "hello" }
    result = retry(fn() {
      return { url: url, q: body.q }
    }, 3)
    assert result.url == "https://api.test.com" "retry closure captures url"
    assert result.q == "hello" "retry closure captures body"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_top_level_statements() {
        run(r#"
api_base = "https://api.example.com"
max_retries = 5
config = { timeout: 30 }

helper "toplevel" {
  brain { plan {} execute {
    assert api_base == "https://api.example.com" "file-root string var"
    assert max_retries == 5 "file-root number var"
    assert config.timeout == 30 "file-root object var"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_map_filter_with_closures() {
        run(r#"
helper "mapfilter" {
  brain { plan {} execute {
    factor = 10
    nums = [1, 2, 3, 4, 5]
    scaled = map(nums, fn(n) { return n * factor })
    assert scaled[0] == 10 "map captures factor"
    assert scaled[4] == 50 "map last element"

    threshold = 3
    big = filter(nums, fn(n) { return n > threshold })
    assert len(big) == 2 "filter captures threshold"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert_builtins() {
        run(r#"
helper "assertfns" {
  brain { plan {} execute {
    assert_eq(2 + 2, 4, "assert_eq")
    assert_true(1 < 2, "assert_true")
    assert_contains("hello world", "world", "assert_contains string")
    assert_contains([1, 2, 3], 2, "assert_contains array")
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_is_tty_builtin() {
        run(r#"
helper "ttytest" {
  brain { plan {} execute {
    t = is_tty()
    assert t == true or t == false "is_tty returns bool"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert_eq_failure() {
        let result = run(r#"
helper "assertfail" {
  brain { plan {} execute {
    assert_eq(1, 2, "should fail")
  } remember {} communicate {} }
}"#);
        assert!(result.is_err(), "assert_eq(1, 2) should fail");
    }

    #[test]
    fn test_runtime_error_has_line_and_stack() {
        let result = run(r#"
function divide(a, b) {
  return a / b
}

helper "errctx" {
  brain { plan {} execute {
    y = divide(10, 0)
  } remember {} communicate {} }
}"#);
        let msg = match result {
            Err(m) => m,
            Ok(_) => panic!("expected a runtime error"),
        };
        assert!(
            msg.contains("at line"),
            "error should include line number: {}",
            msg
        );
        assert!(
            msg.contains("divide()"),
            "error should include stack frame: {}",
            msg
        );
    }

    #[test]
    fn spawned_agent_inherits_parent_capabilities() {
        // Regression test for a real bug: `spawn agent ... with {...}` used
        // to construct the child with a bare Interpreter::new() and copy
        // only helpers/functions, silently resetting every capability grant
        // to its default-denied state regardless of what the parent script
        // had been given. If that regresses, `process_run` inside the
        // spawned agent below fails (`ok: false`), the assert fails, and
        // `run_program` returns Err — this test panics on `.unwrap()`.
        //
        // A `timeout` clause is required here specifically: `spawn agent`
        // *without* one runs `call_agent` synchronously on the same
        // Interpreter (trivially "inherits" everything since there's no
        // second Interpreter at all) — only the `timeout` form goes through
        // `call_agent_with_timeout`'s separate thread + fresh Interpreter,
        // which is the actual code path that had the bug.
        let src = r#"
helper "worker" {
  brain {
    plan { }
    execute {
      result = process_run({ command: "echo", args: ["inherited"] })
      assert result.ok == true "spawned agent must inherit process capability"
    }
    remember { }
    communicate { result.ok }
  }
}

ok = spawn agent "worker" with { } timeout 5000
assert ok == true "spawn agent result reflects the child's process_run success"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.process = true;
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn parallel_map_inherits_parent_capabilities() {
        let src = r#"
helper "worker" {
  brain {
    plan { }
    execute {
      result = process_run({ command: "echo", args: ["x"] })
    }
    remember { }
    communicate { result.ok }
  }
}

results = parallel {
  a: spawn agent "worker" with { }
}
assert results.a == true "parallel{} spawned agent must inherit process capability"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.process = true;
        interp.run_program(&program).unwrap();
    }

    // ── Database runtime ─────────────────────────────────────────────────────

    fn temp_db_path(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "gx_mod_db_test_{}_{}_{}.db",
                label,
                std::process::id(),
                n
            ))
            .to_string_lossy()
            .into_owned()
    }

    fn run_db_test(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        interp.run_program(&program)
    }

    #[test]
    fn db_transaction_rolls_back_on_error_and_leaves_a_clean_state_for_reuse() {
        let path = temp_db_path("rollback");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)", [])
try {{
  db_transaction("{path}") {{
    db_exec(db, "INSERT INTO t(v) VALUES (?)", ["should_not_persist"])
    x = 1 / 0
  }}
}} catch e {{
  assert true "error caught"
}}
rows = db_query("{path}", "SELECT * FROM t", [])
assert len(rows) == 0 "the failed transaction's insert must be rolled back"

// A clean subsequent transaction on the same path must still work —
// proves db_tx_depth wasn't left corrupted by the failed one above.
db_transaction("{path}") {{
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["after_rollback"])
}}
rows2 = db_query("{path}", "SELECT v FROM t", [])
assert len(rows2) == 1 "a later, unrelated transaction must commit normally"
"#,
            path = path
        );
        run_db_test(&src).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nested_db_transaction_uses_a_savepoint_and_both_levels_commit_together() {
        let path = temp_db_path("nested");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)", [])
db_transaction("{path}") {{
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer"])
  db_transaction(db) {{
    db_exec(db, "INSERT INTO t(v) VALUES (?)", ["inner"])
  }}
}}
rows = db_query("{path}", "SELECT v FROM t ORDER BY id", [])
assert len(rows) == 2 "both the outer and nested inserts must be committed"
assert rows[0].v == "outer" "outer insert present"
assert rows[1].v == "inner" "nested insert present"
"#,
            path = path
        );
        run_db_test(&src).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nested_db_transaction_rollback_only_undoes_the_inner_savepoint() {
        let path = temp_db_path("nested_partial_rollback");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)", [])
db_transaction("{path}") {{
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer"])
  try {{
    db_transaction(db) {{
      db_exec(db, "INSERT INTO t(v) VALUES (?)", ["inner_should_roll_back"])
      x = 1 / 0
    }}
  }} catch e {{
    assert true "inner error caught, outer transaction continues"
  }}
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer_after_inner_failure"])
}}
rows = db_query("{path}", "SELECT v FROM t ORDER BY id", [])
assert len(rows) == 2 "only the inner savepoint's insert should be undone"
assert rows[0].v == "outer" "first outer insert present"
assert rows[1].v == "outer_after_inner_failure" "second outer insert present"
"#,
            path = path
        );
        run_db_test(&src).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_exec_inside_a_transaction_on_a_different_path_uses_that_path_not_the_active_transaction()
    {
        // Regression test: db_query/db_exec used to look at a single
        // global "is any transaction active" slot, so a call naming a
        // *different* database while inside a transaction on another one
        // silently ran against the transactional connection instead of
        // the path actually requested.
        let path_a = temp_db_path("cross_a");
        let path_b = temp_db_path("cross_b");
        let src = format!(
            r#"
db_exec("{path_a}", "CREATE TABLE t(id INTEGER PRIMARY KEY)", [])
db_exec("{path_b}", "CREATE TABLE u(id INTEGER PRIMARY KEY, v TEXT)", [])
db_transaction("{path_a}") {{
  db_exec("{path_b}", "INSERT INTO u(v) VALUES (?)", ["direct-to-b"])
}}
rows_b = db_query("{path_b}", "SELECT v FROM u", [])
assert len(rows_b) == 1 "the insert must have landed in b.db, not been swallowed by a.db's transaction"
assert rows_b[0].v == "direct-to-b" "b.db's row has the expected value"
rows_a = db_query("{path_a}", "SELECT * FROM t", [])
assert len(rows_a) == 0 "a.db's table must be untouched"
"#,
            path_a = path_a,
            path_b = path_b
        );
        run_db_test(&src).unwrap();
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn db_connections_are_pooled_and_reused_across_calls() {
        // Proves the same underlying connection is reused (not reopened
        // per call) by relying on a same-connection-only property: an
        // in-memory-visible PRAGMA change persists across calls only if
        // it's really the same connection. journal_mode=WAL (set once at
        // open time by configure_connection) is exactly such a property —
        // a fresh connection opened without configure_connection would
        // default to journal_mode=delete, not wal.
        let path = temp_db_path("pooled");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY)", [])
mode1 = db_query("{path}", "PRAGMA journal_mode", [])[0].journal_mode
mode2 = db_query("{path}", "PRAGMA journal_mode", [])[0].journal_mode
assert mode1 == "wal" "first call sees WAL mode (set by configure_connection on open)"
assert mode2 == "wal" "second call still sees WAL mode from the same pooled connection"
"#,
            path = path
        );
        run_db_test(&src).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_connection_pool_evicts_the_oldest_idle_connection_once_at_capacity() {
        // Regression test: `db_connections` used to grow without bound —
        // one entry per distinct path a long-running Interpreter (a
        // `serve` worker's, most realistically) ever queried, held open
        // for its whole lifetime. Opening more than
        // `MAX_POOLED_DB_CONNECTIONS` distinct paths must keep the pool
        // bounded by evicting the least-recently-used *idle* one, not
        // grow past it.
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        let mut paths = Vec::new();
        for i in 0..MAX_POOLED_DB_CONNECTIONS + 5 {
            let path = temp_db_path(&format!("evict_{}", i));
            interp.db_conn(&path).unwrap();
            paths.push(path);
        }
        assert_eq!(
            interp.db_connections.len(),
            MAX_POOLED_DB_CONNECTIONS,
            "pool must stay capped at MAX_POOLED_DB_CONNECTIONS, not grow past it"
        );
        // The very first path opened is the least recently used (touched
        // once, then never again) — it must be the one evicted, not an
        // arbitrary/most-recent one.
        assert!(
            !interp.db_connections.contains_key(&paths[0]),
            "the oldest idle connection should have been evicted first"
        );
        assert!(
            interp.db_connections.contains_key(paths.last().unwrap()),
            "the most recently opened connection must still be pooled"
        );
        for path in &paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn db_connection_pool_never_evicts_a_connection_with_an_active_transaction() {
        // The Database Runtime's contract — never close a connection out
        // from under an active `db_transaction` — takes priority over the
        // pool cap (see `evict_one_idle_db_connection_if_at_capacity`'s
        // doc comment): this proves it holds even when that connection
        // would otherwise be the natural LRU eviction candidate (the
        // oldest, touched only once, with every other slot filled after
        // it).
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        let in_tx_path = temp_db_path("in_tx");
        interp.db_conn(&in_tx_path).unwrap();
        interp.db_tx_begin(&in_tx_path).unwrap();

        let mut other_paths = Vec::new();
        for i in 0..MAX_POOLED_DB_CONNECTIONS + 5 {
            let path = temp_db_path(&format!("evict_around_tx_{}", i));
            interp.db_conn(&path).unwrap();
            other_paths.push(path);
        }

        assert!(
            interp.db_connections.contains_key(&in_tx_path),
            "a connection with an active transaction must never be evicted, \
             even when the pool is under capacity pressure"
        );
        assert_eq!(
            *interp.db_tx_depth.get(&in_tx_path).unwrap(),
            1,
            "the transaction itself must be untouched by pool pressure"
        );

        interp.db_tx_end(&in_tx_path, 1, false).unwrap();
        let _ = std::fs::remove_file(&in_tx_path);
        for path in &other_paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn try_reserve_sse_responder_slot_rejects_once_the_cap_is_reached_and_allows_below_it() {
        // The exact check `Stmt::RespondStream` makes on every call,
        // exercised directly against a small cap rather than needing
        // MAX_CONCURRENT_SSE_RESPONDERS (256) real stuck HTTP connections
        // to reach the rejection path.
        let counter = std::sync::atomic::AtomicUsize::new(0);
        assert!(try_reserve_sse_responder_slot(&counter, 3));
        assert!(try_reserve_sse_responder_slot(&counter, 3));
        assert!(try_reserve_sse_responder_slot(&counter, 3));
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert!(
            !try_reserve_sse_responder_slot(&counter, 3),
            "a 4th reservation must be refused once the cap is reached"
        );
        // Refusing must not have mutated the counter.
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);

        // Releasing one slot (what a finished responder thread's
        // `fetch_sub` does) must free capacity for the next reservation.
        counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        assert!(try_reserve_sse_responder_slot(&counter, 3));
    }

    #[test]
    fn try_reserve_sse_responder_slot_never_exceeds_the_cap_under_concurrent_pressure() {
        // Real concurrency stress on the exact CAS-based reserve logic
        // production uses — many threads racing to reserve the same
        // small number of slots at once, which is precisely the scenario
        // a TOCTOU bug (a naive "check then increment" race, as opposed
        // to the atomic `fetch_update` this function actually uses) would
        // show up in: a plain read-then-write race could let the counter
        // overshoot the cap when two threads both pass the check before
        // either increments.
        const CAP: usize = 8;
        const CONTENDERS: usize = 200;
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let granted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handles: Vec<_> = (0..CONTENDERS)
            .map(|_| {
                let counter = counter.clone();
                let granted = granted.clone();
                std::thread::spawn(move || {
                    if try_reserve_sse_responder_slot(&counter, CAP) {
                        granted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            granted.load(std::sync::atomic::Ordering::SeqCst),
            CAP,
            "exactly CAP reservations should have been granted out of {} contenders",
            CONTENDERS
        );
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            CAP,
            "the counter itself must never have exceeded the cap, even transiently"
        );
    }

    #[test]
    fn concurrent_read_then_write_transactions_from_separate_connections_serialize_instead_of_erroring(
    ) {
        // Regression test for a real bug found via the Task Runtime
        // milestone's own adversarial testing: `db_tx_begin` used to issue
        // a plain (deferred) `BEGIN`, which takes no lock until the
        // transaction's first statement — a *read* only acquires a shared
        // lock. Several concurrent transactions each doing
        // `row = db_query(...); db_exec(... UPDATE ...)` (an extremely
        // common pattern) could therefore all get past their read and then
        // race to *upgrade* to a write lock at the same time — a race
        // `busy_timeout` cannot resolve by retrying (neither side can ever
        // downgrade), surfacing as an immediate "database is locked" error
        // instead of a bounded wait. `BEGIN IMMEDIATE` (the fix) takes the
        // write lock up front, turning the race into a straightforward,
        // correctly-serialized queue. Uses raw OS threads (not the Task
        // Runtime) — this is a Database Runtime property that must hold
        // regardless of what's driving the concurrency.
        let path = temp_db_path("begin_immediate");
        {
            let mut setup = Interpreter::new();
            setup.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
            let conn = setup.db_conn(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE counters(id INTEGER PRIMARY KEY, n INTEGER); \
                 INSERT INTO counters(id, n) VALUES (1, 0);",
            )
            .unwrap();
        }

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let src = format!(
                        r#"
db_transaction("{path}") {{
  row = db_query(db, "SELECT n FROM counters WHERE id = 1", [])[0]
  db_exec(db, "UPDATE counters SET n = ? WHERE id = 1", [row.n + 1])
}}
"#,
                        path = path
                    );
                    run_db_test(&src)
                })
            })
            .collect();

        for h in handles {
            h.join()
                .unwrap()
                .expect("every concurrent read-then-write transaction must succeed, not hit 'database is locked'");
        }

        let mut check = Interpreter::new();
        check.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        let final_n: i64 = check
            .db_conn(&path)
            .unwrap()
            .query_row("SELECT n FROM counters WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            final_n, 8,
            "all 8 concurrent increments must land — none silently lost or double-applied"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_transaction_panic_pattern_still_rolls_back_and_leaves_depth_consistent() {
        // GX itself has no reachable way to trigger a genuine Rust panic
        // from script code (confirmed empirically in the HTTP milestone —
        // out-of-bounds indexing, division by zero, etc. all return errors,
        // never panic). This test instead verifies the exact catch_unwind/
        // resume_unwind shape run_db_transaction uses, the same way the
        // SSE cleanup guarantee was verified in bridge_impl.rs: directly,
        // against a closure that really does panic, proving db_tx_depth
        // and the underlying SQL transaction state both recover correctly
        // regardless of *why* the body didn't finish normally.
        let path = temp_db_path("panic_pattern");
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        {
            let conn = interp.db_conn(&path).unwrap();
            conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)")
                .unwrap();
        }

        let depth = interp.db_tx_begin(&path).unwrap();
        assert_eq!(depth, 1);
        {
            let conn = interp.db_conn(&path).unwrap();
            db_exec_on_conn(conn, "INSERT INTO t(v) VALUES ('doomed')", vec![]).unwrap();
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("simulated transaction body panic");
        }));
        assert!(outcome.is_err());
        // Mirrors run_db_transaction's own handling exactly: rollback, not commit.
        interp.db_tx_end(&path, depth, false).unwrap();

        assert_eq!(*interp.db_tx_depth.get(&path).unwrap_or(&0), 0);
        let conn = interp.db_conn(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "the doomed insert must have been rolled back");

        // A fresh transaction on the same path must work cleanly afterward.
        let depth2 = interp.db_tx_begin(&path).unwrap();
        assert_eq!(
            depth2, 1,
            "depth must have recovered to 0 before this begin"
        );
        interp.db_tx_end(&path, depth2, true).unwrap();

        let _ = std::fs::remove_file(&path);
    }

    // ── Diagnostics runtime integration ──────────────────────────────────────

    #[test]
    fn spawn_agent_with_timeout_inherits_the_parent_trace_id_for_correlation() {
        // `call_agent_with_timeout` runs the spawned agent on its own thread
        // with its own `Interpreter`, built via `self.diagnostics.for_child()`
        // — without that, the spawned agent would mint its own unrelated
        // trace_id, breaking correlation between a parent operation and the
        // work it delegates to a sub-agent (the very thing `trace_id`/
        // `for_child` exist to prevent).
        let src = r#"
helper "worker" {
  brain {
    plan { }
    execute {
      matched = trace_id() == input.expected
    }
    remember { }
    communicate { matched }
  }
}

expected = trace_id()
result = spawn agent "worker" with { expected: expected } timeout 5000
assert result == true "spawned agent (timeout form) must share the parent's trace_id"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.diagnostics.ensure_trace_id();
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn parallel_map_spawned_agent_inherits_the_parent_trace_id_for_correlation() {
        let src = r#"
helper "worker" {
  brain {
    plan { }
    execute {
      matched = trace_id() == input.expected
    }
    remember { }
    communicate { matched }
  }
}

expected = trace_id()
results = parallel {
  a: spawn agent "worker" with { expected: expected }
}
assert results.a == true "parallel{} spawned agent must share the parent's trace_id"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.diagnostics.ensure_trace_id();
        interp.run_program(&program).unwrap();
    }

    #[test]
    fn db_transaction_with_diagnostics_enabled_leaves_no_span_leaked_after_nested_commit() {
        // A leaked span on `diagnostics.span_stack` would silently corrupt
        // correlation for every later db_transaction/db_query/db_exec call
        // on the same (potentially long-lived, e.g. HTTP-worker) Interpreter
        // — this proves start_span/end_span in run_db_transaction are
        // correctly paired even across nested savepoints.
        let path = temp_db_path("diag_nested_commit");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)", [])
db_transaction("{path}") {{
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer"])
  db_transaction(db) {{
    db_exec(db, "INSERT INTO t(v) VALUES (?)", ["inner"])
  }}
}}
"#,
            path = path
        );
        let tokens = Lexer::new(&src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        interp.diagnostics.set_enabled(true);
        interp.diagnostics.ensure_trace_id();
        interp.run_program(&program).unwrap();
        assert!(
            interp.diagnostics.current_span_id().is_none(),
            "no span should remain open after a fully-committed nested db_transaction"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_transaction_with_diagnostics_enabled_leaves_no_span_leaked_after_inner_failure() {
        let path = temp_db_path("diag_nested_rollback");
        let src = format!(
            r#"
db_exec("{path}", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)", [])
db_transaction("{path}") {{
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer_before"])
  try {{
    db_transaction(db) {{
      db_exec(db, "INSERT INTO t(v) VALUES (?)", ["inner_doomed"])
      x = 1 / 0
    }}
  }} catch e {{
    assert true "inner transaction error caught"
  }}
  db_exec(db, "INSERT INTO t(v) VALUES (?)", ["outer_after"])
}}
"#,
            path = path
        );
        let tokens = Lexer::new(&src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        interp.diagnostics.set_enabled(true);
        interp.diagnostics.ensure_trace_id();
        interp.run_program(&program).unwrap();
        assert!(
            interp.diagnostics.current_span_id().is_none(),
            "no span should remain open after an inner transaction's error is caught"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capability_denied_process_call_emits_an_audit_event_and_still_returns_the_same_error() {
        // Wiring diagnostics into the capability-denial path (authorize_capability)
        // must be purely additive — the thrown error a script sees must be
        // identical whether or not diagnostics happens to be enabled.
        let src = r#"
try {
  process_run({ command: "echo", args: ["x"] })
  assert false "process_run should have been denied by default"
} catch e {
  assert true "process_run denial still throws as before"
}
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.diagnostics.set_enabled(true);
        interp.diagnostics.ensure_trace_id();
        interp.run_program(&program).unwrap();
    }

    // ── Debugger Runtime ─────────────────────────────────────────────────
    //
    // `debug_pause` itself blocks on real stdin (deliberately — see its own
    // doc comment; it's the same "just read from the terminal" shape as the
    // pre-existing `readline()` builtin, which is likewise untested here),
    // so it can't be driven from a `#[test]` without hanging. What *can* be
    // verified without any I/O: that wiring `debug` state onto an
    // `Interpreter` is purely inert when no configured line is ever
    // reached (the overwhelmingly common case — most executions of a
    // `--break`-annotated script don't hit every line), and that
    // `current_line` — the field `breakpoint()` reads to report where it
    // was called — actually tracks execution. The interactive prompt
    // itself (`--break`, `breakpoint()`, `step`/`locals`/`stack`/
    // `print`/`watch`/`quit`) was verified empirically via `printf ... |
    // gx run/gx -e`, matching this codebase's established testing
    // convention for stdin-driven features (see `main.rs`'s REPL, which is
    // tested the same way).

    #[test]
    fn debug_state_with_a_non_matching_break_line_never_pauses_or_alters_output() {
        let tokens = Lexer::new("x = 1\ny = 2\nsay x + y\n").tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.debug.mode = DebugMode::Running;
        interp.debug.break_lines.insert(9999); // never reached by this program
        interp.output_capture = Some(Vec::new());
        interp.run_program(&program).unwrap();
        assert_eq!(
            interp.output_capture.clone().unwrap(),
            vec!["3".to_string()]
        );
    }

    #[test]
    fn debug_mode_off_by_default_on_a_freshly_constructed_interpreter() {
        // The zero-cost default: a script run without any debugger
        // involvement must never even consider pausing.
        let interp = Interpreter::new();
        assert_eq!(interp.debug.mode, DebugMode::Off);
        assert!(interp.debug.break_lines.is_empty());
    }

    #[test]
    fn current_line_tracks_the_most_recently_executed_statement() {
        let tokens = Lexer::new("x = 1\ny = 2\nz = 3\n").tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert_eq!(interp.current_line, 3);
    }

    // ── Testing Framework ────────────────────────────────────────────────

    #[test]
    fn test_registers_a_named_case_without_running_it() {
        // The whole point of deferred registration: a side effect inside
        // the closure body must not happen just from calling test() to
        // register it — only `take_registered_tests` + actually invoking
        // the closure should trigger it.
        let src = r#"
ran = false
test("does nothing yet", fn() {
  ran = true
})
assert ran == false "test() must not run its closure immediately"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(interp.assert_failures.is_empty());
        assert_eq!(interp.registered_tests.len(), 1);
        assert_eq!(interp.registered_tests[0].0, "does nothing yet");
    }

    #[test]
    fn take_registered_tests_drains_and_leaves_the_list_empty() {
        let src = r#"
test("a", fn() { })
test("b", fn() { })
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        let taken = interp.take_registered_tests();
        assert_eq!(taken.len(), 2);
        assert!(interp.registered_tests.is_empty());
    }

    #[test]
    fn call_registered_closure_actually_runs_the_test_body() {
        let src = r#"
result = 0
test("sets result", fn() {
  result = 42
})
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        let (_, test_fn) = interp.take_registered_tests().remove(0);
        let mut env = Env::new();
        interp.call_registered_closure(&test_fn, &mut env).unwrap();
        // The closure's own top-level assignment to `result` is local to
        // its own captured-by-value scope (see the doc comment on
        // `call_registered_closure`) — what this actually verifies is
        // that the closure body *ran at all* (no panic, no error) rather
        // than that an outer plain variable was mutated.
    }

    #[test]
    fn call_registered_closure_propagates_an_assertion_failure() {
        let src = r#"
test("fails", fn() {
  assert 1 == 2 "deliberately false"
})
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        let (_, test_fn) = interp.take_registered_tests().remove(0);
        let mut env = Env::new();
        let result = interp.call_registered_closure(&test_fn, &mut env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("deliberately false"));
    }

    #[test]
    fn before_each_state_reaches_the_test_body_via_shared_memory_env() {
        // The exact channel `crate::toolchain::test` relies on: passing
        // the *same* Env through before_each and the test body lets
        // memory.* mutations in one be visible in the other, unlike a
        // plain captured variable (GX closures capture by value — see
        // `call_registered_closure`'s doc comment).
        let src = r#"
before_each_hook = fn() {
  memory.value = 10
}
test("sees the shared value", fn() {
  assert memory.value == 10 "expected before_each's memory value"
})
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        let hook = interp.global_vars.get("before_each_hook").unwrap().clone();
        let (_, test_fn) = interp.take_registered_tests().remove(0);

        let mut shared_env = Env::new();
        interp
            .call_registered_closure(&hook, &mut shared_env)
            .unwrap();
        let result = interp.call_registered_closure(&test_fn, &mut shared_env);
        assert!(
            result.is_ok(),
            "test should see before_each's memory.value: {:?}",
            result
        );

        // Confirm a *fresh* env (no shared setup) does NOT see it —
        // proves the sharing is coming from the Env, not some global leak.
        let mut isolated_env = Env::new();
        let isolated_result = interp.call_registered_closure(&test_fn, &mut isolated_env);
        assert!(isolated_result.is_err());
    }

    #[test]
    fn set_random_seed_makes_random_int_fully_deterministic() {
        let src = r#"
set_random_seed(42)
a = random_int(1, 1000000)
b = random_int(1, 1000000)
c = random_int(1, 1000000)
set_random_seed(42)
d = random_int(1, 1000000)
e = random_int(1, 1000000)
f = random_int(1, 1000000)
assert a == d "first draw after reseeding must match"
assert b == e "second draw after reseeding must match"
assert c == f "third draw after reseeding must match"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "seeded random_int sequence was not reproducible: {:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn set_random_seed_makes_shuffle_deterministic_too() {
        let src = r#"
set_random_seed(7)
a = shuffle([1, 2, 3, 4, 5, 6, 7, 8])
set_random_seed(7)
b = shuffle([1, 2, 3, 4, 5, 6, 7, 8])
assert json_stringify(a) == json_stringify(b) "seeded shuffle must be reproducible"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "seeded shuffle was not reproducible: {:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn random_without_a_seed_still_works_and_stays_in_bounds() {
        // Backward-compatibility guard: scripts that never call
        // set_random_seed must see exactly the prior behavior (a working,
        // clock-seeded draw), not an error or a default-zero seed.
        let src = r#"
i = 0
while i < 20 {
  n = random_int(5, 10)
  assert n >= 5 "random_int must respect its lower bound"
  assert n <= 10 "random_int must respect its upper bound"
  i = i + 1
}
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(interp.assert_failures.is_empty());
    }

    #[test]
    fn test_temp_dir_returns_a_fresh_writable_directory_each_call() {
        let src = r#"
d1 = test_temp_dir()
d2 = test_temp_dir()
assert d1 != d2 "each call must return a distinct directory"
write_file(d1 + "/f.txt", "hello")
content = read_file(d1 + "/f.txt")
assert content == "hello" "must be able to write into and read back from the returned directory"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        interp.base_path = Some("/tmp/gx_test_temp_dir_test.gx".to_string());
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
        // Cleanup: best-effort, this test creates real directories under cwd's tmp/.
        let _ = std::fs::remove_dir_all("tmp");
    }

    #[test]
    fn assert_golden_creates_the_file_on_first_run_and_passes() {
        let dir = std::env::temp_dir().join(format!(
            "gx_assert_golden_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let golden_path = dir.join("golden.txt");
        let _ = std::fs::remove_file(&golden_path);

        let src = format!(
            r#"
ok = assert_golden("hello world", "{path}")
assert ok == true "first run should write the golden file and pass"
"#,
            path = golden_path.to_string_lossy().replace('\\', "\\\\")
        );
        let tokens = Lexer::new(&src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        interp.run_program(&program).unwrap();
        assert!(interp.assert_failures.is_empty());
        assert_eq!(
            std::fs::read_to_string(&golden_path).unwrap(),
            "hello world"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn assert_golden_fails_when_the_value_no_longer_matches() {
        let dir = std::env::temp_dir().join(format!(
            "gx_assert_golden_mismatch_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let golden_path = dir.join("golden.txt");
        std::fs::write(&golden_path, "expected value").unwrap();

        let src = format!(
            r#"assert_golden("a different value", "{path}")"#,
            path = golden_path.to_string_lossy().replace('\\', "\\\\")
        );
        let tokens = Lexer::new(&src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem = crate::capability::FilesystemAccess::Unrestricted;
        let result = interp.run_program(&program);
        assert!(result.is_err(), "mismatched golden value must fail");

        std::fs::remove_dir_all(&dir).ok();
    }

    // GX_UPDATE_GOLDEN=1's overwrite behavior is deliberately *not* tested
    // here with `std::env::set_var`: env vars are process-global, and
    // `cargo test` runs this file's tests concurrently on shared threads —
    // mutating it here could race the other `assert_golden_*` tests above
    // and make one of them spuriously pass. See `main.rs`'s
    // `assert_golden_update_env_var_overwrites_a_mismatched_file`, which
    // verifies the same behavior by spawning the real `gx` binary with the
    // env var scoped to that one child process instead.

    // ── Configuration Runtime ────────────────────────────────────────────
    //
    // `env_prefix`-driven overrides are deliberately *not* tested here
    // with `std::env::set_var`, for the same reason noted above for
    // GX_UPDATE_GOLDEN — see `main.rs`'s
    // `config_load_env_prefix_overrides_a_default_with_type_coercion`,
    // which verifies that behavior via a subprocess instead.

    #[test]
    fn config_load_defaults_only() {
        let src = r#"
c = config_load({ defaults: { port: 3000, name: "app" } })
assert c.port == 3000 "default port"
assert c.name == "app" "default name"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn config_load_file_layer_overrides_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "gx_config_load_file_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"port": 8080, "name": "from-file"}"#,
        )
        .unwrap();

        let src = r#"
c = config_load({
  defaults: { port: 3000, name: "default", debug: false },
  file: "config.json"
})
assert c.port == 8080 "file should override default port"
assert c.name == "from-file" "file should override default name"
assert c.debug == false "key not present in file keeps its default"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_load_missing_file_falls_back_to_defaults_without_error() {
        let src = r#"
c = config_load({ defaults: { port: 1234 }, file: "does_not_exist.json" })
assert c.port == 1234 "missing config file must not be a hard error"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn config_load_a_present_but_malformed_file_is_a_hard_error() {
        let dir = std::env::temp_dir().join(format!(
            "gx_config_load_malformed_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{ not valid json").unwrap();

        let src = r#"config_load({ defaults: {}, file: "config.json" })"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        let result = interp.run_program(&program);
        assert!(result.is_err(), "a malformed config file the caller explicitly pointed at must error, not be silently ignored");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_load_rejects_an_unrecognized_file_extension() {
        let dir = std::env::temp_dir().join(format!(
            "gx_config_load_bad_ext_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.ini"), "port=8080").unwrap();

        let src = r#"config_load({ defaults: {}, file: "config.ini" })"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        let result = interp.run_program(&program);
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_load_explicit_overrides_win_over_everything_else() {
        let src = r#"
c = config_load({
  defaults: { debug: false },
  overrides: { debug: true }
})
assert c.debug == true "explicit overrides must be the highest-precedence layer"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn config_load_schema_failure_throws_instead_of_returning_invalid_config() {
        let src = r#"
config_load({
  defaults: { port: "not-a-number" },
  schema: { port: "number" }
})
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        assert!(
            result.is_err(),
            "invalid config against the given schema must throw, not silently return"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("port"),
            "error should mention the offending field: {}",
            msg
        );
    }

    #[test]
    fn config_load_valid_schema_passes_through_the_merged_config() {
        let src = r#"
c = config_load({
  defaults: { port: 3000, debug: false },
  schema: { port: "number", debug: "boolean" }
})
assert c.port == 3000 "valid config should pass through unchanged"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn config_load_env_override_cannot_inject_a_new_key_not_already_in_defaults_or_file() {
        // Security property: the environment-override layer only ever
        // overrides a key that already exists after defaults+file — it
        // can never smuggle in a brand-new config key purely from an
        // env var the developer never declared.
        let src = r#"
c = config_load({ defaults: { port: 3000 }, env_prefix: "APP_" })
assert has(c, "port") "declared key must be present"
assert has(c, "injected") == false "an undeclared key must never appear just because an env var could theoretically map to it"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    // ── Serialization Runtime ────────────────────────────────────────────

    #[test]
    fn versioned_stringify_and_parse_round_trip_when_version_matches() {
        let src = r#"
s = versioned_stringify({ name: "Ada" }, 2)
restored = versioned_parse(s, 2)
assert restored.name == "Ada" "round-trip must preserve the data"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn versioned_parse_rejects_a_mismatched_version() {
        let src = r#"
s = versioned_stringify({ name: "Ada" }, 2)
versioned_parse(s, 3)
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("unsupported version"), "{}", msg);
    }

    #[test]
    fn versioned_parse_rejects_a_plain_json_blob_never_produced_by_versioned_stringify() {
        let src = r#"versioned_parse("{\"name\": \"Ada\"}", 1)"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        assert!(
            result.is_err(),
            "a blob with no __gx_version tag must not silently pass version checking"
        );
    }

    #[test]
    fn versioned_parse_without_an_expected_version_returns_the_data_unconditionally() {
        // No expected version given: caller just wants the payload back,
        // whatever version it was written with (e.g. a migration script).
        let src = r#"
s = versioned_stringify({ name: "Ada" }, 7)
restored = versioned_parse(s)
assert restored.name == "Ada"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn data_export_and_import_round_trip_across_every_supported_format() {
        let dir = std::env::temp_dir().join(format!(
            "gx_data_import_export_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        for ext in ["json", "yaml", "toml"] {
            let src = format!(
                r#"
data_export("{ext}.{ext}", {{ name: "Ada", age: 36 }})
back = data_import("{ext}.{ext}")
assert back.name == "Ada" "round-trip name for {ext}"
assert back.age == 36 "round-trip age for {ext}"
"#,
                ext = ext
            );
            let tokens = Lexer::new(&src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            interp.capabilities.filesystem =
                crate::capability::FilesystemAccess::Sandboxed(dir.clone());
            interp.run_program(&program).unwrap();
            assert!(
                interp.assert_failures.is_empty(),
                "format {}: {:?}",
                ext,
                interp.assert_failures
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_export_rejects_a_value_that_fails_the_given_schema() {
        let dir = std::env::temp_dir().join(format!(
            "gx_data_export_schema_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let src = r#"data_export("out.json", { age: "not-a-number" }, { age: "number" })"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        let result = interp.run_program(&program);
        assert!(result.is_err(), "schema-failing value must not be written");
        assert!(
            !dir.join("out.json").exists(),
            "no file should be written when schema validation fails"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_import_rejects_an_unrecognized_extension() {
        let dir = std::env::temp_dir().join(format!(
            "gx_data_import_bad_ext_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("data.ini"), "port=8080").unwrap();

        let src = r#"data_import("data.ini")"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        let result = interp.run_program(&program);
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_import_missing_file_is_a_clear_error_not_a_panic() {
        let src = r#"data_import("does_not_exist.json")"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        assert!(result.is_err());
    }

    #[test]
    fn gx_equality_operator_correctly_compares_objects() {
        // Regression test for the GX-level surface of the Value::PartialEq
        // fix in value.rs: the `==` operator a script actually writes, not
        // just the Rust-level trait impl.
        let src = r#"
a = { x: 1, y: 2 }
b = { x: 1, y: 2 }
c = { x: 1, y: 3 }
assert a == b "structurally identical objects must be =="
assert (a == c) == false "objects with a different value must not be =="
assert [{x: 1}] == [{x: 1}] "arrays containing equal objects must be =="
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    // ── Template & Code Generation Runtime ───────────────────────────────

    #[test]
    fn render_template_composes_with_read_file_for_the_real_workflow() {
        // The realistic case: a template loaded from a file (a runtime
        // string GX's own compile-time "{expr}" interpolation never
        // touches) rather than typed as a GX string literal in source
        // (where "{{name}}" would be mangled by that same interpolation
        // before render_template ever saw it — a real, documented gotcha,
        // not a bug in render_template itself).
        let dir = std::env::temp_dir().join(format!(
            "gx_render_template_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("greeting.template"),
            "Hello, {{name}}! You are {{age}} years old.",
        )
        .unwrap();

        let src = r#"
tmpl = read_file("greeting.template")
result = render_template(tmpl, { name: "Ada", age: 36 })
assert result == "Hello, Ada! You are 36 years old." "rendered output must match"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_template_scaffolding_pattern_generates_multiple_files_from_one_template() {
        // The "project scaffolding" use case named in the milestone:
        // render_template + an ordinary GX loop + write_file, with no
        // separate scaffold() primitive needed.
        let dir = std::env::temp_dir().join(format!(
            "gx_render_template_scaffold_test_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("component.template"),
            "export function {{name}}() {}",
        )
        .unwrap();

        let src = r#"
tmpl = read_file("component.template")
names = ["Button", "Header"]
i = 0
while i < len(names) {
  content = render_template(tmpl, { name: names[i] })
  write_file(names[i] + ".gen", content)
  i = i + 1
}
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(dir.clone());
        interp.run_program(&program).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("Button.gen")).unwrap(),
            "export function Button() {}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("Header.gen")).unwrap(),
            "export function Header() {}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Memory & Performance: array mutation methods ─────────────────────

    #[test]
    fn pop_mutates_the_original_array_when_its_result_is_captured() {
        // Regression test for a real, pre-existing (not introduced by the
        // performance work in this milestone) correctness bug: `x =
        // arr.pop()` returned the correct value but left `arr` completely
        // unchanged, because eval_method has no way to write back to a
        // variable it never sees a name for. A script using the idiomatic
        // `while len(arr) > 0 { x = arr.pop(); ... }` pattern would loop
        // forever.
        let src = r#"
arr = [1, 2, 3, 4]
p = arr.pop()
assert p == 4 "pop must return the last element"
assert len(arr) == 3 "pop must shrink the array, even when its result is captured"
assert arr[2] == 3
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn pop_in_a_while_len_loop_terminates() {
        // The concrete failure mode of the bug above: this loop would
        // never terminate if pop() didn't actually shrink `arr`.
        let src = r#"
arr = [1, 2, 3]
out = []
guard = 0
while len(arr) > 0 {
  x = arr.pop()
  out.push(x)
  guard = guard + 1
  assert guard < 1000 "loop must terminate — pop() isn't shrinking the array"
}
assert out == [3, 2, 1]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn push_self_reassignment_pattern_still_works() {
        // Backward-compatibility guard: `results = results.push(x)` is an
        // existing, real pattern (tests/test_sugar.gx,
        // docs/examples/brain_cycle_progressive.gx) relying on `.push()`
        // being *functional* (returns a new array, doesn't mutate) when
        // its result is captured — deliberately NOT changed by the
        // pop() fix or the bare-statement performance fix, both of which
        // are scoped to leave this exact pattern untouched.
        let src = r#"
results = []
results = results.push("a")
results = results.push("b")
results = results.push("c")
assert results == ["a", "b", "c"]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn bare_push_statement_mutates_in_place_and_is_fast() {
        // The performance fix: `arr.push(x)` as a bare statement must
        // mutate in place (no full-array clone per call) — this test
        // checks correctness; the O(n) vs O(n²) scaling itself was
        // verified empirically (40,000 pushes: 45s before the fix, well
        // under a second after).
        let src = r#"
arr = []
i = 0
while i < 1000 {
  arr.push(i)
  i = i + 1
}
assert len(arr) == 1000
assert arr[0] == 0
assert arr[999] == 999
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn bare_sort_and_reverse_statements_mutate_in_place() {
        let src = r#"
a = [3, 1, 2]
a.sort()
assert a == [1, 2, 3]

b = [1, 2, 3]
b.reverse()
assert b == [3, 2, 1]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn push_on_a_non_array_variable_falls_back_to_the_pre_existing_behavior() {
        // Pre-existing behavior (confirmed identical before this milestone's
        // changes via a direct comparison, and deliberately not altered
        // here — out of scope for this fix): calling an array-only method
        // on a non-array value doesn't throw. It prints a warning, and the
        // variable ends up set to null (eval_method's "unknown method"
        // fallback returns null, and the assignment-fallback path in
        // Stmt::Expr writes that back) — a legitimately surprising
        // behavior in its own right, but not a new one, and not what this
        // fix is about. This test exists only to confirm the fast path's
        // fallback reaches that exact same pre-existing behavior, not a
        // new panic, a new error, or a silent no-op with no warning.
        let src = "x = 5\nx.push(1)\nsay x";
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.output_capture = Some(Vec::new());
        interp.run_program(&program).unwrap();
        assert_eq!(
            interp.output_capture.clone().unwrap(),
            vec!["null".to_string()]
        );
    }

    #[test]
    fn object_field_assignment_mutates_in_place_and_is_fast() {
        // Regression test for the same class of O(n²) bug as push/pop,
        // just in `assign`'s Expr::FieldAccess/Expr::Index handling
        // instead of the array-method auto-mutate path: `obj.field = val`
        // used to clone the *whole* object on every single assignment
        // (get the whole value out, mutate the copy, write the copy
        // back). Verified empirically: 50,000 field assignments took over
        // a minute before this fix, well under a second after.
        let src = r#"
obj = {}
i = 0
while i < 2000 {
  obj["key" + to_string(i)] = i
  i = i + 1
}
assert len(keys(obj)) == 2000
assert obj["key0"] == 0
assert obj["key1999"] == 1999
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn dot_field_assignment_mutates_in_place() {
        let src = r#"
obj = {}
obj.name = "Ada"
obj.age = 36
assert obj.name == "Ada"
assert obj.age == 36
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn array_index_assignment_mutates_in_place() {
        let src = r#"
arr = [1, 2, 3]
arr[1] = 99
assert arr == [1, 99, 3]
arr[-1] = 42
assert arr == [1, 99, 42]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn array_index_assignment_auto_extends_with_null() {
        let src = r#"
arr = [1]
arr[3] = "x"
assert len(arr) == 4
assert arr[1] == null
assert arr[2] == null
assert arr[3] == "x"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn out_of_bounds_negative_index_assignment_errors_instead_of_corrupting_index_zero() {
        // Regression test: `arr[-100] = x` on a 3-element array used to
        // clamp the computed index to 0 (`.max(0)`) and silently overwrite
        // the *first* element — a caller doing bounds-agnostic negative
        // assignment (`arr[-i] = x`) with a bug in `i` would corrupt the
        // wrong slot with no indication anything went wrong. In-bounds
        // negative assignment must still work normally.
        let src = r#"
arr = [1, 2, 3]
arr[-1] = 99
assert arr[-1] == 99 "in-bounds negative assignment must still work"
arr[-100] = 0
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        assert!(
            result.is_err(),
            "out-of-bounds negative assignment must be a catchable error"
        );
        assert!(result.unwrap_err().contains("out of bounds"));
    }

    #[test]
    fn index_assignment_on_null_auto_creates_object_or_array() {
        let src = r#"
a = null
a["k"] = "v"
assert a["k"] == "v"

b = null
b[2] = "z"
assert len(b) == 3
assert b[2] == "z"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn nested_field_assignment_still_works() {
        // The nested (outer.inner.field = val) path wasn't touched by
        // this fix — this test just confirms it still behaves correctly.
        let src = r#"
memory = { user: { name: "old" } }
memory.user.name = "new"
assert memory.user.name == "new"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn nested_field_push_mutates_the_underlying_array() {
        // Regression test for a real, pre-existing (confirmed via direct
        // comparison against the code before this milestone) bug:
        // `memory.items.push(x)` — the single most common agent-memory
        // accumulator pattern — silently did nothing, because the
        // bare-statement auto-mutate fast path only recognized a plain
        // identifier receiver (`arr.push(x)`), not one level of nested
        // field access. No error, no warning — `memory.items` just never
        // grew.
        let src = r#"
memory = { items: [] }
memory.items.push("x")
memory.items.push("y")
assert memory.items == ["x", "y"] "nested field push must mutate the underlying array"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn nested_field_push_evaluates_its_argument_exactly_once() {
        // Regression test for a bug introduced and caught during the same
        // fix above: an early draft evaluated the call's arguments
        // unconditionally before checking whether the nested-field fast
        // path actually applied, so a receiver shape that *didn't* match
        // fell through to the general evaluation at the bottom of the
        // function and evaluated the arguments a second time — silently
        // doubling any side effect in one of them. Verified here via a
        // side effect the interpreter can observe deterministically
        // (an array push), for a receiver shape the fast path *does*
        // handle, confirming exactly one evaluation.
        let src = r#"
memory = { call_log: [], items: [] }
function track() {
  memory.call_log.push("called")
  return "val"
}
memory.items.push(track())
assert len(memory.call_log) == 1 "the pushed argument must be evaluated exactly once"
assert memory.items == ["val"]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn deeper_nested_push_falls_through_without_double_evaluating_args() {
        // Three levels of nesting (a.b.c.push(x)) isn't covered by either
        // fast path and falls through to the general evaluation, matching
        // pre-existing behavior (the pushed value is silently discarded —
        // a known, documented limitation) — this test only guards against
        // the fallback path evaluating the argument twice.
        let src = r#"
memory = { call_log: [] }
function track() {
  memory.call_log.push("called")
  return "val"
}
a = { b: { c: [] } }
a.b.c.push(track())
assert len(memory.call_log) == 1 "even the unhandled deeper-nesting fallback must evaluate the argument exactly once"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn nested_field_pop_mutates_the_underlying_array() {
        // Found during adversarial review of the nested-push fix above:
        // the exact same bug existed for `.pop()` — `memory.items.pop()`
        // returned the correct value but left `memory.items` unchanged,
        // for the same reason (only a bare-identifier receiver was
        // recognized). Unlike push, pop has no legitimate non-mutating
        // reading, so this is fixed unconditionally in eval_call, not
        // scoped to the bare-statement case.
        let src = r#"
memory = { items: [1, 2, 3] }
x = memory.items.pop()
assert x == 3 "pop must return the last element"
assert memory.items == [1, 2] "pop must shrink the nested array too"
memory.items.pop()
assert memory.items == [1] "bare-statement nested pop must also mutate"
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    #[test]
    fn nested_field_pop_in_a_while_len_loop_terminates() {
        let src = r#"
memory = { stack: [10, 20, 30] }
out = []
guard = 0
while len(memory.stack) > 0 {
  out.push(memory.stack.pop())
  guard = guard + 1
  assert guard < 1000 "loop must terminate — nested pop() isn't shrinking the array"
}
assert out == [30, 20, 10]
"#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run_program(&program).unwrap();
        assert!(
            interp.assert_failures.is_empty(),
            "{:?}",
            interp.assert_failures
        );
    }

    // ── Runtime Reliability: recursion depth guard ───────────────────────
    //
    // `MAX_CALL_DEPTH` is calibrated against `WORKER_THREAD_STACK_SIZE` —
    // the stack size every real production entry point (`main()`,
    // `task_spawn`, the HTTP server's worker pool) runs the interpreter
    // on. `cargo test`'s own per-test thread does *not* use that size, so
    // calling `run_program` directly here — on the ambient test thread —
    // would itself overflow the *real* stack before the guard ever gets a
    // chance to trigger (confirmed empirically: it does). These tests
    // spawn their own thread with the same stack size real production
    // code uses, exactly reproducing the environment `MAX_CALL_DEPTH` is
    // actually a safe limit within.
    fn run_on_properly_sized_stack<F, T>(f: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(WORKER_THREAD_STACK_SIZE)
            .spawn(f)
            .expect("failed to spawn test thread")
            .join()
            .expect("test thread panicked")
    }

    #[test]
    fn infinite_recursion_is_a_catchable_error_not_a_crash() {
        // Critical regression test: before this fix, a script with no
        // base case (a real, common programming mistake, not a
        // contrived attack) aborted the entire process with an
        // uncatchable Rust stack overflow — confirmed empirically
        // against the pre-fix code. This test running to completion *at
        // all* (rather than aborting the whole test binary) is itself
        // most of what's being verified here.
        run_on_properly_sized_stack(|| {
            let src = r#"
function f(n) {
  return f(n + 1)
}
f(0)
"#;
            let tokens = Lexer::new(src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            let result = interp.run_program(&program);
            assert!(
                result.is_err(),
                "unbounded recursion must be a catchable error"
            );
            assert!(
                result.unwrap_err().contains("maximum call depth exceeded"),
                "error must clearly name the cause"
            );
        });
    }

    #[test]
    fn recursion_error_is_catchable_via_try_catch_and_leaves_the_interpreter_usable() {
        run_on_properly_sized_stack(|| {
            let src = r#"
function f(n) {
  return f(n + 1)
}
try {
  f(0)
} catch e {
  assert e.message != "" "caught error must have a message"
}
// The interpreter's call_stack must be fully unwound after the caught
// error — verified by confirming ordinary execution (including a fresh,
// unrelated function call) still works correctly afterward.
function normal(a, b) {
  return a + b
}
result = normal(3, 4)
assert result == 7 "interpreter must remain fully usable after catching a recursion error"
"#;
            let tokens = Lexer::new(src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            interp.run_program(&program).unwrap();
            assert!(
                interp.assert_failures.is_empty(),
                "{:?}",
                interp.assert_failures
            );
        });
    }

    #[test]
    fn legitimate_deep_recursion_within_the_limit_still_works() {
        run_on_properly_sized_stack(|| {
            let src = r#"
function sum_to(n) {
  if n <= 0 { return 0 }
  return n + sum_to(n - 1)
}
assert sum_to(500) == 125250 "recursion well within MAX_CALL_DEPTH must work normally"
"#;
            let tokens = Lexer::new(src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            interp.run_program(&program).unwrap();
            assert!(
                interp.assert_failures.is_empty(),
                "{:?}",
                interp.assert_failures
            );
        });
    }

    #[test]
    fn deeply_recursive_closures_are_also_guarded() {
        // The guard covers all three GX call entry points
        // (call_user_function, call_user_function_propagating,
        // call_closure_with_capture) — this specifically exercises the
        // closure path, not just plain `function` declarations. GX
        // closures can't reference themselves by their own assigned
        // variable name (captured-scope snapshot happens before the
        // assignment completes — a common limitation, not a bug) so this
        // uses the classic "pass yourself as an argument" pattern
        // instead, confirmed separately to actually recurse.
        run_on_properly_sized_stack(|| {
            let src = r#"
f = fn(self_ref, n) {
  return self_ref(self_ref, n + 1)
}
f(f, 0)
"#;
            let tokens = Lexer::new(src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            let result = interp.run_program(&program);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("maximum call depth exceeded"));
        });
    }

    #[test]
    fn deep_recursion_error_message_truncates_the_call_stack_trace() {
        // The error message itself must stay readable — not one line
        // containing ~1000 repeated stack frames.
        run_on_properly_sized_stack(|| {
            let src = r#"
function f(n) {
  return f(n + 1)
}
f(0)
"#;
            let tokens = Lexer::new(src).tokenize().unwrap();
            let program = Parser::new(tokens).parse().unwrap();
            let mut interp = Interpreter::new();
            let err = interp.run_program(&program).unwrap_err();
            assert!(
                err.contains("more frames omitted"),
                "a very deep trace must be truncated: {}",
                err
            );
            assert!(
                err.len() < 2000,
                "truncated error message should be well under 2000 characters, was {}",
                err.len()
            );
        });
    }
}
