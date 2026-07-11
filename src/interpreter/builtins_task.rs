//! Native Task Runtime: `task_spawn`, `task_wait`, `task_wait_all`,
//! `task_wait_any`, `task_cancel`, `task_status`, `task_id`, `is_cancelled`.
//!
//! This is the language-level concurrency primitive every other GX runtime
//! builds on: `spawn agent ... timeout`/`parallel { ... }` are implemented
//! in terms of it (see `Interpreter::call_agent_with_timeout`/
//! `eval_parallel_map` in `mod.rs`), not as a separate mechanism.
//!
//! ## Model
//!
//! A task is a GX closure (`fn() { ... }`, the same first-class value
//! `retry`/`map`/`filter` already accept) run on its own OS thread, against
//! its own child `Interpreter` — exactly the "one Interpreter per unit of
//! concurrent work" pattern already established for `spawn agent`, the HTTP
//! server's worker pool, and `parallel { ... }`. There is no async runtime
//! underneath; GX's tree-walking interpreter has no cooperative yield
//! points inside expression evaluation, so this is the only execution
//! model available without a much larger rewrite — matching the milestone's
//! instruction to not require async/await today.
//!
//! ## Cancellation is cooperative, not preemptive
//!
//! There is no safe way to forcibly kill a running Rust thread. Cancelling
//! a task therefore means: set a flag, and the task's own execution notices
//! it at the next checkpoint. Checkpoints are: the top of every statement
//! (`Interpreter::run_stmt` — covers every loop body and every straight-line
//! script, with no need to special-case `while`/`for`/`loop until`
//! individually), `sleep()` (polls in small increments instead of one long
//! blocking sleep), and process-runtime waits (a cancelled task kills any
//! child OS process it's waiting on — see `builtins_process::
//! wait_until_done_or_deadline`). A single very long `http_get`/`db_query`
//! call with no timeout of its own will still run to completion before a
//! cancellation takes effect — the same "cooperative" boundary every
//! mainstream language's cancellation model has (Go's `context`, Java's
//! `Thread.interrupt`, .NET's `CancellationToken`).
//!
//! ## Structured concurrency — no orphans
//!
//! Every task's `TaskState.parent` points at whatever task (if any) was
//! active on the *spawning* Interpreter at the moment of `task_spawn` —
//! cancelling a task therefore cancels every task nested under it too
//! (`TaskState::is_cancelled` walks the chain). And because a task's body
//! runs against its own child `Interpreter`, any further tasks *it* spawns
//! live in *that* Interpreter's own `tasks` registry — which gets cleaned
//! up by `Interpreter`'s `Drop` impl (`cleanup_tasks`, mirroring
//! `cleanup_processes` exactly) the moment that Interpreter's thread
//! function returns, for any reason. A task can never outlive the
//! Interpreter that owns it, recursively — the same guarantee
//! `cleanup_processes` already gives OS processes, extended to tasks.

use super::{Env, Signal};
use crate::value::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often cancellation-while-waiting is polled — `task_wait`/
/// `task_wait_all`/`task_wait_any`, and a task waiting for a bounded pool
/// slot. Same value as the process runtime's `POLL_INTERVAL` for the same
/// reason: quick enough to feel immediate, coarse enough not to burn CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Bounded grace period `cleanup_tasks` gives still-running tasks to notice
/// cancellation and finish, mirroring `builtins_process::
/// UNRESPONSIVE_GRACE_PERIOD` exactly (including its rationale: a task
/// blocked inside an uninterruptible builtin call can't be forced to stop
/// sooner than that call's own completion).
const CLEANUP_GRACE_PERIOD: Duration = Duration::from_secs(2);
/// Ceiling on how many tasks one Interpreter tracks concurrently (not yet
/// reaped by `task_wait`/`task_wait_all`/`task_wait_any`, or joined by
/// `cleanup_tasks`). Generous enough that no legitimate use should ever hit
/// it — this exists purely to turn "unbounded `task_spawn` loop eventually
/// exhausts OS threads and panics" into a normal, catchable `Signal::Error`
/// instead. Reaping already-finished tasks (via any `task_wait*` call, or a
/// bounded `pool`) keeps well under this even for a long-lived script that
/// spawns many short tasks over its life.
const MAX_CONCURRENT_TASKS: usize = 10_000;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

// ── Bounded parallel execution ──────────────────────────────────────────────

/// A minimal counting semaphore backing `task_spawn`'s optional `pool`/
/// `max_concurrent` option — a plain `AtomicUsize` compare-exchange rather
/// than a condvar/OS semaphore, consistent with this runtime's poll-based
/// waiting throughout (the same tradeoff `wait_until_done_or_deadline`
/// already makes: slightly less efficient than a wakeup-based primitive,
/// much simpler, and it composes for free with cancellation-while-waiting).
pub(super) struct Pool {
    limit: usize,
    in_use: AtomicUsize,
}

impl Pool {
    fn try_acquire(&self) -> bool {
        let cur = self.in_use.load(Ordering::SeqCst);
        if cur >= self.limit {
            return false;
        }
        self.in_use
            .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn release(&self) {
        self.in_use.fetch_sub(1, Ordering::SeqCst);
    }
}

// ── Task state ───────────────────────────────────────────────────────────────

enum TaskOutcome {
    Ok(Value),
    Err(String),
    Cancelled(String),
}

/// Cap on unconsumed `task_emit` entries per task — the same
/// defense-in-depth shape as `MAX_CONCURRENT_TASKS`: a task that emits
/// progress in a tight loop with nobody ever calling `task_progress` to
/// drain it must fail loudly (an error from `task_emit`) rather than grow
/// this queue — and the process's memory — without bound.
const MAX_PENDING_PROGRESS_ENTRIES: usize = 1000;

pub(super) struct TaskState {
    id: String,
    /// Whatever task was active on the spawning Interpreter at spawn time —
    /// the structured-concurrency link. `None` for a task spawned directly
    /// from top-level (non-task) script code.
    parent: Option<Arc<TaskState>>,
    label: Option<String>,
    done: AtomicBool,
    cancelled: AtomicBool,
    cancel_reason: Mutex<Option<String>>,
    result: Mutex<Option<TaskOutcome>>,
    started_at: Instant,
    started_at_unix_ms: i64,
    finished_at: Mutex<Option<Instant>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Incremental progress values pushed by `task_emit` from *inside* the
    /// running task, drained by `task_progress` from the caller — the one
    /// piece of Task Runtime state a long-running task genuinely had no way
    /// to report before finishing: `task_wait`'s result is only available
    /// on completion, and a spawned task's `Interpreter` is a completely
    /// separate instance with no memory shared with the caller's (see
    /// `spawn_task_internal`), so there was no channel at all for
    /// "here's what I've done so far, keep going."
    progress: Mutex<Vec<Value>>,
}

impl TaskState {
    fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    /// Whether this task should stop: it was cancelled directly, or any
    /// ancestor was (structured concurrency). Checked on every statement
    /// execution (`Interpreter::run_stmt`), so kept to a handful of atomic
    /// loads even in the worst case — real task nesting is expected to stay
    /// shallow (a handful of levels at most).
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
            || self.parent.as_deref().is_some_and(TaskState::is_cancelled)
    }

    /// First-writer-wins: if this task is cancelled more than once (e.g. an
    /// explicit `task_cancel` races a deadline watchdog), the first reason
    /// recorded is the one that sticks — that's whichever one actually
    /// mattered chronologically.
    fn cancel(&self, reason: &str) {
        let mut r = self.cancel_reason.lock().unwrap();
        if r.is_none() {
            *r = Some(reason.to_string());
        }
        drop(r);
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// The reason this task is cancelled — its own, or (walking up) the
    /// nearest cancelled ancestor's. Only meaningful when `is_cancelled()`.
    pub(super) fn cancel_reason(&self) -> String {
        if self.cancelled.load(Ordering::SeqCst) {
            self.cancel_reason
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "task_cancel() called".to_string())
        } else if let Some(p) = &self.parent {
            p.cancel_reason()
        } else {
            "cancelled".to_string()
        }
    }

    fn status_str(&self) -> &'static str {
        if !self.is_done() {
            return "running";
        }
        match &*self.result.lock().unwrap() {
            Some(TaskOutcome::Ok(_)) => "done",
            Some(TaskOutcome::Cancelled(_)) => "cancelled",
            Some(TaskOutcome::Err(_)) => "failed",
            None => "done",
        }
    }

    /// Build the `task_wait`/`task_wait_all`/`task_wait_any` result object
    /// for a task that has already finished. Panics if called before
    /// `is_done()` — every call site below only calls this after confirming
    /// completion.
    fn result_object(&self) -> Value {
        let mut obj = HashMap::new();
        // Locked once, for the shortest scope that covers both the
        // ok/value/error fields AND the status string below — computing
        // status via a second, separate `self.status_str()` call here
        // would try to lock `self.result` again while this guard is still
        // held, deadlocking (`std::sync::Mutex` isn't reentrant).
        let result = self.result.lock().unwrap();
        let (ok, value, error, error_kind, cancelled, status) = match result.as_ref() {
            Some(TaskOutcome::Ok(v)) => (true, v.clone(), Value::Null, Value::Null, false, "done"),
            Some(TaskOutcome::Err(e)) => (
                false,
                Value::Null,
                Value::Str(e.clone()),
                Value::Str(super::util::infer_error_kind(e).to_string()),
                false,
                "failed",
            ),
            Some(TaskOutcome::Cancelled(reason)) => (
                false,
                Value::Null,
                Value::Str(reason.clone()),
                Value::Str("cancelled".to_string()),
                true,
                "cancelled",
            ),
            None => (
                false,
                Value::Null,
                Value::Str("task has no result yet".into()),
                Value::Str("no_result".to_string()),
                false,
                "done",
            ),
        };
        drop(result);
        obj.insert("ok".into(), Value::Bool(ok));
        obj.insert("value".into(), value);
        obj.insert("error".into(), error);
        // Same `{ ok: false, error, error_kind, ... }` convention http_*/
        // process_*/context_ask already follow (see `unwrap`'s doc comment
        // in mod.rs) — task results used to omit `error_kind` entirely,
        // so `unwrap(task_wait(...))` on a failed task lost the structured
        // kind that every other builtin's failure carries.
        obj.insert("error_kind".into(), error_kind);
        obj.insert("cancelled".into(), Value::Bool(cancelled));
        obj.insert("timed_out".into(), Value::Bool(false));
        obj.insert("status".into(), Value::Str(status.to_string()));
        obj.insert("task_id".into(), Value::Str(self.id.clone()));
        // Any progress not yet drained via task_progress() — task_wait
        // reaps this TaskState from the Interpreter's registry right after
        // building this object (see reap_finished_task), so a caller that
        // only ever calls task_wait (never polling task_progress while the
        // task was still running) would otherwise have no way to reach
        // whatever was emitted right before completion; it would just be
        // silently unreachable the instant this function returns.
        obj.insert("progress".into(), Value::Array(self.drain_progress()));
        Value::Object(obj)
    }

    /// Push one progress value — called only from inside the task's own
    /// running body (see `Interpreter::task_emit`). Rejects once
    /// `MAX_PENDING_PROGRESS_ENTRIES` unconsumed entries have piled up
    /// rather than growing forever.
    fn emit_progress(&self, value: Value) -> Result<(), Signal> {
        let mut q = self.progress.lock().unwrap();
        if q.len() >= MAX_PENDING_PROGRESS_ENTRIES {
            return Err(Signal::Error(format!(
                "task_emit: {} unconsumed progress entries already pending — call task_progress() to drain them",
                MAX_PENDING_PROGRESS_ENTRIES
            )));
        }
        q.push(value);
        Ok(())
    }

    /// Take every progress value pushed since the last drain — an empty
    /// array when there's nothing new, not an error, since "no progress
    /// since I last checked" is the ordinary, expected case for a caller
    /// polling a still-running task.
    fn drain_progress(&self) -> Vec<Value> {
        std::mem::take(&mut self.progress.lock().unwrap())
    }

    fn timed_out_object(&self) -> Value {
        let mut obj = HashMap::new();
        obj.insert("ok".into(), Value::Bool(false));
        obj.insert("value".into(), Value::Null);
        obj.insert("error".into(), Value::Str("task timed out".to_string()));
        obj.insert("error_kind".into(), Value::Str("timeout".to_string()));
        obj.insert("cancelled".into(), Value::Bool(false));
        obj.insert("timed_out".into(), Value::Bool(true));
        obj.insert("status".into(), Value::Str("running".to_string()));
        obj.insert("task_id".into(), Value::Str(self.id.clone()));
        Value::Object(obj)
    }

    fn status_object(&self) -> Value {
        let mut obj = HashMap::new();
        obj.insert("task_id".into(), Value::Str(self.id.clone()));
        obj.insert("status".into(), Value::Str(self.status_str().to_string()));
        obj.insert("done".into(), Value::Bool(self.is_done()));
        obj.insert("cancelled".into(), Value::Bool(self.is_cancelled()));
        obj.insert(
            "label".into(),
            self.label.clone().map(Value::Str).unwrap_or(Value::Null),
        );
        obj.insert(
            "parent_id".into(),
            self.parent
                .as_ref()
                .map(|p| Value::Str(p.id.clone()))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "started_at".into(),
            Value::Number(self.started_at_unix_ms as f64),
        );
        let duration_ms = match self.finished_at.lock().unwrap().as_ref() {
            Some(finished) => finished.duration_since(self.started_at).as_millis() as f64,
            None => self.started_at.elapsed().as_millis() as f64,
        };
        obj.insert("duration_ms".into(), Value::Number(duration_ms));
        Value::Object(obj)
    }
}

/// Poll until `state.is_done()`, or (if `deadline` given) until the
/// deadline passes — same shape as `builtins_process::
/// wait_until_done_or_deadline`, deliberately: this runtime's whole
/// "wait for something in the background" idiom is this one poll loop.
fn wait_until_done_or_deadline(state: &TaskState, deadline: Option<Instant>) -> bool {
    loop {
        if state.is_done() {
            return true;
        }
        if let Some(d) = deadline {
            if Instant::now() >= d {
                return false;
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

pub(super) struct SpawnOpts {
    timeout: Option<Duration>,
    label: Option<String>,
    pool: Option<String>,
    max_concurrent: Option<usize>,
}

impl SpawnOpts {
    /// Construct opts for an internal (non-GX-closure) spawn — used by
    /// `call_agent_with_timeout`/`eval_parallel_map`, which have their own
    /// pre-existing timeout/label conventions and don't go through
    /// `parse_spawn_opts`'s GX-object-argument parsing.
    pub(super) fn new(label: impl Into<String>, timeout: Option<Duration>) -> Self {
        SpawnOpts {
            timeout,
            label: Some(label.into()),
            pool: None,
            max_concurrent: None,
        }
    }
}

fn parse_spawn_opts(v: Option<&Value>) -> SpawnOpts {
    let Some(Value::Object(m)) = v else {
        return SpawnOpts {
            timeout: None,
            label: None,
            pool: None,
            max_concurrent: None,
        };
    };
    SpawnOpts {
        // `ms.max(0.0)` alone rescues NaN (`f64::max` ignores a NaN operand)
        // but not `Infinity` — reachable from an ordinary GX numeric
        // literal with enough digits to overflow `f64` — which used to
        // reach `Duration::from_secs_f64` unclamped and panic.
        timeout: m.get("timeout").and_then(|v| v.as_number()).map(|ms| {
            crate::clamp_duration_secs(ms.max(0.0) / 1000.0, Duration::from_secs(315_360_000))
        }),
        label: m.get("label").and_then(|v| v.as_str()).map(String::from),
        pool: m.get("pool").and_then(|v| v.as_str()).map(String::from),
        max_concurrent: m
            .get("max_concurrent")
            .and_then(|v| v.as_number())
            .map(|n| n.max(1.0) as usize),
    }
}

/// Pulled out of `spawn_task_internal` as a pure function so the cap logic
/// is testable directly, without actually spawning `MAX_CONCURRENT_TASKS`
/// real OS threads just to exercise the rejection path.
fn check_task_capacity(current_count: usize, limit: usize) -> Result<(), Signal> {
    if current_count >= limit {
        return Err(Signal::Error(format!(
            "task_spawn: too many concurrently-tracked tasks ({} already tracked by this \
             interpreter, limit {}) — call task_wait/task_wait_all on finished tasks to \
             free their slots, or use a bounded pool ({{ pool: \"name\", max_concurrent: N }})",
            current_count, limit
        )));
    }
    Ok(())
}

fn require_handle_arg<'a>(args: &'a [Value], who: &str) -> Result<&'a str, Signal> {
    args.first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| Signal::Error(format!("{} requires a task handle string", who)))
}

/// Optional second argument shared by `task_wait`/`task_wait_all`/
/// `task_wait_any`: a timeout in milliseconds, `None` meaning "wait
/// forever" — the same convention `process_wait`'s own timeout uses.
fn parse_wait_timeout(v: Option<&Value>) -> Option<Duration> {
    v.and_then(|v| v.as_number()).map(|ms| {
        crate::clamp_duration_secs(ms.max(0.0) / 1000.0, Duration::from_secs(315_360_000))
    })
}

impl super::Interpreter {
    /// `task_spawn(fn, opts?) -> handle`. Returns immediately; the closure
    /// runs on a new thread against a fresh child `Interpreter` that
    /// inherits capabilities/diagnostics/helpers/functions — the same
    /// inheritance `spawn agent`/`parallel {}` already give a spawned
    /// agent.
    pub(super) fn task_spawn(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let closure = args.first().cloned().ok_or_else(|| {
            Signal::Error("task_spawn: expected a function as the first argument".into())
        })?;
        if !matches!(closure, Value::Closure(..)) {
            return Err(Signal::Error(format!(
                "task_spawn: expected a function, got {}",
                closure.type_name()
            )));
        }
        let opts = parse_spawn_opts(args.get(1));
        let state = self.spawn_task_internal(opts, move |child| {
            let mut env = Env::new();
            child.call_closure(&closure, vec![], &mut env)
        })?;
        Ok(Value::Str(state.id.clone()))
    }

    /// Drop a finished task's entry from `self.tasks` and join its thread
    /// (near-instant — it's already confirmed done). Without this,
    /// `self.tasks` would grow without bound for any long-lived Interpreter
    /// that calls `task_spawn` many times over its life (an HTTP worker
    /// spawning a background task per request, say) — every entry stays
    /// reachable, and with it the closure's captured environment and
    /// result value, forever. Mirrors `process_wait`'s own
    /// `self.processes.remove(handle)` exactly. Only called once a task is
    /// confirmed `is_done()` — never for one that's merely been told to
    /// cancel and might still be winding down.
    fn reap_finished_task(&mut self, state: &Arc<TaskState>) {
        self.tasks.remove(state.id());
        if let Some(handle) = state.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    /// The shared spawn path behind `task_spawn` (a GX closure body) and
    /// every internal caller that needs a real, tracked, cancellable task
    /// instead of a bare untracked thread — `call_agent_with_timeout`
    /// (`spawn agent ... timeout`) and `eval_parallel_map` (`parallel {
    /// ... }`) both go through this now, which is what fixes their
    /// pre-existing bug: on timeout, they used to just stop *waiting* and
    /// abandon the still-running background thread with no way to ever
    /// cancel or reap it. Now that thread is a task like any other —
    /// `task_cancel`-able, and guaranteed joined by `cleanup_tasks` even if
    /// nobody ever calls `task_wait` on it again.
    ///
    /// Bounded by `MAX_CONCURRENT_TASKS`: unlike `process_spawn` (whose OS
    /// limit is enforced by the kernel and surfaces as an ordinary
    /// `Result`), `std::thread::Builder::spawn` *returns* `io::Result`
    /// rather than panicking — mapped below to an ordinary `Signal::Error`
    /// so a host under memory/thread-count pressure (a container with a
    /// tight `ulimit -u`, for instance) fails one `task_spawn` call
    /// gracefully instead of crashing the whole interpreter. This can
    /// happen well before `MAX_CONCURRENT_TASKS` tracked tasks are
    /// reached, since that cap bounds *tracked* tasks, not the OS's own
    /// thread-creation limit.
    pub(super) fn spawn_task_internal(
        &mut self,
        opts: SpawnOpts,
        body: impl FnOnce(&mut super::Interpreter) -> Result<Value, Signal> + Send + 'static,
    ) -> Result<Arc<TaskState>, Signal> {
        check_task_capacity(self.tasks.len(), MAX_CONCURRENT_TASKS)?;
        let id = uuid::Uuid::new_v4().to_string();
        let parent = self.current_task.clone();

        let pool = match (&opts.pool, opts.max_concurrent) {
            (Some(name), Some(max)) => Some(
                self.task_pools
                    .entry(name.clone())
                    .or_insert_with(|| {
                        Arc::new(Pool {
                            limit: max,
                            in_use: AtomicUsize::new(0),
                        })
                    })
                    .clone(),
            ),
            _ => None,
        };

        let helpers = self.helpers.clone();
        let functions = self.functions.clone();
        let capabilities = self.capabilities.clone();
        let diagnostics = self.diagnostics.for_child();
        // Testing Framework: `set_random_seed(n)` must make an entire test
        // run deterministic, including any work it hands off via
        // `task_spawn` — a child Interpreter that silently reset to
        // clock-seeded randomness would make "seeded" a lie for exactly
        // the concurrent code a test is most likely to want deterministic
        // coverage of.
        let rng_seed = self.rng_seed;
        let deadline = opts.timeout.map(|d| Instant::now() + d);
        let label = opts.label.clone();

        let state = Arc::new(TaskState {
            id: id.clone(),
            parent,
            label: label.clone(),
            done: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            cancel_reason: Mutex::new(None),
            result: Mutex::new(None),
            started_at: Instant::now(),
            started_at_unix_ms: now_unix_ms(),
            finished_at: Mutex::new(None),
            thread: Mutex::new(None),
            progress: Mutex::new(Vec::new()),
        });

        let thread_state = state.clone();
        let watchdog_state = state.clone();

        let join_handle = std::thread::Builder::new()
            .stack_size(super::WORKER_THREAD_STACK_SIZE)
            .spawn(move || {
                if let Some(d) = deadline {
                    std::thread::spawn(move || {
                        while !watchdog_state.is_done() {
                            if Instant::now() >= d {
                                watchdog_state.cancel("deadline exceeded");
                                return;
                            }
                            std::thread::sleep(POLL_INTERVAL);
                        }
                    });
                }

                let mut child = super::Interpreter::new();
                child.helpers = helpers;
                child.functions = functions;
                child.capabilities = capabilities;
                child.diagnostics = diagnostics;
                child.rng_seed = rng_seed;
                child.current_task = Some(thread_state.clone());

                let outcome =
                    run_task_body(&mut child, body, pool.as_deref(), &thread_state, label);

                *thread_state.finished_at.lock().unwrap() = Some(Instant::now());
                *thread_state.result.lock().unwrap() = Some(outcome);
                thread_state.done.store(true, Ordering::SeqCst);
                // `child` drops here — its own `tasks`/`processes` registries
                // (any further tasks/processes *this* task spawned) are cleaned
                // up by `Interpreter::drop`, recursively.
            })
            .map_err(|e| {
                Signal::Error(format!(
                    "task_spawn: failed to create an OS thread for the task: {}",
                    e
                ))
            })?;

        *state.thread.lock().unwrap() = Some(join_handle);
        self.tasks.insert(id.clone(), state.clone());
        Ok(state)
    }

    pub(super) fn task_wait(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "task_wait")?;
        let Some(state) = self.tasks.get(handle).cloned() else {
            return Ok(Value::Null);
        };
        let timeout = parse_wait_timeout(args.get(1));
        let deadline = timeout.map(|t| Instant::now() + t);
        if wait_until_done_or_deadline(&state, deadline) {
            let result = state.result_object();
            self.reap_finished_task(&state);
            Ok(result)
        } else {
            // Mirrors process_wait: this call's own timeout also cancels
            // the task, the same "subprocess.run(timeout=)" convention
            // already established there. Not yet actually done, so not
            // reaped — a later task_wait/task_status can still observe it
            // winding down.
            state.cancel("task_wait timeout elapsed");
            Ok(state.timed_out_object())
        }
    }

    pub(super) fn task_wait_all(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handles = match args.first() {
            Some(Value::Array(items)) => items.clone(),
            _ => {
                return Err(Signal::Error(
                    "task_wait_all: expected an array of task handles".into(),
                ))
            }
        };
        let timeout = parse_wait_timeout(args.get(1));
        let deadline = timeout.map(|t| Instant::now() + t);

        let states: Vec<Option<Arc<TaskState>>> = handles
            .iter()
            .map(|h| h.as_str().and_then(|h| self.tasks.get(h).cloned()))
            .collect();

        let mut results = Vec::with_capacity(states.len());
        for state in &states {
            let Some(state) = state else {
                results.push(Value::Null);
                continue;
            };
            if wait_until_done_or_deadline(state, deadline) {
                results.push(state.result_object());
                self.reap_finished_task(state);
            } else {
                state.cancel("task_wait_all timeout elapsed");
                results.push(state.timed_out_object());
            }
        }
        Ok(Value::Array(results))
    }

    pub(super) fn task_wait_any(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handles = match args.first() {
            Some(Value::Array(items)) => items.clone(),
            _ => {
                return Err(Signal::Error(
                    "task_wait_any: expected an array of task handles".into(),
                ))
            }
        };
        let timeout = parse_wait_timeout(args.get(1));
        let deadline = timeout.map(|t| Instant::now() + t);

        let states: Vec<Option<Arc<TaskState>>> = handles
            .iter()
            .map(|h| h.as_str().and_then(|h| self.tasks.get(h).cloned()))
            .collect();
        if states.is_empty() || states.iter().all(|s| s.is_none()) {
            return Ok(Value::Null);
        }

        loop {
            for (i, state) in states.iter().enumerate() {
                if let Some(state) = state {
                    if state.is_done() {
                        let mut obj = match state.result_object() {
                            Value::Object(m) => m,
                            _ => unreachable!(),
                        };
                        obj.insert("index".into(), Value::Number(i as f64));
                        // Only the winner is reaped — the other handles are
                        // still (or may still be) running elsewhere; a
                        // script that wants them stopped calls task_cancel
                        // itself (see the pattern in the language reference).
                        self.reap_finished_task(state);
                        return Ok(Value::Object(obj));
                    }
                }
            }
            if let Some(d) = deadline {
                if Instant::now() >= d {
                    return Ok(Value::Null);
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    pub(super) fn task_cancel(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "task_cancel")?;
        let reason = args
            .get(1)
            .and_then(|v| v.as_str())
            .unwrap_or("task_cancel() called");
        let Some(state) = self.tasks.get(handle) else {
            return Ok(Value::Bool(false));
        };
        if state.is_done() {
            return Ok(Value::Bool(false));
        }
        state.cancel(reason);
        Ok(Value::Bool(true))
    }

    pub(super) fn task_status(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "task_status")?;
        let Some(state) = self.tasks.get(handle) else {
            return Ok(Value::Null);
        };
        Ok(state.status_object())
    }

    /// task_emit(value) — called from *inside* a running task's own body
    /// to report incremental progress, drained by the caller via
    /// `task_progress(handle)`. Uses `self.current_task` (set on the
    /// task's own child Interpreter — see `spawn_task_internal`), not
    /// `self.tasks` (the caller's registry of tasks it owns, which the
    /// task's own separate Interpreter instance never has), since this is
    /// the task reporting on *itself*.
    pub(super) fn task_emit(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let value = args.first().cloned().unwrap_or(Value::Null);
        let Some(task) = self.current_task.clone() else {
            return Err(Signal::Error(
                "task_emit: not running inside a task (use task_spawn to create one)".into(),
            ));
        };
        task.emit_progress(value)?;
        Ok(Value::Null)
    }

    /// task_progress(handle) → array — every progress value the task has
    /// `task_emit`'d since the *last* call to `task_progress` for this
    /// handle (or since the task started, for the first call). Draining
    /// rather than peeking: a caller polling in a loop naturally wants
    /// each entry exactly once, not to re-process everything ever emitted
    /// on every poll.
    pub(super) fn task_progress(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "task_progress")?;
        let Some(state) = self.tasks.get(handle) else {
            return Ok(Value::Array(Vec::new()));
        };
        Ok(Value::Array(state.drain_progress()))
    }

    /// The current task's handle, or `null` outside of one — mirrors
    /// `trace_id()`/`span_id()`'s "null when there's no ambient context"
    /// convention exactly.
    pub(super) fn task_id_builtin(&self) -> Value {
        self.current_task
            .as_ref()
            .map(|t| Value::Str(t.id.clone()))
            .unwrap_or(Value::Null)
    }

    /// Whether the current task (or an ancestor of it) has been cancelled —
    /// `false` outside of a task. The primary primitive for a script's own
    /// long-running loop to check cooperatively, on top of the automatic
    /// per-statement check every task already gets for free.
    pub(super) fn is_cancelled_builtin(&self) -> Value {
        Value::Bool(self.current_task.as_ref().is_some_and(|t| t.is_cancelled()))
    }

    /// A reusable "should the thing I'm about to wait on stop?" check for
    /// other runtimes to integrate against, without depending on
    /// `TaskState`'s type directly — the Process Runtime's
    /// `spawn_cancellation_killer` is the first consumer ("Process Runtime
    /// terminates child processes when parent tasks are cancelled"); a
    /// future HTTP client cancellation hook would use the same thing.
    /// `None` when there's no active task, so a caller with no task context
    /// doesn't pay for a closure/thread it doesn't need.
    pub(super) fn cancellation_check(&self) -> Option<Box<dyn Fn() -> bool + Send>> {
        let task = self.current_task.clone()?;
        Some(Box::new(move || task.is_cancelled()))
    }

    /// Cancel + join every task this Interpreter still directly owns.
    /// Called from `Drop` and explicitly before `exit()` — mirrors
    /// `cleanup_processes` exactly, including the "abandon, don't block
    /// forever" fallback for a task that's still stuck past the grace
    /// period (its OS thread is reclaimed when this process exits either
    /// way; this just bounds *our* shutdown latency).
    pub(crate) fn cleanup_tasks(&mut self) {
        for state in self.tasks.values() {
            state.cancel("interpreter shutting down");
        }
        let deadline = Instant::now() + CLEANUP_GRACE_PERIOD;
        for state in self.tasks.values() {
            if wait_until_done_or_deadline(state, Some(deadline)) {
                if let Some(handle) = state.thread.lock().unwrap().take() {
                    let _ = handle.join();
                }
            }
            // Else: still running past the grace period (stuck in an
            // uninterruptible builtin call) — abandoned, not blocked on
            // forever. Reclaimed by the OS when this process exits.
        }
        self.tasks.clear();
    }
}

/// Run one task's body: acquire a bounded-pool permit if requested (itself
/// cancellable while waiting), start its span, invoke the closure with
/// panic isolation, end the span. Pulled out of `task_spawn`'s thread
/// closure so every exit path — normal, error, cancelled-while-queued,
/// panic — funnels through one place that always records exactly one
/// `TaskOutcome` and always releases the pool permit it acquired.
fn run_task_body(
    child: &mut super::Interpreter,
    body: impl FnOnce(&mut super::Interpreter) -> Result<Value, Signal>,
    pool: Option<&Pool>,
    state: &Arc<TaskState>,
    label: Option<String>,
) -> TaskOutcome {
    if let Some(pool) = pool {
        loop {
            if state.is_cancelled() {
                return TaskOutcome::Cancelled(state.cancel_reason());
            }
            if pool.try_acquire() {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    let span_name = label.unwrap_or_else(|| "task".to_string());
    let span = child.diagnostics.start_span(span_name);

    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(child)));

    let outcome = match panic_result {
        Ok(Ok(v)) => TaskOutcome::Ok(v),
        Ok(Err(Signal::Cancelled(reason))) => TaskOutcome::Cancelled(reason),
        // describe_stray_signal, not `{:?}` — a task's `error` field used to
        // leak Rust's raw enum debug syntax (e.g. `Error("division by
        // zero...")`) instead of the plain message every other builtin's
        // failure carries.
        Ok(Err(e)) => TaskOutcome::Err(child.describe_stray_signal(e)),
        Err(payload) => TaskOutcome::Err(format!("panic: {}", panic_message(&*payload))),
    };

    let err_msg = match &outcome {
        TaskOutcome::Err(e) => Some(e.clone()),
        TaskOutcome::Ok(_) | TaskOutcome::Cancelled(_) => None,
    };
    let span_outcome = match (&outcome, &err_msg) {
        (TaskOutcome::Ok(_), _) => crate::diagnostics::Outcome::Ok,
        (TaskOutcome::Cancelled(_), _) => crate::diagnostics::Outcome::Cancelled,
        (TaskOutcome::Err(_), Some(e)) => crate::diagnostics::Outcome::Error(e.as_str()),
        (TaskOutcome::Err(_), None) => unreachable!(),
    };
    child.diagnostics.end_span(
        span,
        span_outcome,
        &[("task_id", serde_json::Value::String(state.id.clone()))],
    );

    if let Some(pool) = pool {
        pool.release();
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::super::Interpreter;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::value::Value;
    use std::time::{Duration, Instant};

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        Interpreter::new().run_program(&program)
    }

    /// Same as `run`, but hands back the `Interpreter` afterward so a test
    /// can inspect state `run_program`'s `Result<(), String>` throws away
    /// (e.g. `assert_count`).
    fn run_with_interp(src: &str) -> (Interpreter, Result<(), String>) {
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run_program(&program);
        (interp, result)
    }

    #[test]
    fn task_spawn_and_wait_returns_the_closures_value() {
        run(r#"
h = task_spawn(fn() { return 1 + 1 })
r = task_wait(h)
assert r.ok == true "task must report ok"
assert r.value == 2 "task must return the closure's value"
assert r.cancelled == false "a normal completion is not a cancellation"
assert r.timed_out == false "a normal completion did not time out"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_and_status_on_an_unknown_handle_return_null() {
        run(r#"
assert task_wait("definitely-not-a-real-handle") == null "task_wait: unknown handle -> null"
assert task_status("definitely-not-a-real-handle") == null "task_status: unknown handle -> null"
"#)
        .unwrap();
    }

    #[test]
    fn task_cancel_on_an_unknown_or_already_finished_handle_returns_false() {
        run(r#"
assert task_cancel("definitely-not-a-real-handle") == false "task_cancel: unknown handle -> false"
h = task_spawn(fn() { return 1 })
task_wait(h)
assert task_cancel(h) == false "task_cancel: already-finished task -> false"
"#)
        .unwrap();
    }

    #[test]
    fn task_cancel_stops_a_running_loop_via_the_next_statement_checkpoint() {
        run(r#"
h = task_spawn(fn() {
  i = 0
  while i < 100000000 {
    i = i + 1
  }
  return "should never reach here"
})
sleep(0.1)
assert task_cancel(h) == true "task_cancel must find and flag a running task"
r = task_wait(h, 5000)
assert r.ok == false "a cancelled task did not complete successfully"
assert r.cancelled == true "task_wait must report the cancellation"
assert r.timed_out == false "this is a real cancellation, not task_wait's own timeout"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_own_timeout_cancels_the_task_and_reports_timed_out() {
        run(r#"
h = task_spawn(fn() {
  i = 0
  while i < 100000000 {
    i = i + 1
  }
  return "should never reach here"
})
r = task_wait(h, 100)
assert r.timed_out == true "task_wait's own timeout must be reported distinctly"
assert r.ok == false "a timed-out wait is not a successful result"
assert r.error_kind == "timeout" "must match the same error_kind vocabulary http_*/process_* use for timeouts"
assert r.error != null "a timed-out result must carry a human-readable error, not just error_kind"
// The timeout must have actually cancelled the task (not just stopped waiting) —
// confirmed by a second wait picking up the real, now-cancelled outcome.
r2 = task_wait(h, 5000)
assert r2.cancelled == true "task_wait's own timeout must cancel, not just stop waiting"
assert r2.error_kind == "cancelled" "a cancelled task's own result must also carry a structured error_kind"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_on_a_failed_task_carries_a_structured_error_kind() {
        // Regression test: task result objects used to have no `error_kind`
        // field at all, unlike every other `{ ok: false, error, error_kind,
        // ... }` producer (http_*, process_*, context_ask) — so
        // `unwrap(task_wait(...))` on a failed task lost the structured
        // kind that every other builtin's failure carries.
        run(r#"
h = task_spawn(fn() {
  assert false "deliberate task failure"
})
r = task_wait(h)
assert r.ok == false "a failed task is not a successful result"
assert r.error_kind == "AssertionError" "must reuse the same classifier try/catch uses for thrown errors"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_all_returns_every_result_in_the_same_order_as_the_input_handles() {
        run(r#"
h1 = task_spawn(fn() { return 1 })
h2 = task_spawn(fn() { return 2 })
h3 = task_spawn(fn() { return 3 })
results = task_wait_all([h1, h2, h3])
assert len(results) == 3 "one result per handle"
assert results[0].value == 1 "results[0] corresponds to h1"
assert results[1].value == 2 "results[1] corresponds to h2"
assert results[2].value == 3 "results[2] corresponds to h3"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_any_returns_the_first_task_to_finish() {
        run(r#"
slow = task_spawn(fn() { sleep(1) return "slow" })
fast = task_spawn(fn() { return "fast" })
winner = task_wait_any([slow, fast])
assert winner.value == "fast" "the faster task must win the race"
assert winner.index == 1 "index must identify which handle won"
task_cancel(slow)
"#)
        .unwrap();
    }

    #[test]
    fn task_id_and_is_cancelled_are_null_and_false_outside_any_task() {
        run(r#"
assert task_id() == null "task_id() outside a task is null"
assert is_cancelled() == false "is_cancelled() outside a task is false"
"#)
        .unwrap();
    }

    #[test]
    fn task_id_and_is_cancelled_reflect_the_running_task_from_inside_it() {
        run(r#"
h = task_spawn(fn() {
  inner_id = task_id()
  inner_cancelled = is_cancelled()
  return { id_matches_self: inner_id != null, was_cancelled: inner_cancelled }
})
r = task_wait(h)
assert r.value.id_matches_self == true "task_id() inside a task returns its own non-null handle"
assert r.value.was_cancelled == false "is_cancelled() inside a never-cancelled task is false"
"#)
        .unwrap();
    }

    #[test]
    fn bounded_pool_actually_serializes_execution_not_just_completion() {
        // 6 tasks x 150ms each, max_concurrent 2 -> 3 sequential batches ->
        // must take meaningfully longer than running all 6 unbounded
        // (~150ms) would. This is the one property a "does it eventually
        // finish" test can't distinguish from a pool that's a no-op.
        let (interp, result) = run_with_interp(
            r#"
start = now_ms()
handles = []
i = 0
while i < 6 {
  handles = handles + [task_spawn(fn() {
    sleep(0.15)
    return true
  }, { pool: "test-pool", max_concurrent: 2 })]
  i = i + 1
}
results = task_wait_all(handles, 10000)
j = 0
all_ok = true
while j < len(results) {
  if results[j].ok != true { all_ok = false }
  j = j + 1
}
elapsed = now_ms() - start
assert all_ok == true "every pooled task must still complete successfully"
assert elapsed >= 400 "max_concurrent=2 must serialize 6x150ms tasks into >=3 batches, got {elapsed}ms"
"#,
        );
        result.unwrap();
        drop(interp);
    }

    #[test]
    fn a_panic_inside_a_task_body_is_isolated_and_reported_as_a_failure() {
        // GX itself has no reachable way to trigger a genuine Rust panic
        // (confirmed in earlier milestones), so this exercises
        // `run_task_body`'s catch_unwind directly rather than through GX
        // source — the same approach used for `db_transaction`'s own
        // panic-safety test.
        let mut interp = Interpreter::new();
        let opts = super::SpawnOpts::new("panic-test", None);
        let state = interp
            .spawn_task_internal(opts, |_child| {
                panic!("simulated task body panic");
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !state.is_done() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            state.is_done(),
            "a panicking task must still be reported done, not hang"
        );
        let result = interp
            .task_wait(&[Value::Str(state.id().to_string())])
            .unwrap();
        let Value::Object(m) = result else {
            panic!("expected an object");
        };
        assert_eq!(m.get("ok"), Some(&Value::Bool(false)));
        assert!(m
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("panic"));
    }

    #[test]
    fn cleanup_tasks_cancels_and_joins_a_task_the_interpreter_still_owns() {
        // No explicit task_wait/task_cancel — the Interpreter going out of
        // scope must still cancel and join it (Drop -> cleanup_tasks),
        // exactly mirroring cleanup_processes's guarantee for OS processes.
        // Real-world equivalent: a script spawns a task and simply ends.
        let start = Instant::now();
        {
            let mut interp = Interpreter::new();
            let opts = super::SpawnOpts::new("cleanup-test", None);
            let _state = interp
                .spawn_task_internal(opts, |child| {
                    let mut env = super::Env::new();
                    child.call_closure(
                        &Value::Closure(
                            vec![],
                            vec![crate::ast::Stmt::While {
                                condition: crate::ast::Expr::Bool(true),
                                body: vec![],
                                line: 0,
                            }],
                            std::collections::HashMap::new(),
                        ),
                        vec![],
                        &mut env,
                    )
                })
                .unwrap();
            // interp drops here -> cleanup_tasks must cancel + bound the wait,
            // not hang until the infinite loop finishes (it never would).
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "Drop must not block indefinitely on a task that never finishes on its own"
        );
    }

    #[test]
    fn structured_concurrency_cancelling_a_parent_task_cancels_its_child() {
        run(r#"
parent = task_spawn(fn() {
  child = task_spawn(fn() {
    i = 0
    while i < 100000000 {
      i = i + 1
    }
    return "should never reach here"
  })
  return task_wait(child)
})
sleep(0.1)
task_cancel(parent)
parent_result = task_wait(parent, 5000)
assert parent_result.ok == true "the parent's own body (waiting on its child) completes normally"
child_result = parent_result.value
assert child_result.cancelled == true "the child must be cancelled too — structured concurrency"
"#)
        .unwrap();
    }

    #[test]
    fn task_run_leaves_no_orphaned_os_process_when_the_owning_task_is_cancelled() {
        // "Process Runtime terminates child processes when parent tasks are
        // cancelled" — the actual cross-runtime integration point.
        let mut interp = Interpreter::new();
        interp.capabilities.process = true;
        let opts = super::SpawnOpts::new("process-cancel-test", None);
        let (cmd, args) = if cfg!(windows) {
            (
                "cmd",
                vec![
                    "/C".to_string(),
                    "timeout".to_string(),
                    "/T".to_string(),
                    "30".to_string(),
                ],
            )
        } else {
            ("sleep", vec!["30".to_string()])
        };
        let state = interp
            .spawn_task_internal(opts, move |child| {
                let mut obj = std::collections::HashMap::new();
                obj.insert("command".to_string(), Value::Str(cmd.to_string()));
                obj.insert(
                    "args".to_string(),
                    Value::Array(args.iter().cloned().map(Value::Str).collect()),
                );
                child.process_run(&[Value::Object(obj)])
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let cancel_start = Instant::now();
        state.cancel("test cancellation");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !state.is_done() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(state.is_done(), "task must finish promptly once cancelled");
        assert!(
            cancel_start.elapsed() < Duration::from_secs(3),
            "the child process must be killed promptly, not left to run its full 30s duration"
        );
    }

    #[test]
    fn task_wait_reaps_a_finished_tasks_entry_so_a_long_lived_interpreter_does_not_leak() {
        // Regression test for a real bug found in adversarial review: an
        // earlier version of task_wait/task_wait_all/task_wait_any never
        // removed a finished task's entry from `self.tasks` — every
        // task_spawn call over a long-lived Interpreter's life (an HTTP
        // worker spawning a background task per request, say) would stay
        // reachable forever.
        run(r#"
h = task_spawn(fn() { return 1 })
task_wait(h)
assert task_status(h) == null "task_wait must reap a finished task's entry, not leave it queryable forever"
"#)
        .unwrap();
    }

    #[test]
    fn task_wait_does_not_reap_a_task_that_is_still_running_after_its_own_timeout() {
        // The counterpart to the reap test above: task_wait's own timeout
        // elapsing must NOT reap the entry (the task isn't actually done
        // yet) — a later task_wait/task_status must still be able to
        // observe it winding down after cancellation.
        run(r#"
h = task_spawn(fn() {
  i = 0
  while i < 100000000 {
    i = i + 1
  }
  return "unreachable"
})
r = task_wait(h, 50)
assert r.timed_out == true "sanity: this wait must actually have timed out, not completed"
assert task_status(h) != null "a task that merely timed out (not yet actually finished) must stay queryable"
task_wait(h, 5000)
"#)
        .unwrap();
    }

    #[test]
    fn check_task_capacity_rejects_once_the_limit_is_reached_and_allows_below_it() {
        // MAX_CONCURRENT_TASKS exists specifically to turn "an unbounded
        // task_spawn loop eventually panics the whole interpreter (OS
        // thread exhaustion — std::thread::spawn panics, it doesn't return
        // a Result)" into an ordinary, catchable error instead. This is the
        // exact check spawn_task_internal makes on every call, exercised
        // directly against a small limit rather than actually spawning
        // MAX_CONCURRENT_TASKS (10,000) real OS threads.
        assert!(super::check_task_capacity(0, 3).is_ok());
        assert!(super::check_task_capacity(2, 3).is_ok());
        let err = super::check_task_capacity(3, 3).unwrap_err();
        assert!(format!("{:?}", err).contains("too many concurrently-tracked tasks"));
        assert!(super::check_task_capacity(100, 3).is_err());
    }

    #[test]
    fn set_random_seed_propagates_into_a_spawned_task() {
        // Regression test: task_spawn constructs a fresh child Interpreter
        // that only copied helpers/functions/capabilities/diagnostics —
        // set_random_seed's whole point (a fully deterministic run) would
        // silently stop being true for exactly the code most likely to
        // benefit from it (concurrent work spawned during a test).
        let (_, result) = run_with_interp(
            r#"
set_random_seed(42)
h = task_spawn(fn() { return random_int(1, 1000000) })
r = task_wait(h)
set_random_seed(42)
direct = random_int(1, 1000000)
assert r.value == direct "a spawned task must inherit the parent's random seed"
"#,
        );
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn task_progress_delivers_values_emitted_while_the_task_is_still_running() {
        // The primary use case: poll task_progress WHILE the task is still
        // running (not after task_wait — see the next test for why that's
        // a separate path with its own guarantee).
        let (_, result) = run_with_interp(
            r#"
h = task_spawn(fn() {
  task_emit(1)
  sleep(0.05)
  task_emit(2)
  sleep(0.05)
  task_emit(3)
  return "done"
})
collected = []
i = 0
while i < 20 {
  collected = collected + task_progress(h)
  if task_status(h).done { break }
  sleep(0.02)
  i = i + 1
}
collected = collected + task_wait(h).progress
assert len(collected) == 3 "every emitted value must eventually be observed"
assert collected[0] == 1
assert collected[1] == 2
assert collected[2] == 3
"#,
        );
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn task_wait_result_includes_progress_never_drained_before_completion() {
        // Regression test: task_wait reaps the TaskState from the
        // Interpreter's registry immediately after building its result
        // object (see reap_finished_task) — a caller that only ever calls
        // task_wait, never polling task_progress while the task was still
        // running, would otherwise have no way to observe anything emitted
        // right before the task returned; task_progress(h) after task_wait
        // would just see an unknown handle and return an empty array.
        let (_, result) = run_with_interp(
            r#"
h = task_spawn(fn() {
  task_emit(1)
  task_emit(2)
  task_emit(3)
  return "done"
})
r = task_wait(h)
assert len(r.progress) == 3 "task_wait's own result must carry undrained progress"
assert r.progress[0] == 1
assert r.progress[1] == 2
assert r.progress[2] == 3
assert r.value == "done"
after = task_progress(h)
assert len(after) == 0 "a reaped task's handle is simply unknown, not an error"
"#,
        );
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn task_progress_drains_so_a_second_call_sees_only_whats_new() {
        let (_, result) = run_with_interp(
            r#"
h = task_spawn(fn() {
  task_emit("a")
  sleep(0.3)
  return "done"
})
sleep(0.05)
first = task_progress(h)
second = task_progress(h)
task_wait(h)
assert len(first) == 1 "first drain sees the emitted value"
assert len(second) == 0 "second drain, before more is emitted, sees nothing new"
"#,
        );
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn task_emit_outside_a_task_is_a_clear_error() {
        let result = run("task_emit(1)");
        assert!(
            result.is_err(),
            "task_emit must fail when there's no current task"
        );
    }

    #[test]
    fn task_progress_on_an_unknown_handle_returns_an_empty_array_not_an_error() {
        let (_, result) = run_with_interp(
            r#"
entries = task_progress("not-a-real-handle")
assert len(entries) == 0
"#,
        );
        assert!(result.is_ok(), "{:?}", result);
    }
}
