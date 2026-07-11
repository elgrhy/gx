//! The AI Context Runtime: `context_create`, `context_set_system`,
//! `context_add_message`, `context_add_tool_result`, `context_ask`,
//! `context_trim`, `context_summarize_and_trim`, `context_clone`,
//! `context_reset`, `context_serialize`, `context_deserialize`,
//! `context_stats`.
//!
//! This is the runtime responsible for managing AI conversation state —
//! message history, token budgeting, trimming, and prompt assembly — for
//! every GX application. It is deliberately *not* an AI framework: it does
//! not decide when to call a model, which tools to invoke, or how to
//! summarize a conversation. It provides the primitives every such policy
//! is built from, the same way the Task Runtime provides `task_spawn`
//! without deciding what any given task should do.
//!
//! ## A context is plain data, not a handle into a registry
//!
//! Every other stateful GX runtime (Process, Task) wraps a real external
//! resource — an OS process, a background thread — behind an opaque handle
//! string, because that resource can't be represented as ordinary data and
//! must be tracked so it's never orphaned. An AI context wraps none of
//! that: it's a system prompt, an array of messages, and some token-budget
//! configuration — pure, already-`Clone`, already-serializable data. Making
//! it a plain `Value::Object` instead of a handle-plus-registry gets four
//! of the milestone's required primitives for free, with zero code:
//!
//! - **Context isolation**: GX values have no shared mutable state, so a
//!   context passed into a `task_spawn`/`spawn agent` closure is already an
//!   independent copy — mutating it inside the task can never affect the
//!   caller's copy.
//! - **Context inheritance**: passing a context as a task/agent input
//!   argument (`task_spawn(fn() { ... }, ...)` closures capture it,
//!   `spawn agent "x" with { ctx: my_context }`) already hands the child a
//!   full snapshot of the conversation so far — no separate "propagate
//!   context into a task" mechanism was needed; it's the same closure
//!   capture the Task Runtime already has.
//! - **Context cloning**: `ctx2 = ctx` already deep-copies. `context_clone`
//!   (below) exists anyway, as an explicit, discoverable primitive that
//!   additionally stamps a fresh context id and records the lineage —
//!   useful for diagnostics when branching a conversation into two
//!   continuations.
//! - **Context serialization/persistence**: a context is already
//!   `json_stringify`-able. `context_serialize`/`context_deserialize`
//!   exist as a *versioned* pair (see `CONTEXT_VERSION`) so a context saved
//!   before a future schema change fails loudly instead of loading
//!   silently-wrong. Persistence itself is just `db_exec`/`db_query` on the
//!   resulting string — no new Database Runtime API needed.
//!
//! This is why this module — unlike `builtins_process`/`builtins_task` —
//! adds no new field to `Interpreter` at all.

use super::Signal;
use crate::value::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use crate::ai;

/// Bumped only if the context object's *shape* changes in a way that would
/// make an older serialized context misread under a newer GX — checked by
/// `context_deserialize` so loading a stale/foreign JSON blob fails with a
/// clear error instead of silently producing a malformed context.
const CONTEXT_VERSION: f64 = 1.0;

const DEFAULT_MAX_HISTORY_TOKENS: f64 = 8000.0;
const DEFAULT_RESERVE_TOKENS: f64 = 1000.0;
/// A hard ceiling independent of the (heuristic, not exact) token estimate
/// — defense in depth against a pathological case such as thousands of
/// tiny messages that individually estimate to very few tokens but
/// collectively bloat memory and request size.
const DEFAULT_MAX_MESSAGES: f64 = 200.0;
const DEFAULT_TRIM_STRATEGY: &str = "drop_oldest_pair";
/// Cap on any single tool-result message's content — "tool output limits":
/// an uncapped shell/HTTP tool result is one of the most common real
/// sources of prompt-explosion, distinct from ordinary conversation growth.
const DEFAULT_TOOL_OUTPUT_MAX_CHARS: f64 = 8000.0;
const DEFAULT_RESPONSE_MAX_TOKENS: f64 = 1024.0;
/// A hard, non-configurable backstop on any single `user`/`assistant`
/// message's content — deliberately much larger than
/// `tool_output_max_chars` (real conversational content deserves a
/// generous allowance) but not infinite: without this, one accidentally
/// enormous message (a pasted document, a runaway generation fed back in)
/// could exceed the *entire* token budget by itself, and `apply_trim` can
/// only remove *other* messages — it can't fix a single message that's
/// already bigger than the whole budget on its own.
const MAX_MESSAGE_CONTENT_CHARS: usize = 100_000;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Same ~4-chars-per-token heuristic as the `token_count()` builtin
/// (`interpreter/mod.rs`) — deliberately not shared via a common function:
/// it's a one-line formula, and duplicating it here keeps this module
/// self-contained the same way every other `builtins_*.rs` file is,
/// without introducing a cross-file dependency for something this trivial.
fn estimate_tokens(s: &str) -> f64 {
    ((s.chars().count() as f64) / 4.0).ceil()
}

fn new_context_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn require_context<'a>(v: &'a Value, who: &str) -> Result<&'a HashMap<String, Value>, Signal> {
    match v {
        Value::Object(m) if m.contains_key("__gx_context_version") => Ok(m),
        _ => Err(Signal::Error(format!(
            "{}: expected a context (from context_create/context_deserialize), got {}",
            who,
            v.type_name()
        ))),
    }
}

fn get_str(m: &HashMap<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn get_num(m: &HashMap<String, Value>, key: &str, default: f64) -> f64 {
    m.get(key).and_then(|v| v.as_number()).unwrap_or(default)
}

fn get_messages(m: &HashMap<String, Value>) -> Vec<Value> {
    match m.get("messages") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn valid_trim_strategy(s: &str) -> bool {
    matches!(s, "none" | "drop_oldest" | "drop_oldest_pair" | "summarize")
}

/// Build one message object and compute (and cache) its token estimate.
/// Applies `tool_output_max_chars` truncation for `role == "tool"` only —
/// truncating a user's or the model's own words would silently corrupt the
/// conversation; truncating a shell/HTTP tool's output is exactly the
/// "tool output limits" this runtime exists to enforce.
#[allow(clippy::too_many_arguments)]
fn build_message(
    role: &str,
    content: &str,
    tool_call_id: Option<String>,
    name: Option<String>,
    tool_calls: Option<Value>,
    tool_output_max_chars: usize,
) -> Value {
    // Tool output gets the configurable, role-specific cap (usually the
    // tightest — tool output is the single most common source of an
    // accidentally-enormous message). Every other role still gets a hard,
    // non-configurable backstop: without one, a single oversized
    // user/assistant message (an accidentally-pasted document, a runaway
    // generation) could blow straight past the whole token budget in one
    // add — `apply_trim` can only remove *other* messages, so an
    // over-budget single message would leave the context permanently
    // over_budget with nothing left to trim. Same truncate-and-flag
    // convention as tool output, not a hard rejection, so a script never
    // has to special-case "message too large" as its own error class.
    let effective_cap = if role == "tool" {
        tool_output_max_chars
    } else {
        tool_output_max_chars.max(MAX_MESSAGE_CONTENT_CHARS)
    };
    let (content, truncated) = if content.chars().count() > effective_cap {
        let truncated_content: String = content.chars().take(effective_cap).collect();
        (truncated_content, true)
    } else {
        (content.to_string(), false)
    };
    let mut m = HashMap::new();
    m.insert("role".into(), Value::Str(role.to_string()));
    m.insert(
        "estimated_tokens".into(),
        Value::Number(estimate_tokens(&content)),
    );
    m.insert("content".into(), Value::Str(content));
    m.insert(
        "tool_call_id".into(),
        tool_call_id.map(Value::Str).unwrap_or(Value::Null),
    );
    m.insert("name".into(), name.map(Value::Str).unwrap_or(Value::Null));
    m.insert("tool_calls".into(), tool_calls.unwrap_or(Value::Null));
    m.insert("truncated".into(), Value::Bool(truncated));
    Value::Object(m)
}

fn message_tokens(msg: &Value) -> f64 {
    match msg {
        Value::Object(m) => get_num(m, "estimated_tokens", 0.0),
        _ => 0.0,
    }
}

fn total_estimated_tokens(m: &HashMap<String, Value>) -> f64 {
    let system_tokens = get_str(m, "system")
        .map(|s| estimate_tokens(&s))
        .unwrap_or(0.0);
    let messages_tokens: f64 = get_messages(m).iter().map(message_tokens).sum();
    system_tokens + messages_tokens
}

fn trim_threshold(m: &HashMap<String, Value>) -> f64 {
    let max_history = get_num(m, "max_history_tokens", DEFAULT_MAX_HISTORY_TOKENS);
    let reserve = get_num(m, "reserve_tokens", DEFAULT_RESERVE_TOKENS);
    (max_history - reserve).max(0.0)
}

fn needs_trim(m: &HashMap<String, Value>) -> bool {
    let max_messages = get_num(m, "max_messages", DEFAULT_MAX_MESSAGES) as usize;
    get_messages(m).len() > max_messages || total_estimated_tokens(m) > trim_threshold(m)
}

/// Apply `ctx`'s configured trim strategy in place until it's back under
/// budget (or there's nothing left to drop). Returns how many messages
/// were removed. `"summarize"` deliberately behaves exactly like
/// `"drop_oldest_pair"` here — see the module doc and
/// `context_summarize_and_trim` for why actually generating a summary is
/// not something this automatic path can or should do itself.
fn apply_trim(m: &mut HashMap<String, Value>) -> usize {
    let strategy = get_str(m, "trim_strategy").unwrap_or_else(|| DEFAULT_TRIM_STRATEGY.into());
    if strategy == "none" {
        return 0;
    }
    let mut messages = get_messages(m);
    let mut removed = 0usize;
    let step = if strategy == "drop_oldest" { 1 } else { 2 };
    while !messages.is_empty() && needs_trim(m) {
        let n = step.min(messages.len());
        messages.drain(0..n);
        removed += n;
        m.insert("messages".into(), Value::Array(messages.clone()));
    }
    removed
}

fn record_trim(m: &mut HashMap<String, Value>, removed: usize) {
    if removed == 0 {
        return;
    }
    let mut stats = match m.get("stats") {
        Some(Value::Object(s)) => s.clone(),
        _ => HashMap::new(),
    };
    let prior = get_num(&stats, "total_messages_trimmed", 0.0);
    stats.insert(
        "total_messages_trimmed".into(),
        Value::Number(prior + removed as f64),
    );
    m.insert("stats".into(), Value::Object(stats));
}

struct CreateOpts {
    system: Option<String>,
    max_history_tokens: f64,
    reserve_tokens: f64,
    max_messages: f64,
    trim_strategy: String,
    tool_output_max_chars: f64,
}

fn parse_create_opts(v: Option<&Value>) -> Result<CreateOpts, Signal> {
    let Some(Value::Object(m)) = v else {
        return Ok(CreateOpts {
            system: None,
            max_history_tokens: DEFAULT_MAX_HISTORY_TOKENS,
            reserve_tokens: DEFAULT_RESERVE_TOKENS,
            max_messages: DEFAULT_MAX_MESSAGES,
            trim_strategy: DEFAULT_TRIM_STRATEGY.to_string(),
            tool_output_max_chars: DEFAULT_TOOL_OUTPUT_MAX_CHARS,
        });
    };
    let trim_strategy = get_str(m, "trim_strategy").unwrap_or_else(|| DEFAULT_TRIM_STRATEGY.into());
    if !valid_trim_strategy(&trim_strategy) {
        return Err(Signal::Error(format!(
            "context_create: 'trim_strategy' must be one of none/drop_oldest/drop_oldest_pair/summarize, got '{}'",
            trim_strategy
        )));
    }
    let max_history_tokens = get_num(m, "max_history_tokens", DEFAULT_MAX_HISTORY_TOKENS);
    let reserve_tokens = get_num(m, "reserve_tokens", DEFAULT_RESERVE_TOKENS);
    // A `reserve_tokens` at or above `max_history_tokens` clamps the actual
    // trim threshold to 0 — every message would be trimmed the instant it's
    // added, with no error and no obvious symptom beyond "the context
    // always looks empty". Rejecting this at creation time (the same
    // fail-fast treatment `trim_strategy` gets above) surfaces a
    // misconfiguration immediately instead of as a silent, confusing
    // runtime behavior — this is a real case that came up while testing a
    // custom `max_history_tokens` without also adjusting the default
    // `reserve_tokens`.
    if reserve_tokens >= max_history_tokens {
        return Err(Signal::Error(format!(
            "context_create: 'reserve_tokens' ({}) must be less than 'max_history_tokens' ({}) — \
             otherwise every message would be trimmed the instant it's added",
            reserve_tokens, max_history_tokens
        )));
    }
    Ok(CreateOpts {
        system: get_str(m, "system"),
        max_history_tokens,
        reserve_tokens,
        max_messages: get_num(m, "max_messages", DEFAULT_MAX_MESSAGES),
        trim_strategy,
        tool_output_max_chars: get_num(m, "tool_output_max_chars", DEFAULT_TOOL_OUTPUT_MAX_CHARS),
    })
}

fn fresh_stats() -> Value {
    let mut s = HashMap::new();
    s.insert("created_at".into(), Value::Number(now_unix_ms() as f64));
    s.insert("ask_count".into(), Value::Number(0.0));
    s.insert("total_tokens_used".into(), Value::Number(0.0));
    s.insert("total_messages_added".into(), Value::Number(0.0));
    s.insert("total_messages_trimmed".into(), Value::Number(0.0));
    s.insert("last_ask_at".into(), Value::Null);
    Value::Object(s)
}

impl super::Interpreter {
    pub(super) fn context_create(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let opts = parse_create_opts(args.first())?;
        let mut m = HashMap::new();
        m.insert(
            "__gx_context_version".into(),
            Value::Number(CONTEXT_VERSION),
        );
        m.insert("__gx_context_id".into(), Value::Str(new_context_id()));
        m.insert(
            "system".into(),
            opts.system.map(Value::Str).unwrap_or(Value::Null),
        );
        m.insert("messages".into(), Value::Array(Vec::new()));
        m.insert(
            "max_history_tokens".into(),
            Value::Number(opts.max_history_tokens),
        );
        m.insert("reserve_tokens".into(), Value::Number(opts.reserve_tokens));
        m.insert("max_messages".into(), Value::Number(opts.max_messages));
        m.insert("trim_strategy".into(), Value::Str(opts.trim_strategy));
        m.insert(
            "tool_output_max_chars".into(),
            Value::Number(opts.tool_output_max_chars),
        );
        m.insert("stats".into(), fresh_stats());
        Ok(Value::Object(m))
    }

    pub(super) fn context_set_system(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_set_system")?;
        let text = args
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_set_system: expected a string".into()))?;
        let mut m = ctx.clone();
        m.insert("system".into(), Value::Str(text.to_string()));
        Ok(Value::Object(m))
    }

    pub(super) fn context_add_message(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_add_message")?;
        let role = args
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_add_message: expected a role string".into()))?;
        if !matches!(role, "user" | "assistant" | "tool") {
            return Err(Signal::Error(format!(
                "context_add_message: role must be 'user', 'assistant', or 'tool' — use \
                 context_set_system for the system prompt (got '{}')",
                role
            )));
        }
        let content = args
            .get(2)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_add_message: expected content string".into()))?
            .to_string();
        let opts = args.get(3);
        let tool_call_id = opts.and_then(|v| match v {
            Value::Object(o) => get_str(o, "tool_call_id"),
            _ => None,
        });
        let name = opts.and_then(|v| match v {
            Value::Object(o) => get_str(o, "name"),
            _ => None,
        });
        let tool_calls = opts.and_then(|v| match v {
            Value::Object(o) => o.get("tool_calls").cloned(),
            _ => None,
        });
        if role == "tool" && tool_call_id.is_none() {
            return Err(Signal::Error(
                "context_add_message: role 'tool' requires opts.tool_call_id".into(),
            ));
        }

        let mut m = ctx.clone();
        let tool_output_max_chars =
            get_num(&m, "tool_output_max_chars", DEFAULT_TOOL_OUTPUT_MAX_CHARS) as usize;
        let msg = build_message(
            role,
            &content,
            tool_call_id,
            name,
            tool_calls,
            tool_output_max_chars,
        );

        let mut messages = get_messages(&m);
        messages.push(msg);
        m.insert("messages".into(), Value::Array(messages));

        let mut stats = match m.get("stats") {
            Some(Value::Object(s)) => s.clone(),
            _ => HashMap::new(),
        };
        let prior = get_num(&stats, "total_messages_added", 0.0);
        stats.insert("total_messages_added".into(), Value::Number(prior + 1.0));
        m.insert("stats".into(), Value::Object(stats));

        let removed = apply_trim(&mut m);
        record_trim(&mut m, removed);

        Ok(Value::Object(m))
    }

    pub(super) fn context_add_tool_result(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let tool_call_id = args
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_add_tool_result: expected tool_call_id".into()))?
            .to_string();
        let name = args
            .get(2)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_add_tool_result: expected tool name".into()))?
            .to_string();
        let content = args
            .get(3)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_add_tool_result: expected content".into()))?
            .to_string();
        let mut opts = HashMap::new();
        opts.insert("tool_call_id".into(), Value::Str(tool_call_id));
        opts.insert("name".into(), Value::Str(name));
        self.context_add_message(&[
            args.first().cloned().unwrap_or(Value::Null),
            Value::Str("tool".into()),
            Value::Str(content),
            Value::Object(opts),
        ])
    }

    pub(super) fn context_trim(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_trim")?;
        let mut m = ctx.clone();
        if let Some(Value::Object(opts)) = args.get(1) {
            if let Some(strategy) = get_str(opts, "strategy") {
                if !valid_trim_strategy(&strategy) {
                    return Err(Signal::Error(format!(
                        "context_trim: 'strategy' must be one of none/drop_oldest/drop_oldest_pair/summarize, got '{}'",
                        strategy
                    )));
                }
                m.insert("trim_strategy".into(), Value::Str(strategy));
            }
        }
        // A manual trim forces at least one pass even if `needs_trim` is
        // already false for "none" strategy contexts calling this
        // explicitly — but "none" still means "never auto-trim", so honor
        // it here too rather than special-casing an explicit call.
        let removed = apply_trim(&mut m);
        record_trim(&mut m, removed);
        Ok(Value::Object(m))
    }

    /// Replace the oldest messages with a single caller-supplied summary
    /// message. This is the actual "automatic summarization hook": GX
    /// provides the mechanical replace-N-messages-with-one operation and
    /// tracks it in `stats`/diagnostics; deciding *when* to call this and
    /// *how* to produce `summary_text` (almost always itself an AI call,
    /// e.g. via `context_ask`/`ask`) is application policy, not something
    /// this runtime can build in — see the module doc for why.
    pub(super) fn context_summarize_and_trim(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(
            args.first().unwrap_or(&Value::Null),
            "context_summarize_and_trim",
        )?;
        let summary_text = args
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Signal::Error("context_summarize_and_trim: expected a summary string".into())
            })?
            .to_string();
        let keep_last = args
            .get(2)
            .and_then(|v| match v {
                Value::Object(o) => o.get("keep_last").and_then(|v| v.as_number()),
                _ => None,
            })
            .unwrap_or(0.0) as usize;

        let mut m = ctx.clone();
        let mut messages = get_messages(&m);
        let keep_from = messages.len().saturating_sub(keep_last);
        let kept: Vec<Value> = messages.split_off(keep_from);
        let removed = messages.len();

        let mut new_messages = Vec::with_capacity(kept.len() + 1);
        new_messages.push(build_message(
            "assistant",
            &summary_text,
            None,
            Some("summary".to_string()),
            None,
            usize::MAX,
        ));
        new_messages.extend(kept);
        m.insert("messages".into(), Value::Array(new_messages));
        record_trim(&mut m, removed);
        Ok(Value::Object(m))
    }

    pub(super) fn context_clone(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_clone")?;
        let original_id = get_str(ctx, "__gx_context_id");
        let mut m = ctx.clone();
        m.insert("__gx_context_id".into(), Value::Str(new_context_id()));
        let mut stats = match m.get("stats") {
            Some(Value::Object(s)) => s.clone(),
            _ => HashMap::new(),
        };
        stats.insert(
            "forked_from".into(),
            original_id.map(Value::Str).unwrap_or(Value::Null),
        );
        m.insert("stats".into(), Value::Object(stats));
        Ok(Value::Object(m))
    }

    pub(super) fn context_reset(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_reset")?;
        let keep_system = match args.get(1) {
            Some(Value::Object(o)) => o.get("keep_system").map(|v| v.is_truthy()).unwrap_or(true),
            _ => true,
        };
        let mut m = ctx.clone();
        m.insert("__gx_context_id".into(), Value::Str(new_context_id()));
        if !keep_system {
            m.insert("system".into(), Value::Null);
        }
        m.insert("messages".into(), Value::Array(Vec::new()));
        m.insert("stats".into(), fresh_stats());
        Ok(Value::Object(m))
    }

    pub(super) fn context_serialize(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_serialize")?;
        let json = super::gx_value_to_json(&Value::Object(ctx.clone()));
        serde_json::to_string(&json)
            .map(Value::Str)
            .map_err(|e| Signal::Error(format!("context_serialize: {}", e)))
    }

    pub(super) fn context_deserialize(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let s = args
            .first()
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_deserialize: expected a string".into()))?;
        let json: serde_json::Value = serde_json::from_str(s)
            .map_err(|e| Signal::Error(format!("context_deserialize: invalid JSON: {}", e)))?;
        let val = super::json_to_gx_value(&json);
        let ctx = require_context(&val, "context_deserialize")?;
        let version = get_num(ctx, "__gx_context_version", 0.0);
        if version != CONTEXT_VERSION {
            return Err(Signal::Error(format!(
                "context_deserialize: unsupported context version {} (this GX supports version {})",
                version, CONTEXT_VERSION
            )));
        }
        Ok(val)
    }

    pub(super) fn context_stats(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_stats")?;
        let stats = match ctx.get("stats") {
            Some(Value::Object(s)) => s.clone(),
            _ => HashMap::new(),
        };
        let mut out = HashMap::new();
        out.insert(
            "context_id".into(),
            ctx.get("__gx_context_id").cloned().unwrap_or(Value::Null),
        );
        out.insert(
            "message_count".into(),
            Value::Number(get_messages(ctx).len() as f64),
        );
        out.insert(
            "estimated_tokens".into(),
            Value::Number(total_estimated_tokens(ctx)),
        );
        out.insert(
            "system_present".into(),
            Value::Bool(get_str(ctx, "system").is_some()),
        );
        out.insert(
            "max_history_tokens".into(),
            Value::Number(get_num(
                ctx,
                "max_history_tokens",
                DEFAULT_MAX_HISTORY_TOKENS,
            )),
        );
        out.insert("over_budget".into(), Value::Bool(needs_trim(ctx)));
        out.insert(
            "trim_strategy".into(),
            ctx.get("trim_strategy").cloned().unwrap_or(Value::Null),
        );
        out.insert(
            "ask_count".into(),
            Value::Number(get_num(&stats, "ask_count", 0.0)),
        );
        out.insert(
            "total_tokens_used".into(),
            Value::Number(get_num(&stats, "total_tokens_used", 0.0)),
        );
        out.insert(
            "total_messages_added".into(),
            Value::Number(get_num(&stats, "total_messages_added", 0.0)),
        );
        out.insert(
            "total_messages_trimmed".into(),
            Value::Number(get_num(&stats, "total_messages_trimmed", 0.0)),
        );
        out.insert(
            "created_at".into(),
            stats.get("created_at").cloned().unwrap_or(Value::Null),
        );
        out.insert(
            "last_ask_at".into(),
            stats.get("last_ask_at").cloned().unwrap_or(Value::Null),
        );
        Ok(Value::Object(out))
    }

    /// The main integration point: assemble `ctx` into a provider call,
    /// authorize + trace it exactly like the single-shot `ask` primitive
    /// does, then return the AI response *and* the updated context (the
    /// assistant's reply already appended, auto-trimmed if configured).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn context_ask(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let ctx = require_context(args.first().unwrap_or(&Value::Null), "context_ask")?.clone();
        let provider = args
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Signal::Error("context_ask: expected a provider string".into()))?
            .to_string();
        let opts = match args.get(2) {
            Some(Value::Object(o)) => o.clone(),
            _ => HashMap::new(),
        };
        let model = get_str(&opts, "model");

        self.authorize_ai_provider(&provider)?;

        // Ollama's multi-turn chat endpoint (`/api/chat`) isn't wired up
        // yet — only its single-shot `/api/generate` path is (via the
        // `messages` param `ask_ai` doesn't forward to `ask_ollama` at
        // all). Without this check, `context_ask(ctx, "ollama", ...)`
        // would silently send an *empty* prompt (the flat `prompt` param
        // `context_ask` never sets) instead of the conversation — a real
        // bug caught in adversarial review, not a hypothetical one. Fail
        // clearly instead of silently degrading.
        if provider == "ollama" {
            let mut out = HashMap::new();
            out.insert("ok".into(), Value::Bool(false));
            out.insert("text".into(), Value::Str(String::new()));
            out.insert(
                "error".into(),
                Value::Str(
                    "context_ask: multi-turn conversations are not yet supported for 'ollama' \
                     (only single-shot `ask ollama { ... }` is). Use 'openai' or 'anthropic', \
                     or use the single-shot ask primitive for ollama."
                        .into(),
                ),
            );
            out.insert("error_kind".into(), Value::Str("invalid_request".into()));
            out.insert("retry_after_ms".into(), Value::Null);
            out.insert("provider".into(), Value::Str("ollama".into()));
            out.insert("tokens_used".into(), Value::Number(0.0));
            out.insert("context".into(), Value::Object(ctx));
            return Ok(Value::Object(out));
        }

        let mut params: HashMap<String, Value> = HashMap::new();
        if let Some(sys) = get_str(&ctx, "system") {
            params.insert("system".into(), Value::Str(sys));
        }
        params.insert("messages".into(), Value::Array(get_messages(&ctx)));
        params.insert(
            "max_tokens".into(),
            Value::Number(get_num(&opts, "max_tokens", DEFAULT_RESPONSE_MAX_TOKENS)),
        );
        params.insert(
            "temperature".into(),
            Value::Number(get_num(&opts, "temperature", 0.7)),
        );
        if let Some(t) = opts.get("timeout").cloned() {
            params.insert("timeout".into(), t);
        }
        if let Some(tools) = opts.get("tools").cloned() {
            params.insert("tools".into(), tools);
        }

        let agent = self.http_agent();
        let span = self.diagnostics.start_span("ai.request");
        let message_count = get_messages(&ctx).len();
        let context_id = get_str(&ctx, "__gx_context_id").unwrap_or_default();
        let result = ai::ask_ai(&provider, model.as_deref(), &params, &agent);
        self.record_tokens(&result);

        let (ok_for_span, error_kind) = match &result {
            Value::Object(m) => (
                matches!(m.get("ok"), Some(Value::Bool(true))),
                m.get("error_kind")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            ),
            _ => (true, None),
        };
        let span_outcome = if ok_for_span {
            crate::diagnostics::Outcome::Ok
        } else {
            crate::diagnostics::Outcome::Error(error_kind.as_deref().unwrap_or("error"))
        };
        self.diagnostics.end_span(
            span,
            span_outcome,
            &[
                ("provider", serde_json::Value::String(provider.clone())),
                (
                    "model",
                    model
                        .clone()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                ),
                ("context_id", serde_json::Value::String(context_id)),
                (
                    "message_count",
                    serde_json::Value::Number(serde_json::Number::from(message_count)),
                ),
            ],
        );

        let Value::Object(result_map) = &result else {
            return Err(Signal::Error(
                "context_ask: unexpected AI response shape".into(),
            ));
        };
        let ok = matches!(result_map.get("ok"), Some(Value::Bool(true)));
        let tokens_used = get_num(result_map, "tokens_used", 0.0);

        let mut new_ctx = ctx.clone();
        let mut stats = match new_ctx.get("stats") {
            Some(Value::Object(s)) => s.clone(),
            _ => HashMap::new(),
        };
        let prior_ask_count = get_num(&stats, "ask_count", 0.0);
        stats.insert("ask_count".into(), Value::Number(prior_ask_count + 1.0));
        let prior_tokens = get_num(&stats, "total_tokens_used", 0.0);
        stats.insert(
            "total_tokens_used".into(),
            Value::Number(prior_tokens + tokens_used),
        );
        stats.insert("last_ask_at".into(), Value::Number(now_unix_ms() as f64));
        new_ctx.insert("stats".into(), Value::Object(stats));

        if ok {
            let text = get_str(result_map, "text").unwrap_or_default();
            let tool_calls = result_map.get("tool_calls").cloned();
            let append_result = self.context_add_message(&[
                Value::Object(new_ctx.clone()),
                Value::Str("assistant".into()),
                Value::Str(text),
                {
                    let mut o = HashMap::new();
                    if let Some(tc) = tool_calls {
                        o.insert("tool_calls".into(), tc);
                    }
                    Value::Object(o)
                },
            ]);
            // The AI call already succeeded (and was already billed for) by
            // this point — if appending its reply to the context somehow
            // failed, that must never discard the response itself. Fall
            // back to `new_ctx` as it was before the append (still valid,
            // just without this turn recorded) rather than propagating the
            // error and losing the caller's `result.text`/`tokens_used`.
            if let Ok(Value::Object(m)) = append_result {
                new_ctx = m;
            }
        }

        let mut out = result_map.clone();
        out.insert("context".into(), Value::Object(new_ctx));
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Interpreter;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        Interpreter::new().run_program(&program)
    }

    #[test]
    fn context_create_has_sane_defaults_and_a_stable_shape() {
        run(r#"
ctx = context_create()
s = context_stats(ctx)
assert s.message_count == 0 "a fresh context has no messages"
assert s.system_present == false "no system prompt given -> not present"
assert s.max_history_tokens == 8000 "default max_history_tokens"
assert s.ask_count == 0 "no asks yet"
assert s.total_tokens_used == 0 "no tokens used yet"
assert ctx.trim_strategy == "drop_oldest_pair" "default trim strategy"
"#)
        .unwrap();
    }

    #[test]
    fn context_create_rejects_an_unknown_trim_strategy() {
        run(r#"
try {
  context_create({ trim_strategy: "made_up_strategy" })
  assert false "an unknown trim_strategy must be rejected"
} catch e {
  assert true "unknown trim_strategy correctly rejected"
}
"#)
        .unwrap();
    }

    #[test]
    fn context_create_rejects_reserve_tokens_that_would_clamp_the_budget_to_zero() {
        // Regression test for a real bug caught while writing these very
        // tests: reserve_tokens (default 1000) >= a smaller custom
        // max_history_tokens silently clamped the trim threshold to 0, so
        // every message was discarded the instant it was added, with no
        // error — "the context always looks empty" with no clue why.
        run(r#"
try {
  context_create({ max_history_tokens: 500, reserve_tokens: 1000 })
  assert false "reserve_tokens >= max_history_tokens must be rejected at creation"
} catch e {
  assert true "reserve_tokens >= max_history_tokens correctly rejected"
}
try {
  context_create({ max_history_tokens: 500, reserve_tokens: 500 })
  assert false "reserve_tokens == max_history_tokens must also be rejected (zero headroom)"
} catch e {
  assert true "reserve_tokens == max_history_tokens correctly rejected"
}
// The fix must not be overly strict — a sane combination still works.
ctx = context_create({ max_history_tokens: 500, reserve_tokens: 100 })
assert context_stats(ctx).max_history_tokens == 500 "a valid reserve_tokens/max_history_tokens pair is accepted"
"#)
        .unwrap();
    }

    #[test]
    fn context_add_message_rejects_system_role_and_bad_tool_messages() {
        run(r#"
ctx = context_create()
try {
  context_add_message(ctx, "system", "nope")
  assert false "role=system must be rejected — use context_set_system"
} catch e {
  assert true "role=system correctly rejected"
}
try {
  context_add_message(ctx, "tool", "result with no id")
  assert false "role=tool without tool_call_id must be rejected"
} catch e {
  assert true "role=tool without tool_call_id correctly rejected"
}
"#)
        .unwrap();
    }

    #[test]
    fn automatic_trimming_keeps_estimated_tokens_under_budget() {
        run(r#"
ctx = context_create({ max_history_tokens: 100, reserve_tokens: 0, trim_strategy: "drop_oldest_pair" })
i = 0
while i < 40 {
  ctx = context_add_message(ctx, "user", "padding message number " + to_string(i) + " with extra text")
  ctx = context_add_message(ctx, "assistant", "ack " + to_string(i))
  i = i + 1
}
s = context_stats(ctx)
assert s.estimated_tokens <= 100 "auto-trim must keep the context under max_history_tokens"
assert s.total_messages_trimmed > 0 "trimming must actually have happened"
assert s.total_messages_added == 80 "stats must count every add, even ones later trimmed away"
"#)
        .unwrap();
    }

    #[test]
    fn trim_strategy_none_never_auto_trims() {
        run(r#"
ctx = context_create({ max_history_tokens: 1, reserve_tokens: 0, trim_strategy: "none" })
ctx = context_add_message(ctx, "user", "this alone already exceeds the tiny budget")
ctx = context_add_message(ctx, "assistant", "so does this")
s = context_stats(ctx)
assert s.message_count == 2 "trim_strategy 'none' must never drop messages automatically"
assert s.over_budget == true "but stats must still honestly report being over budget"
"#)
        .unwrap();
    }

    #[test]
    fn max_messages_hard_cap_triggers_even_when_under_the_token_budget() {
        // Defense in depth: the token estimate is a heuristic, so a hard
        // message-count ceiling exists independently of it.
        run(r#"
ctx = context_create({ max_history_tokens: 1000000, max_messages: 4, trim_strategy: "drop_oldest" })
i = 0
while i < 10 {
  ctx = context_add_message(ctx, "user", "x")
  i = i + 1
}
s = context_stats(ctx)
assert s.message_count <= 4 "max_messages must cap message count regardless of token budget"
"#)
        .unwrap();
    }

    #[test]
    fn tool_output_max_chars_truncates_and_flags_only_tool_messages() {
        run(r#"
ctx = context_create({ tool_output_max_chars: 10 })
ctx = context_add_tool_result(ctx, "call_1", "search", "0123456789ABCDEF")
ctx = context_add_message(ctx, "user", "0123456789ABCDEF")
tool_msg = ctx.messages[0]
user_msg = ctx.messages[1]
assert tool_msg.truncated == true "a tool result over the cap must be truncated"
assert len(tool_msg.content) == 10 "truncated content must be exactly tool_output_max_chars long"
assert user_msg.truncated == false "tool_output_max_chars must not apply to non-tool messages"
assert len(user_msg.content) == 16 "a user message must never be silently truncated by this cap"
"#)
        .unwrap();
    }

    #[test]
    fn user_and_assistant_messages_are_capped_by_the_hard_backstop_not_left_unbounded() {
        // Regression test for a real gap found in adversarial review: only
        // tool-role messages were ever capped — a single accidentally
        // enormous user/assistant message had no limit at all, and could
        // blow straight past the whole token budget in one add with
        // nothing `apply_trim` could do to fix it (it only removes *other*
        // messages). Exercises `build_message` directly since constructing
        // a 100,000+ character GX string literal isn't a practical way to
        // test a numeric boundary.
        let huge = "x".repeat(super::MAX_MESSAGE_CONTENT_CHARS + 500);
        let msg = super::build_message("user", &huge, None, None, None, 8000);
        let crate::value::Value::Object(m) = msg else {
            panic!("expected object");
        };
        assert_eq!(m.get("truncated"), Some(&crate::value::Value::Bool(true)));
        let crate::value::Value::Str(content) = m.get("content").unwrap() else {
            panic!("expected string content");
        };
        assert_eq!(content.chars().count(), super::MAX_MESSAGE_CONTENT_CHARS);

        // A message comfortably under the backstop is left completely
        // untouched, even though it's still much larger than the
        // (tool-specific, much smaller) tool_output_max_chars passed in.
        let normal = "y".repeat(50_000);
        let msg2 = super::build_message("assistant", &normal, None, None, None, 8000);
        let crate::value::Value::Object(m2) = msg2 else {
            panic!("expected object");
        };
        assert_eq!(m2.get("truncated"), Some(&crate::value::Value::Bool(false)));
    }

    #[test]
    fn clone_gets_an_independent_id_and_reset_preserves_configuration() {
        run(r#"
ctx = context_create({ system: "persona", max_history_tokens: 500, reserve_tokens: 100, trim_strategy: "drop_oldest" })
ctx = context_add_message(ctx, "user", "hi")

cloned = context_clone(ctx)
assert context_stats(cloned).context_id != context_stats(ctx).context_id "clone must mint a new context id"
assert len(cloned.messages) == 1 "clone starts with the same message history"

// Mutating the clone must never affect the original (context isolation —
// proves GX's value semantics, not just that clone *looks* independent).
cloned = context_add_message(cloned, "user", "only in the clone")
assert len(cloned.messages) == 2 "the clone's own further mutation only affects the clone"
assert len(ctx.messages) == 1 "the original must be completely unaffected by the clone's mutation"

r = context_reset(ctx)
assert len(r.messages) == 0 "reset clears the conversation"
assert r.system == "persona" "reset (keep_system default true) preserves the system prompt"
assert r.max_history_tokens == 500 "reset preserves configuration, not just defaults"
assert r.trim_strategy == "drop_oldest" "reset preserves trim_strategy"

r2 = context_reset(ctx, { keep_system: false })
assert r2.system == null "keep_system: false must clear the system prompt too"
"#)
        .unwrap();
    }

    #[test]
    fn serialize_deserialize_round_trips_and_rejects_a_bad_version() {
        run(r#"
ctx = context_create({ system: "persona" })
ctx = context_add_message(ctx, "user", "hello")
ctx = context_add_message(ctx, "assistant", "hi there")

json_str = context_serialize(ctx)
restored = context_deserialize(json_str)
assert len(restored.messages) == 2 "deserialize restores every message"
assert restored.messages[0].content == "hello" "deserialize preserves message content"
assert restored.system == "persona" "deserialize preserves the system prompt"

try {
  context_deserialize("{\"__gx_context_version\": 999999, \"messages\": []}")
  assert false "a context with an unsupported version must be rejected"
} catch e {
  assert true "unsupported version correctly rejected"
}

try {
  context_deserialize("not even json")
  assert false "invalid JSON must be rejected, not silently accepted as an empty context"
} catch e {
  assert true "invalid JSON correctly rejected"
}
"#)
        .unwrap();
    }

    #[test]
    fn summarize_and_trim_replaces_the_oldest_messages_with_one_summary() {
        run(r#"
ctx = context_create()
ctx = context_add_message(ctx, "user", "msg1")
ctx = context_add_message(ctx, "assistant", "reply1")
ctx = context_add_message(ctx, "user", "msg2")
ctx = context_add_message(ctx, "assistant", "reply2")

trimmed = context_summarize_and_trim(ctx, "Earlier discussion summarized.", { keep_last: 2 })
assert len(trimmed.messages) == 3 "1 summary message + 2 kept"
assert trimmed.messages[0].name == "summary" "the summary message is recognizable by name"
assert trimmed.messages[0].content == "Earlier discussion summarized." "the summary text is preserved verbatim"
assert trimmed.messages[1].content == "msg2" "keep_last preserves the most recent messages, in order"
assert trimmed.messages[2].content == "reply2" "keep_last preserves the most recent messages, in order"
assert context_stats(trimmed).total_messages_trimmed == 2 "the replaced messages are counted as trimmed"
"#)
        .unwrap();
    }

    #[test]
    fn context_ask_on_a_provider_error_preserves_the_context_unchanged() {
        // No API key is set in the test environment, so this always hits
        // the "credentials missing" error path — which is exactly the
        // behavior under test: on failure, context_ask must not invent an
        // assistant message, and must still hand back a usable `.context`.
        run(r#"
ctx = context_create({ system: "Be terse." })
ctx = context_add_message(ctx, "user", "hello")
result = context_ask(ctx, "openai", {})
assert result.ok == false "no credentials configured in this environment -> failure expected"
assert result.context != null "result.context must be present even on failure"
assert len(result.context.messages) == 1 "on failure, no assistant message should have been appended"
assert result.error_kind != null "every failure must carry a structured error_kind"
"#)
        .unwrap();
    }

    #[test]
    fn context_ask_rejects_ollama_clearly_instead_of_silently_sending_an_empty_prompt() {
        // Regression test for a real bug found in adversarial review:
        // ask_ai's flat `prompt` param (what ask_ollama actually reads) is
        // never set by context_ask (only `messages` is) — without this
        // explicit check, a context_ask against "ollama" would silently
        // send an *empty* prompt instead of the real conversation, rather
        // than failing in an obviously-wrong way.
        run(r#"
ctx = context_create({ system: "test" })
ctx = context_add_message(ctx, "user", "hello")
result = context_ask(ctx, "ollama", {})
assert result.ok == false "ollama is not yet supported by context_ask and must fail clearly"
assert result.error_kind == "invalid_request" "this is a caller/configuration error, not a transient one"
assert len(result.context.messages) == 1 "the context must be preserved unchanged, not corrupted"
"#)
        .unwrap();
    }

    #[test]
    fn require_context_rejects_a_plain_object_that_was_never_created_via_context_create() {
        run(r#"
try {
  context_add_message({ not_a: "context" }, "user", "hi")
  assert false "a plain object without __gx_context_version must be rejected"
} catch e {
  assert true "plain object correctly rejected as not a context"
}
"#)
        .unwrap();
    }
}
