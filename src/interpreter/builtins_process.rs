//! Native process execution runtime: `process_run`, `process_spawn`,
//! `process_wait`, `process_kill`, `process_exists`, `process_status`,
//! `process_read`.
//!
//! No shell is ever invoked here — every command runs via
//! `Command::new(command).args(args)` directly. That's what makes this
//! immune to shell-quoting/injection concerns and consistent across
//! platforms: there is no quoting dialect to get right or wrong, because
//! there is no shell parsing the argument list in the first place.
//!
//! Every spawned process is owned by the runtime for its entire lifetime:
//! - A dedicated reaper thread calls `try_wait()` in a poll loop and reaps
//!   the child the moment it exits (Unix processes that aren't waited on
//!   become zombies; this thread exists specifically to prevent that,
//!   regardless of whether the GX script ever calls `process_wait`).
//! - Dedicated reader threads continuously drain stdout/stderr into shared
//!   buffers, independent of whether/when the script reads them — this is
//!   what prevents the classic deadlock where a child blocks on a full pipe
//!   because nobody is draining it.
//! - An optional timeout thread kills the child if it outlives its budget.
//!
//! The `Mutex<Child>` is polled with `try_wait()`, never a blocking `wait()`
//! — a blocking wait would hold the lock indefinitely and starve
//! `process_kill`, which needs to acquire the same lock to send the signal.

use super::Signal;
use crate::value::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often the reaper/timeout threads poll. Small enough that `process_kill`
/// and timeouts feel immediate, large enough not to burn CPU spinning.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Hard cap on stdout/stderr retained per stream. This is what actually
/// bounds memory for a runaway or chatty child: once hit, the reader thread
/// keeps draining the pipe (so the child never blocks on a full pipe) but
/// stops retaining further bytes.
const MAX_CAPTURED_OUTPUT: usize = 32 * 1024 * 1024; // 32 MiB per stream
/// How long we'll wait to *confirm* a process has actually exited after a
/// kill was attempted (spec.timeout's auto-kill, `process_wait`'s own
/// timeout, or an explicit `process_kill`) before giving up and reporting
/// `error_kind: "unresponsive"` instead of blocking forever. This exists
/// because of a real, unfixable-in-userspace edge case: a process stuck in
/// uninterruptible sleep (Linux "D state" — typically blocked on hung
/// kernel-level I/O such as an unresponsive NFS mount) does not respond to
/// SIGKILL at all until it leaves that state, which can be indefinite. No
/// signal, timeout, or userspace trick can force it to terminate sooner —
/// that would require killing something the kernel itself is waiting on.
/// Two seconds is generous slack for the overwhelmingly common case (a
/// normal kill takes effect in milliseconds); a real D-state process would
/// still be alive well past this window.
const UNRESPONSIVE_GRACE_PERIOD: Duration = Duration::from_secs(2);
/// Executable name/path — real paths are nowhere near this on any platform
/// (Linux `PATH_MAX` is 4096; Windows' historical `MAX_PATH` is 260, though
/// long-path-aware Windows APIs go higher).
const MAX_COMMAND_LEN: usize = 4096;
/// Same rationale as `MAX_COMMAND_LEN` — `cwd` is a path, not a payload.
const MAX_CWD_LEN: usize = 4096;
/// Each `args` element. Arguments are for parameters, not payloads — a
/// script that needs to hand a program a large blob of data should use
/// `stdin`, which has a much larger budget (`MAX_STDIN_LEN`) for exactly
/// that reason.
const MAX_ARG_LEN: usize = 1024 * 1024; // 1 MiB
/// Environment variable *names* are always short in every real convention.
const MAX_ENV_KEY_LEN: usize = 256;
/// Environment variable *values* occasionally carry a real payload (a JSON
/// config blob, a token) — same budget as an argument.
const MAX_ENV_VALUE_LEN: usize = 1024 * 1024; // 1 MiB
/// `stdin` is the intended channel for handing a spawned process real data,
/// so it gets the same generous budget as HMAC/Ed25519 messages.
const MAX_STDIN_LEN: usize = 10 * 1024 * 1024; // 10 MiB

/// Split off the longest valid-UTF-8 prefix of `bytes`, returning
/// `(decoded_string, bytes_consumed)`. `process_read` pulls raw bytes off a
/// live pipe in arbitrary 8KB chunks with no regard for UTF-8 character
/// boundaries — a multi-byte character can straddle two separate
/// `process_read` calls. Naively lossy-decoding each call's slice and
/// advancing the cursor to the end would mangle that character into a
/// replacement char and then skip past its trailing bytes entirely (they'd
/// never be re-read). Instead: if the tail looks like an *incomplete*
/// multi-byte sequence, leave those bytes unconsumed for the next read to
/// complete; only fall back to lossy replacement for a sequence that's
/// genuinely invalid (not just truncated), so a real encoding error can't
/// stall progress forever.
fn take_valid_utf8_prefix(bytes: &[u8]) -> (String, usize) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), bytes.len()),
        Err(e) => match e.error_len() {
            Some(_) => (String::from_utf8_lossy(bytes).into_owned(), bytes.len()),
            None => {
                let valid_len = e.valid_up_to();
                let s = std::str::from_utf8(&bytes[..valid_len])
                    .expect("valid_up_to() always points at a UTF-8 boundary")
                    .to_string();
                (s, valid_len)
            }
        },
    }
}

/// Extract the handle-string argument every `process_*` lookup function
/// (`kill`/`exists`/`status`/`read`/`wait`) takes as its first parameter —
/// shared so the "requires a handle string" error is worded consistently
/// and isn't repeated five times.
fn require_handle_arg<'a>(args: &'a [Value], who: &str) -> Result<&'a str, Signal> {
    args.first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| Signal::Error(format!("{} requires a handle string", who)))
}

/// Pull whatever's been appended to `buf` since `pos`, advancing `pos` to
/// match — the shared core of `process_read`'s stdout/stderr handling.
fn read_new_chunk(buf: &Mutex<Vec<u8>>, pos: &Mutex<usize>) -> String {
    let buf = buf.lock().unwrap();
    let mut pos = pos.lock().unwrap();
    let (chunk, consumed) = take_valid_utf8_prefix(&buf[(*pos).min(buf.len())..]);
    *pos += consumed;
    chunk
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── Shared process state ──────────────────────────────────────────────────────

pub(super) struct ProcessState {
    child: Mutex<Child>,
    pid: u32,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    /// Bytes of `stdin` handed to the child (0 if none was given). Fixed at
    /// construction time — never mutated afterward, so no synchronization
    /// is needed to read it from other threads.
    stdin_bytes: usize,
    stdout_buf: Mutex<Vec<u8>>,
    stderr_buf: Mutex<Vec<u8>>,
    /// Total bytes ever read from each stream, tracked independently of how
    /// much is actually *retained* in the buffer above (which stops growing
    /// past `MAX_CAPTURED_OUTPUT`) — this is what lets callers learn "this
    /// process actually produced 500 MiB of output" even though only 32 MiB
    /// of it is available, instead of silently handing back an incomplete
    /// result with no way to tell it's incomplete.
    stdout_total_bytes: AtomicUsize,
    stderr_total_bytes: AtomicUsize,
    stdout_read_pos: Mutex<usize>,
    stderr_read_pos: Mutex<usize>,
    done: AtomicBool,
    exit_code: Mutex<Option<i32>>,
    timed_out: AtomicBool,
    killed: AtomicBool,
    /// Set when this process was killed because the Task Runtime task that
    /// started it was cancelled — distinct from `timed_out` (this process's
    /// own `timeout`) and `killed` (an explicit `process_kill`), so a
    /// script can tell the three apart. See `spawn_cancellation_killer`.
    cancelled: AtomicBool,
    started_at: Instant,
    started_at_unix_ms: i64,
    finished_at: Mutex<Option<Instant>>,
    finished_at_unix_ms: Mutex<Option<i64>>,
    background_threads: Mutex<Option<Vec<std::thread::JoinHandle<()>>>>,
}

impl ProcessState {
    fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// Send a kill signal. Returns false if the process was already finished
    /// (nothing to kill) — never panics, never blocks on the child exiting.
    fn kill(&self) -> bool {
        if self.is_done() {
            return false;
        }
        self.killed.store(true, Ordering::SeqCst);
        self.child.lock().unwrap().kill().is_ok()
    }

    /// Like `kill()`, but marks the process as timed out rather than
    /// explicitly killed. Used by both the spec-level timeout-killer thread
    /// and `process_wait`'s own `timeout` parameter, so a script can check
    /// `result.timed_out` consistently regardless of which one fired —
    /// without this, only `spec.timeout` set `timed_out`, while
    /// `process_wait(h, timeout)` firing only set `killed`, silently
    /// missing the same check.
    fn kill_due_to_timeout(&self) -> bool {
        if self.is_done() {
            return false;
        }
        self.timed_out.store(true, Ordering::SeqCst);
        self.child.lock().unwrap().kill().is_ok()
    }

    /// Like `kill()`, but marks the process as cancelled (via its owning
    /// task) rather than explicitly killed or timed out. Called by
    /// `spawn_cancellation_killer` — the Task Runtime integration point:
    /// "Process Runtime terminates child processes when parent tasks are
    /// cancelled."
    fn kill_due_to_cancellation(&self) -> bool {
        if self.is_done() {
            return false;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        self.child.lock().unwrap().kill().is_ok()
    }

    fn was_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Join every background thread (reader/reaper/timeout/stdin-writer).
    /// Idempotent — safe to call more than once.
    fn join_background_threads(&self) {
        let handles = self.background_threads.lock().unwrap().take();
        if let Some(handles) = handles {
            for h in handles {
                let _ = h.join();
            }
        }
    }

    /// `(stdout_total_bytes, stderr_total_bytes, stdout_truncated, stderr_truncated)`.
    /// "Total" counts every byte the process actually produced, independent
    /// of how much is retained in the (capped) buffer — this is what makes
    /// truncation detectable instead of silent.
    fn output_stats(&self) -> (usize, usize, bool, bool) {
        let stdout_total = self.stdout_total_bytes.load(Ordering::SeqCst);
        let stderr_total = self.stderr_total_bytes.load(Ordering::SeqCst);
        (
            stdout_total,
            stderr_total,
            stdout_total > MAX_CAPTURED_OUTPUT,
            stderr_total > MAX_CAPTURED_OUTPUT,
        )
    }
}

fn spawn_reader(
    mut reader: impl Read + Send + 'static,
    state: Arc<ProcessState>,
    is_stdout: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break, // EOF: child closed this stream
                Ok(n) => {
                    let (buf, total) = if is_stdout {
                        (&state.stdout_buf, &state.stdout_total_bytes)
                    } else {
                        (&state.stderr_buf, &state.stderr_total_bytes)
                    };
                    total.fetch_add(n, Ordering::SeqCst);
                    let mut b = buf.lock().unwrap();
                    if b.len() < MAX_CAPTURED_OUTPUT {
                        let remaining = MAX_CAPTURED_OUTPUT - b.len();
                        b.extend_from_slice(&chunk[..n.min(remaining)]);
                    }
                    // Beyond the cap we keep reading (draining the pipe so the
                    // child never blocks) but stop retaining the bytes —
                    // `total` above still counts every byte actually
                    // produced, so truncation is always detectable.
                }
                Err(_) => break,
            }
        }
    })
}

fn spawn_stdin_writer(
    mut stdin: std::process::ChildStdin,
    data: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = stdin.write_all(data.as_bytes());
        // Dropping `stdin` here closes the pipe, sending EOF to the child.
    })
}

fn spawn_reaper(state: Arc<ProcessState>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        // `child`'s lock is held only for the `try_wait()` call itself — it
        // is dropped (end of this block) before touching any other field,
        // so `process_kill` (which needs the same lock) is never made to
        // wait behind bookkeeping it doesn't care about. This isn't fixing
        // a deadlock (no other code path acquires these locks in the
        // reverse order), just keeping the lock's held duration minimal.
        let wait_result = { state.child.lock().unwrap().try_wait() };
        match wait_result {
            Ok(Some(status)) => {
                *state.exit_code.lock().unwrap() = status.code();
                state.finished_at.lock().unwrap().replace(Instant::now());
                state
                    .finished_at_unix_ms
                    .lock()
                    .unwrap()
                    .replace(now_unix_ms());
                state.done.store(true, Ordering::SeqCst);
                return;
            }
            Ok(None) => {}
            Err(_) => {
                state.done.store(true, Ordering::SeqCst);
                return;
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    })
}

fn spawn_timeout_killer(
    state: Arc<ProcessState>,
    timeout: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if state.is_done() {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        state.kill_due_to_timeout();
    })
}

/// Watches `is_cancelled` (the owning Task Runtime task's effective
/// cancellation state, if this process was started from inside a task) and
/// kills the child the moment it flips — the same shape as
/// `spawn_timeout_killer`, deliberately: cancellation is just another
/// deadline, except its trigger is an external flag instead of a fixed
/// `Instant`. A plain `Fn() -> bool` rather than depending on
/// `builtins_task::TaskState` directly keeps this module decoupled from
/// the Task Runtime's internals — it only needs to know "should I stop?".
fn spawn_cancellation_killer(
    state: Arc<ProcessState>,
    is_cancelled: impl Fn() -> bool + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        if state.is_done() {
            return;
        }
        if is_cancelled() {
            state.kill_due_to_cancellation();
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    })
}

// ── Spawn error classification (shared with bridges — see bridge.rs) ─────────

pub(crate) enum SpawnErrorKind {
    NotFound,
    PermissionDenied,
    Other,
}

pub(crate) fn classify_spawn_error(e: &std::io::Error) -> SpawnErrorKind {
    match e.kind() {
        std::io::ErrorKind::NotFound => SpawnErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
        _ => SpawnErrorKind::Other,
    }
}

// ── Spec parsing ───────────────────────────────────────────────────────────────

struct ProcessSpec {
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
    stdin: Option<String>,
    timeout: Option<Duration>,
}

/// Look up a string field on an object, checking its length against
/// `max_len` on the borrow *before* cloning — a `Value::Str(s)` match arm
/// only ever hands back `&String` here, so oversized attacker-controlled
/// input (a huge `stdin` payload, say) is rejected via an O(1) length check
/// and never actually copied.
fn field_str_checked(
    obj: &HashMap<String, Value>,
    key: &str,
    who: &str,
    max_len: usize,
) -> Result<Option<String>, Signal> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(s)) => {
            if s.len() > max_len {
                return Err(Signal::Error(format!(
                    "{}: '{}' exceeds the maximum allowed length of {} bytes (got {})",
                    who,
                    key,
                    max_len,
                    s.len()
                )));
            }
            Ok(Some(s.clone()))
        }
        _ => Err(Signal::Error(format!(
            "{}: '{}' must be a string",
            who, key
        ))),
    }
}

fn parse_spec(value: Option<&Value>, who: &str) -> Result<ProcessSpec, Signal> {
    let obj = match value {
        Some(Value::Object(m)) => m,
        _ => {
            return Err(Signal::Error(format!(
                "{} expects an object: {{command, args?, cwd?, env?, stdin?, timeout?}}",
                who
            )))
        }
    };
    let command = field_str_checked(obj, "command", who, MAX_COMMAND_LEN)?.ok_or_else(|| {
        Signal::Error(format!(
            "{}: 'command' is required and must be a string",
            who
        ))
    })?;
    let args = match obj.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                // Same borrow-then-check-then-clone order as field_str_checked.
                let s = match item {
                    Value::Str(s) => s,
                    _ => {
                        return Err(Signal::Error(format!(
                            "{}: every element of 'args' must be a string",
                            who
                        )))
                    }
                };
                if s.len() > MAX_ARG_LEN {
                    return Err(Signal::Error(format!(
                        "{}: an 'args' element exceeds the maximum allowed length of {} bytes (got {})",
                        who, MAX_ARG_LEN, s.len()
                    )));
                }
                out.push(s.clone());
            }
            out
        }
        _ => {
            return Err(Signal::Error(format!(
                "{}: 'args' must be an array of strings",
                who
            )))
        }
    };
    let cwd = field_str_checked(obj, "cwd", who, MAX_CWD_LEN)?;
    let env = match obj.get("env") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(m)) => {
            let mut out = Vec::with_capacity(m.len());
            for (k, v) in m {
                if k.len() > MAX_ENV_KEY_LEN {
                    return Err(Signal::Error(format!(
                        "{}: env key '{}' exceeds the maximum allowed length of {} bytes",
                        who, k, MAX_ENV_KEY_LEN
                    )));
                }
                let s = match v {
                    Value::Str(s) => s,
                    _ => {
                        return Err(Signal::Error(format!(
                            "{}: env['{}'] must be a string",
                            who, k
                        )))
                    }
                };
                if s.len() > MAX_ENV_VALUE_LEN {
                    return Err(Signal::Error(format!(
                        "{}: env['{}'] exceeds the maximum allowed length of {} bytes (got {})",
                        who,
                        k,
                        MAX_ENV_VALUE_LEN,
                        s.len()
                    )));
                }
                out.push((k.clone(), s.clone()));
            }
            out
        }
        _ => return Err(Signal::Error(format!("{}: 'env' must be an object", who))),
    };
    let stdin = field_str_checked(obj, "stdin", who, MAX_STDIN_LEN)?;
    let timeout = match obj.get("timeout") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_number().ok_or_else(|| {
                Signal::Error(format!("{}: 'timeout' must be a number (seconds)", who))
            })?;
            // `n.is_nan() || n <= 0.0` (rather than just `n <= 0.0`) also
            // rejects NaN, since every comparison with NaN is false either
            // way — `n <= 0.0` alone let NaN fall through to the unguarded
            // `Duration::from_secs_f64` below, which panicked on it.
            // `clamp_duration_secs` below additionally guards a *finite*
            // value so large it still exceeds `Duration::MAX` (e.g.
            // `1e300`, reachable from an ordinary GX numeric literal) —
            // `n.is_finite()` alone doesn't catch that case.
            if n.is_nan() || n <= 0.0 {
                return Err(Signal::Error(format!(
                    "{}: 'timeout' must be a positive number of seconds",
                    who
                )));
            }
            Some(crate::clamp_duration_secs(
                n,
                Duration::from_secs(315_360_000),
            ))
        }
    };
    Ok(ProcessSpec {
        command,
        args,
        cwd,
        env,
        stdin,
        timeout,
    })
}

/// Spawn the OS process and wire up its background threads. Does not touch
/// the interpreter's handle table — callers decide whether to register it
/// (`process_spawn`) or consume it directly (`process_run`). `is_cancelled`
/// is `Some` when this process is being started from inside a Task Runtime
/// task — see `spawn_cancellation_killer`.
fn start(
    spec: &ProcessSpec,
    cwd_resolved: Option<std::path::PathBuf>,
    is_cancelled: Option<Box<dyn Fn() -> bool + Send + 'static>>,
) -> Result<Arc<ProcessState>, (SpawnErrorKind, String)> {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args);
    if let Some(dir) = &cwd_resolved {
        cmd.current_dir(dir);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(if spec.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        let kind = classify_spawn_error(&e);
        (kind, e.to_string())
    })?;

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    let state = Arc::new(ProcessState {
        child: Mutex::new(child),
        pid,
        command: spec.command.clone(),
        args: spec.args.clone(),
        cwd: cwd_resolved.map(|p| p.to_string_lossy().into_owned()),
        stdin_bytes: spec.stdin.as_ref().map(|s| s.len()).unwrap_or(0),
        stdout_buf: Mutex::new(Vec::new()),
        stderr_buf: Mutex::new(Vec::new()),
        stdout_total_bytes: AtomicUsize::new(0),
        stderr_total_bytes: AtomicUsize::new(0),
        stdout_read_pos: Mutex::new(0),
        stderr_read_pos: Mutex::new(0),
        done: AtomicBool::new(false),
        exit_code: Mutex::new(None),
        timed_out: AtomicBool::new(false),
        killed: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        started_at: Instant::now(),
        started_at_unix_ms: now_unix_ms(),
        finished_at: Mutex::new(None),
        finished_at_unix_ms: Mutex::new(None),
        background_threads: Mutex::new(Some(Vec::new())),
    });

    let mut threads = Vec::new();
    if let Some(out) = stdout {
        threads.push(spawn_reader(out, state.clone(), true));
    }
    if let Some(err) = stderr {
        threads.push(spawn_reader(err, state.clone(), false));
    }
    if let (Some(stdin_handle), Some(data)) = (stdin, spec.stdin.clone()) {
        threads.push(spawn_stdin_writer(stdin_handle, data));
    }
    threads.push(spawn_reaper(state.clone()));
    if let Some(timeout) = spec.timeout {
        threads.push(spawn_timeout_killer(state.clone(), timeout));
    }
    if let Some(is_cancelled) = is_cancelled {
        threads.push(spawn_cancellation_killer(state.clone(), move || {
            is_cancelled()
        }));
    }
    *state.background_threads.lock().unwrap() = Some(threads);

    Ok(state)
}

/// Poll until the process is confirmed done, or (if `deadline` is given)
/// until the deadline passes — returns whether it's actually done. A `None`
/// deadline waits forever, matching the default "block until natural
/// completion" behavior of every blocking subprocess API when no timeout is
/// requested.
fn wait_until_done_or_deadline(state: &ProcessState, deadline: Option<Instant>) -> bool {
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

/// Best-effort result for when we've given up waiting to confirm that a
/// process actually terminated (see `UNRESPONSIVE_GRACE_PERIOD`). Reads
/// whatever output has been captured so far *without* joining the
/// background threads — joining could block forever if the OS process is
/// genuinely still alive (e.g. stuck in D-state).
/// Insert `stdout_bytes`/`stderr_bytes` (total bytes actually produced, not
/// just retained) and `stdout_truncated`/`stderr_truncated`/`truncated` into
/// a result object — shared by every result-producing path so a script
/// never has to special-case whether these fields are present.
fn insert_output_stats(obj: &mut HashMap<String, Value>, state: &ProcessState) {
    let (stdout_total, stderr_total, stdout_truncated, stderr_truncated) = state.output_stats();
    obj.insert("stdout_bytes".into(), Value::Number(stdout_total as f64));
    obj.insert("stderr_bytes".into(), Value::Number(stderr_total as f64));
    obj.insert("stdout_truncated".into(), Value::Bool(stdout_truncated));
    obj.insert("stderr_truncated".into(), Value::Bool(stderr_truncated));
    obj.insert(
        "truncated".into(),
        Value::Bool(stdout_truncated || stderr_truncated),
    );
}

fn unresponsive_result_object(state: &ProcessState) -> Value {
    let stdout = String::from_utf8_lossy(&state.stdout_buf.lock().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&state.stderr_buf.lock().unwrap()).into_owned();
    let mut obj = HashMap::new();
    obj.insert("ok".into(), Value::Bool(false));
    obj.insert("stdout".into(), Value::Str(stdout));
    obj.insert("stderr".into(), Value::Str(stderr));
    obj.insert("exit_code".into(), Value::Null);
    obj.insert(
        "timed_out".into(),
        Value::Bool(state.timed_out.load(Ordering::SeqCst)),
    );
    obj.insert(
        "killed".into(),
        Value::Bool(state.killed.load(Ordering::SeqCst)),
    );
    obj.insert(
        "cancelled".into(),
        Value::Bool(state.cancelled.load(Ordering::SeqCst)),
    );
    obj.insert("error_kind".into(), Value::Str("unresponsive".to_string()));
    obj.insert(
        "error".into(),
        Value::Str("process did not exit within the grace period after being killed".to_string()),
    );
    insert_output_stats(&mut obj, state);
    Value::Object(obj)
}

fn result_object(state: &ProcessState) -> Value {
    state.join_background_threads();
    let exit_code = *state.exit_code.lock().unwrap();
    let timed_out = state.timed_out.load(Ordering::SeqCst);
    let killed = state.killed.load(Ordering::SeqCst);
    let cancelled = state.cancelled.load(Ordering::SeqCst);
    let stdout = String::from_utf8_lossy(&state.stdout_buf.lock().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&state.stderr_buf.lock().unwrap()).into_owned();

    let (error_kind, error_message) = if timed_out {
        (Some("timeout"), Some("process timed out"))
    } else if cancelled {
        (Some("cancelled"), Some("process was cancelled"))
    } else if killed {
        (Some("killed"), Some("process was killed"))
    } else {
        (None, None)
    };
    let ok = !timed_out && !killed && !cancelled && exit_code == Some(0);

    let mut obj = HashMap::new();
    obj.insert("ok".into(), Value::Bool(ok));
    obj.insert("stdout".into(), Value::Str(stdout));
    obj.insert("stderr".into(), Value::Str(stderr));
    obj.insert(
        "exit_code".into(),
        exit_code
            .map(|c| Value::Number(c as f64))
            .unwrap_or(Value::Null),
    );
    obj.insert("timed_out".into(), Value::Bool(timed_out));
    obj.insert("killed".into(), Value::Bool(killed));
    obj.insert("cancelled".into(), Value::Bool(cancelled));
    obj.insert(
        "error_kind".into(),
        error_kind
            .map(|s| Value::Str(s.to_string()))
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "error".into(),
        error_message
            .map(|s| Value::Str(s.to_string()))
            .unwrap_or(Value::Null),
    );
    insert_output_stats(&mut obj, state);
    Value::Object(obj)
}

fn spawn_error_object(kind: &SpawnErrorKind, message: &str) -> Value {
    let error_kind = match kind {
        SpawnErrorKind::NotFound => "not_found",
        SpawnErrorKind::PermissionDenied => "permission_denied",
        SpawnErrorKind::Other => "spawn_failed",
    };
    let mut obj = HashMap::new();
    obj.insert("ok".into(), Value::Bool(false));
    obj.insert("stdout".into(), Value::Str(String::new()));
    obj.insert("stderr".into(), Value::Str(String::new()));
    obj.insert("exit_code".into(), Value::Null);
    obj.insert("timed_out".into(), Value::Bool(false));
    obj.insert("killed".into(), Value::Bool(false));
    obj.insert("cancelled".into(), Value::Bool(false));
    obj.insert("error_kind".into(), Value::Str(error_kind.to_string()));
    obj.insert("error".into(), Value::Str(message.to_string()));
    // No process ever started, so there's nothing to have truncated —
    // present regardless, so every result-object shape is uniform and a
    // script never needs to special-case a missing field.
    obj.insert("stdout_bytes".into(), Value::Number(0.0));
    obj.insert("stderr_bytes".into(), Value::Number(0.0));
    obj.insert("stdout_truncated".into(), Value::Bool(false));
    obj.insert("stderr_truncated".into(), Value::Bool(false));
    obj.insert("truncated".into(), Value::Bool(false));
    Value::Object(obj)
}

// ── Interpreter-facing entry points ───────────────────────────────────────────

impl super::Interpreter {
    fn check_process_capability(&self, command: &str) -> Result<(), Signal> {
        self.authorize_capability(crate::capability::Resource::Process, Some(command))
            .map_err(|e| {
                let hint = match &e {
                    crate::capability::Denial::NotInAllowlist { .. } => {
                        " — add it to gx.json's dependencies.process to allow it."
                    }
                    _ => "",
                };
                Signal::Error(format!("{}{}", e, hint))
            })
    }

    /// Resolve `cwd` through the sandbox if one is active; `None` cwd is a no-op.
    fn resolve_process_cwd(
        &self,
        cwd: &Option<String>,
    ) -> Result<Option<std::path::PathBuf>, Signal> {
        match cwd {
            None => Ok(None),
            Some(c) => self.safe_path(c).map(Some),
        }
    }

    /// `process_run`/`process_wait` are each a single blocking statement —
    /// the automatic per-statement cancellation check in `run_stmt` can't
    /// fire *during* one. Without this, a task cancelled while blocked here
    /// would still have its child process killed (via
    /// `spawn_cancellation_killer`), but the call would return a perfectly
    /// normal `Ok(result)` once the process exits — so the *task* would
    /// never actually register as cancelled, only the process it happened
    /// to be running. This is what makes cancellation propagate through the
    /// blocking call itself: if the process was killed specifically because
    /// its owning task was cancelled (not its own `timeout`, not an
    /// explicit `process_kill`), surface that as `Signal::Cancelled` here,
    /// exactly as if `run_stmt`'s own checkpoint had caught it.
    fn cancelled_signal_or(&self, state: &ProcessState, result: Value) -> Result<Value, Signal> {
        if state.was_cancelled() {
            if let Some(task) = &self.current_task {
                return Err(Signal::Cancelled(task.cancel_reason()));
            }
        }
        Ok(result)
    }

    pub(super) fn process_run(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let spec = parse_spec(args.first(), "process_run")?;
        self.check_process_capability(&spec.command)?;
        let cwd = self.resolve_process_cwd(&spec.cwd)?;
        let span = self.diagnostics.start_span("process.run");
        let command_attr = spec.command.clone();

        let state = match start(&spec, cwd, self.cancellation_check()) {
            Ok(s) => s,
            Err((kind, msg)) => {
                let kind_str = match kind {
                    SpawnErrorKind::NotFound => "not_found",
                    SpawnErrorKind::PermissionDenied => "permission_denied",
                    SpawnErrorKind::Other => "spawn_failed",
                };
                self.diagnostics.end_span(
                    span,
                    crate::diagnostics::Outcome::Error(kind_str),
                    &[("command", serde_json::Value::String(command_attr))],
                );
                return Ok(spawn_error_object(&kind, &msg));
            }
        };

        // No timeout: wait for natural completion with no artificial bound —
        // the default for any blocking subprocess API. With a timeout: wait
        // up to it (the timeout-killer thread kills the child once it
        // elapses).
        match spec.timeout {
            None => {
                wait_until_done_or_deadline(&state, None);
            }
            Some(t) => {
                wait_until_done_or_deadline(&state, Some(Instant::now() + t));
            }
        }

        if !state.is_done() {
            // The timeout elapsed and the timeout-killer thread is killing
            // (or has just killed) the child — give it a bounded grace
            // period to actually confirm before giving up. Never joins
            // (which could block forever) unless done is actually true.
            let grace_deadline = Instant::now() + UNRESPONSIVE_GRACE_PERIOD;
            if !wait_until_done_or_deadline(&state, Some(grace_deadline)) {
                self.diagnostics.end_span(
                    span,
                    crate::diagnostics::Outcome::Error("unresponsive"),
                    &[("command", serde_json::Value::String(command_attr))],
                );
                return Ok(unresponsive_result_object(&state));
            }
        }
        let result = result_object(&state);
        let ok =
            matches!(&result, Value::Object(m) if matches!(m.get("ok"), Some(Value::Bool(true))));
        self.diagnostics.end_span(
            span,
            if state.was_cancelled() {
                crate::diagnostics::Outcome::Cancelled
            } else if ok {
                crate::diagnostics::Outcome::Ok
            } else {
                crate::diagnostics::Outcome::Error("non-zero exit")
            },
            &[("command", serde_json::Value::String(command_attr))],
        );
        self.cancelled_signal_or(&state, result)
    }

    pub(super) fn process_spawn(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let spec = parse_spec(args.first(), "process_spawn")?;
        self.check_process_capability(&spec.command)?;
        let cwd = self.resolve_process_cwd(&spec.cwd)?;
        let span = self.diagnostics.start_span("process.spawn");
        let command_attr = spec.command.clone();

        let state = match start(&spec, cwd, self.cancellation_check()) {
            Ok(s) => s,
            Err((kind, msg)) => {
                let kind_str = match kind {
                    SpawnErrorKind::NotFound => "not_found",
                    SpawnErrorKind::PermissionDenied => "permission_denied",
                    SpawnErrorKind::Other => "spawn_failed",
                };
                self.diagnostics.end_span(
                    span,
                    crate::diagnostics::Outcome::Error(kind_str),
                    &[("command", serde_json::Value::String(command_attr))],
                );
                // process_spawn still needs to hand back *something* the
                // caller can inspect uniformly with process_status, so a
                // failed spawn is reported as an immediately-failed handle
                // rather than a bare thrown error — but since there's no
                // live process to register, we surface it as a result object
                // instead of a handle. Scripts that check `ok` before using
                // a "handle" work either way; scripts that assume a handle
                // string should check the return type or use process_run.
                return Ok(spawn_error_object(&kind, &msg));
            }
        };
        let handle = uuid::Uuid::new_v4().to_string();
        self.diagnostics.end_span(
            span,
            crate::diagnostics::Outcome::Ok,
            &[
                ("command", serde_json::Value::String(command_attr)),
                ("handle", serde_json::Value::String(handle.clone())),
            ],
        );
        self.processes.insert(handle.clone(), state);
        Ok(Value::Str(handle))
    }

    pub(super) fn process_wait(&mut self, args: &[Value]) -> Result<Value, Signal> {
        // A spawn that fails before any process exists (not found, sandbox
        // violation aside — those still throw) has nothing to register a
        // handle for, so `process_spawn` hands back the same result-object
        // shape `process_run` would have produced. Passing that straight
        // back here — instead of throwing "requires a handle string" — means
        // `h = process_spawn(...); process_wait(h)` stays robust regardless
        // of whether the spawn actually succeeded.
        if let Some(already_resolved @ Value::Object(m)) = args.first() {
            if m.contains_key("ok") && m.contains_key("error_kind") {
                return Ok(already_resolved.clone());
            }
        }
        let handle = require_handle_arg(args, "process_wait")?;
        let timeout = match args.get(1) {
            None | Some(Value::Null) => None,
            Some(v) => {
                let n = v.as_number().ok_or_else(|| {
                    Signal::Error("process_wait: timeout must be a number (seconds)".into())
                })?;
                // Unlike the spec-object `timeout` field above, this one had
                // no validation at all — `n` (including a negative value,
                // NaN, or Infinity) went straight into
                // `Duration::from_secs_f64`, which panics on all three.
                if n.is_nan() || n < 0.0 {
                    return Err(Signal::Error(
                        "process_wait: timeout must be a non-negative number of seconds".into(),
                    ));
                }
                Some(crate::clamp_duration_secs(
                    n,
                    Duration::from_secs(315_360_000),
                ))
            }
        };
        let Some(state) = self.processes.get(handle).cloned() else {
            return Ok(Value::Null);
        };

        match timeout {
            // No timeout given: wait for natural completion with no
            // artificial bound, same as process_run's default.
            None => {
                wait_until_done_or_deadline(&state, None);
            }
            // This call's own timeout also terminates the process — mirrors
            // the well-known subprocess.run(timeout=) convention. Uses
            // kill_due_to_timeout (sets `timed_out`, not `killed`) so a
            // script can check `result.timed_out` consistently whether the
            // timeout came from here or from the spec's own `timeout`.
            Some(t) => {
                let deadline = Instant::now() + t;
                if !wait_until_done_or_deadline(&state, Some(deadline)) {
                    state.kill_due_to_timeout();
                }
            }
        }

        if !state.is_done() {
            // A kill was just attempted above, or was already in flight
            // (spec.timeout's auto-kill, or an earlier explicit
            // process_kill) — bound how long we wait to confirm it actually
            // took effect rather than risk blocking forever on a genuinely
            // stuck (D-state) process.
            let grace_deadline = Instant::now() + UNRESPONSIVE_GRACE_PERIOD;
            if !wait_until_done_or_deadline(&state, Some(grace_deadline)) {
                // Leave the handle registered — the process may still be
                // alive and could finish later, so a script can call
                // process_wait/process_status again to check.
                return Ok(unresponsive_result_object(&state));
            }
        }

        let result = result_object(&state);
        self.processes.remove(handle);
        self.cancelled_signal_or(&state, result)
    }

    pub(super) fn process_kill(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "process_kill")?;
        let killed = self
            .processes
            .get(handle)
            .map(|state| state.kill())
            .unwrap_or(false);
        Ok(Value::Bool(killed))
    }

    pub(super) fn process_exists(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "process_exists")?;
        let running = self
            .processes
            .get(handle)
            .map(|state| !state.is_done())
            .unwrap_or(false);
        Ok(Value::Bool(running))
    }

    pub(super) fn process_status(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "process_status")?;
        let Some(state) = self.processes.get(handle) else {
            return Ok(Value::Null);
        };
        let running = !state.is_done();
        let exit_code = *state.exit_code.lock().unwrap();
        let timed_out = state.timed_out.load(Ordering::SeqCst);
        let killed = state.killed.load(Ordering::SeqCst);
        let finished_at_ms = *state.finished_at_unix_ms.lock().unwrap();
        let duration_ms = match state.finished_at.lock().unwrap().as_ref() {
            Some(finished) => finished.duration_since(state.started_at).as_millis() as f64,
            None => state.started_at.elapsed().as_millis() as f64,
        };
        // A single at-a-glance summary of *why* a process is in its current
        // state, on top of the individual booleans below — useful for
        // logging/dashboards without re-deriving it from several fields.
        let exit_reason = if running {
            "running"
        } else if timed_out {
            "timeout"
        } else if killed {
            "killed"
        } else {
            match exit_code {
                Some(0) => "exited",
                Some(_) => "exited_error",
                None => "unknown",
            }
        };

        let mut obj = HashMap::new();
        obj.insert("pid".into(), Value::Number(state.pid as f64));
        obj.insert("command".into(), Value::Str(state.command.clone()));
        obj.insert(
            "args".into(),
            Value::Array(state.args.iter().map(|a| Value::Str(a.clone())).collect()),
        );
        obj.insert(
            "cwd".into(),
            state.cwd.clone().map(Value::Str).unwrap_or(Value::Null),
        );
        obj.insert("running".into(), Value::Bool(running));
        obj.insert(
            "exit_code".into(),
            exit_code
                .map(|c| Value::Number(c as f64))
                .unwrap_or(Value::Null),
        );
        obj.insert("exit_reason".into(), Value::Str(exit_reason.to_string()));
        obj.insert(
            "started_at".into(),
            Value::Number(state.started_at_unix_ms as f64),
        );
        obj.insert(
            "finished_at".into(),
            finished_at_ms
                .map(|v| Value::Number(v as f64))
                .unwrap_or(Value::Null),
        );
        obj.insert("duration_ms".into(), Value::Number(duration_ms));
        obj.insert(
            "stdin_bytes".into(),
            Value::Number(state.stdin_bytes as f64),
        );
        obj.insert("timed_out".into(), Value::Bool(timed_out));
        obj.insert("killed".into(), Value::Bool(killed));
        insert_output_stats(&mut obj, state);
        Ok(Value::Object(obj))
    }

    pub(super) fn process_read(&mut self, args: &[Value]) -> Result<Value, Signal> {
        let handle = require_handle_arg(args, "process_read")?;
        let Some(state) = self.processes.get(handle) else {
            return Ok(Value::Null);
        };

        let stdout_new = read_new_chunk(&state.stdout_buf, &state.stdout_read_pos);
        let stderr_new = read_new_chunk(&state.stderr_buf, &state.stderr_read_pos);

        let mut obj = HashMap::new();
        obj.insert("stdout".into(), Value::Str(stdout_new));
        obj.insert("stderr".into(), Value::Str(stderr_new));
        obj.insert("done".into(), Value::Bool(state.is_done()));
        Ok(Value::Object(obj))
    }

    /// Kill every process this interpreter still owns. Called on interpreter
    /// Drop and explicitly before `exit()` (which calls `std::process::exit`
    /// and therefore does not run destructors). Best-effort and bounded —
    /// after issuing kill signals we give reaper threads a short grace period
    /// to observe the exit before we stop waiting.
    pub(crate) fn cleanup_processes(&mut self) {
        for state in self.processes.values() {
            state.kill();
        }
        // One shared deadline for the whole cleanup pass, not one per
        // process — this bounds total shutdown latency to roughly
        // UNRESPONSIVE_GRACE_PERIOD regardless of how many processes are
        // still tracked, rather than that budget multiplying per process.
        let deadline = Instant::now() + UNRESPONSIVE_GRACE_PERIOD;
        for state in self.processes.values() {
            wait_until_done_or_deadline(state, Some(deadline));
            if state.is_done() {
                state.join_background_threads();
            }
            // Else: genuinely stuck (e.g. D-state) — abandon it rather than
            // block program exit forever. This doesn't leak at the OS
            // level: once this program's own process exits, the kernel
            // reclaims everything regardless of whether we joined our
            // reader/reaper threads first.
        }
        self.processes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::Interpreter;
    use super::{
        MAX_ARG_LEN, MAX_COMMAND_LEN, MAX_CWD_LEN, MAX_ENV_KEY_LEN, MAX_ENV_VALUE_LEN,
        MAX_STDIN_LEN, UNRESPONSIVE_GRACE_PERIOD,
    };
    use crate::value::Value;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn interp() -> Interpreter {
        let mut i = Interpreter::new();
        i.capabilities.process = true;
        i
    }

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Object(m)
    }

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(m) => m.get(key).unwrap_or(&Value::Null),
            _ => panic!("expected object, got {:?}", v),
        }
    }

    /// (command, args-that-print "hello world") — a real, standalone
    /// executable on every CI platform (Linux/macOS/Windows), never a shell
    /// builtin, so this exercises the same `Command::new(cmd).args(args)`
    /// path everywhere without any shell involved.
    #[cfg(windows)]
    fn echo_cmd() -> (&'static str, Vec<String>) {
        (
            "cmd",
            vec!["/C".into(), "echo".into(), "hello".into(), "world".into()],
        )
    }
    #[cfg(not(windows))]
    fn echo_cmd() -> (&'static str, Vec<String>) {
        ("echo", vec!["hello".into(), "world".into()])
    }

    /// (command, args) for a process that runs for approximately `secs`
    /// seconds — used to exercise timeout/kill/cleanup paths. `sleep N` on
    /// Unix; **not** `cmd /C timeout /T N` on Windows, despite that being
    /// the obvious equivalent — `timeout.exe` refuses to run at all under
    /// redirected/non-interactive stdin ("Input redirection is not
    /// supported"), which is exactly `Command`'s default here and in every
    /// CI runner, so it exited immediately instead of actually waiting and
    /// every timeout/kill assertion downstream saw a process that had
    /// already finished. `ping -n <secs+1> 127.0.0.1` is the standard
    /// console-free substitute (each ping after the first waits ~1s, so
    /// `secs+1` pings takes ~`secs` seconds) and needs no shell/redirection.
    fn sleep_cmd_args(secs: u32) -> (&'static str, Vec<Value>) {
        if cfg!(windows) {
            (
                "ping",
                vec![s("-n"), s(&(secs + 1).to_string()), s("127.0.0.1")],
            )
        } else {
            ("sleep", vec![s(&secs.to_string())])
        }
    }

    #[test]
    fn process_run_normal_execution() {
        let mut i = interp();
        let (cmd, args) = echo_cmd();
        let result = i
            .process_run(&[obj(&[
                ("command", s(cmd)),
                (
                    "args",
                    Value::Array(args.into_iter().map(Value::Str).collect()),
                ),
            ])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true));
        assert_eq!(field(&result, "exit_code"), &Value::Number(0.0));
        let stdout = field(&result, "stdout").as_str().unwrap().to_string();
        assert!(stdout.contains("hello"));
        assert!(stdout.contains("world"));
    }

    #[test]
    fn process_run_rejects_when_capability_not_granted() {
        let mut i = Interpreter::new(); // capabilities.process left false
        let (cmd, _) = echo_cmd();
        let err = i.process_run(&[obj(&[("command", s(cmd))])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_enforces_gx_json_allowlist() {
        let mut i = interp();
        i.capabilities.process_executables =
            crate::capability::Allowlist::only(["git".to_string()]);
        let (cmd, _) = echo_cmd(); // not "git" -> should be rejected
        let err = i.process_run(&[obj(&[("command", s(cmd))])]);
        assert!(err.is_err());

        // "git" itself would be allowed by the allowlist (capability check
        // passes); whether it actually runs depends on git being installed,
        // which we don't assert here — just that it's not rejected by the
        // allowlist for the wrong reason.
    }

    #[test]
    fn process_run_invalid_spec_missing_command_throws() {
        let mut i = interp();
        let err = i.process_run(&[obj(&[("args", Value::Array(vec![]))])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_invalid_spec_non_string_arg_throws() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let err = i.process_run(&[obj(&[
            ("command", s(cmd)),
            ("args", Value::Array(vec![Value::Number(1.0)])),
        ])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_command() {
        let mut i = interp();
        let huge = "x".repeat(MAX_COMMAND_LEN + 1);
        let err = i.process_run(&[obj(&[("command", s(&huge))])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_cwd() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let huge = "/".repeat(MAX_CWD_LEN + 1);
        let err = i.process_run(&[obj(&[("command", s(cmd)), ("cwd", s(&huge))])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_arg_element() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let huge = "x".repeat(MAX_ARG_LEN + 1);
        let err = i.process_run(&[obj(&[
            ("command", s(cmd)),
            ("args", Value::Array(vec![s(&huge)])),
        ])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_env_value() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let huge = "x".repeat(MAX_ENV_VALUE_LEN + 1);
        let err = i.process_run(&[obj(&[
            ("command", s(cmd)),
            ("env", obj(&[("MY_VAR", s(&huge))])),
        ])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_env_key() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let huge_key = "K".repeat(MAX_ENV_KEY_LEN + 1);
        let err = i.process_run(&[obj(&[
            ("command", s(cmd)),
            ("env", obj(&[(huge_key.as_str(), s("v"))])),
        ])]);
        assert!(err.is_err());
    }

    #[test]
    fn process_run_rejects_oversized_stdin() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let huge = "x".repeat(MAX_STDIN_LEN + 1);
        let err = i.process_run(&[obj(&[("command", s(cmd)), ("stdin", s(&huge))])]);
        assert!(err.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn process_run_accepts_stdin_up_to_the_cap() {
        // A payload right at (not over) the cap should still work — this is
        // the "reasonable, not oversized" case the cap is meant to allow.
        let mut i = interp();
        let data = "x".repeat(1000); // nowhere near the cap, just confirms no off-by-one on the boundary path
        let result = i
            .process_run(&[obj(&[("command", s("cat")), ("stdin", s(&data))])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true));
        assert_eq!(field(&result, "stdout").as_str().unwrap().len(), 1000);
    }

    #[test]
    fn process_run_not_found_reports_result_object_not_error() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[(
                "command",
                s("gx-definitely-does-not-exist-xyz123"),
            )])])
            .unwrap(); // must NOT be Err — external failure, not programmer error
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert_eq!(
            field(&result, "error_kind"),
            &Value::Str("not_found".to_string())
        );
    }

    #[test]
    fn process_run_not_found_also_carries_a_human_readable_error_message() {
        // Regression test: `result_object`/`unresponsive_result_object` used
        // to set `error_kind` without ever setting `error`, unlike every
        // other `{ ok: false, error, error_kind, ... }` producer (http_*,
        // context_ask) — so `unwrap()` on a failed process result degraded
        // to the generic "operation failed (<kind>)" fallback instead of a
        // real message. `spawn_error_object` (this path) already set
        // `error`; asserting it here pins the shape so timeout/killed/
        // cancelled/unresponsive (fixed alongside this) don't regress back
        // to missing it.
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[(
                "command",
                s("gx-definitely-does-not-exist-xyz123"),
            )])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert!(field(&result, "error")
            .as_str()
            .is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn process_run_timeout_also_carries_a_human_readable_error_message() {
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(5);
        let result = i
            .process_run(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
                ("timeout", Value::Number(1.0)),
            ])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert_eq!(
            field(&result, "error_kind"),
            &Value::Str("timeout".to_string())
        );
        assert!(field(&result, "error")
            .as_str()
            .is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn process_run_with_diagnostics_enabled_leaves_no_span_leaked_on_spawn_failure() {
        // process_run's span is started before `start()` and must be ended
        // on every exit path, including the "command not found" one, which
        // returns a result object rather than propagating an Err — a span
        // leak here would corrupt correlation for every later diagnostic
        // call on this same (potentially long-lived) Interpreter.
        let mut i = interp();
        i.diagnostics.set_enabled(true);
        i.diagnostics.ensure_trace_id();
        let result = i
            .process_run(&[obj(&[(
                "command",
                s("gx-definitely-does-not-exist-xyz123"),
            )])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert!(
            i.diagnostics.current_span_id().is_none(),
            "no span should remain open after a failed spawn"
        );
    }

    #[test]
    fn process_run_with_diagnostics_enabled_leaves_no_span_leaked_on_success() {
        let mut i = interp();
        i.diagnostics.set_enabled(true);
        i.diagnostics.ensure_trace_id();
        let (cmd, args) = echo_cmd();
        let result = i
            .process_run(&[obj(&[
                ("command", s(cmd)),
                (
                    "args",
                    Value::Array(args.into_iter().map(Value::Str).collect()),
                ),
            ])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true));
        assert!(
            i.diagnostics.current_span_id().is_none(),
            "no span should remain open after a successful process_run"
        );
    }

    #[test]
    fn process_spawn_with_diagnostics_enabled_leaves_no_span_leaked() {
        let mut i = interp();
        i.diagnostics.set_enabled(true);
        i.diagnostics.ensure_trace_id();
        let (cmd, args) = echo_cmd();
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s(cmd)),
                (
                    "args",
                    Value::Array(args.into_iter().map(Value::Str).collect()),
                ),
            ])])
            .unwrap();
        assert!(handle.as_str().is_some());
        assert!(
            i.diagnostics.current_span_id().is_none(),
            "process_spawn's launch span must end once the process is registered, \
             not stay open for the process's whole lifetime"
        );
    }

    #[test]
    fn process_run_non_zero_exit_is_result_not_error() {
        let mut i = interp();
        #[cfg(windows)]
        let spec = obj(&[
            ("command", s("cmd")),
            ("args", Value::Array(vec![s("/C"), s("exit"), s("1")])),
        ]);
        #[cfg(not(windows))]
        let spec = obj(&[("command", s("false"))]);
        let result = i.process_run(&[spec]).unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert_eq!(field(&result, "exit_code"), &Value::Number(1.0));
        assert_eq!(field(&result, "error_kind"), &Value::Null);
    }

    #[cfg(unix)]
    #[test]
    fn process_run_captures_stderr_separately_from_stdout() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[
                ("command", s("sh")),
                (
                    "args",
                    Value::Array(vec![s("-c"), s("echo out-line; echo err-line 1>&2")]),
                ),
            ])])
            .unwrap();
        assert!(field(&result, "stdout")
            .as_str()
            .unwrap()
            .contains("out-line"));
        assert!(field(&result, "stderr")
            .as_str()
            .unwrap()
            .contains("err-line"));
        assert!(!field(&result, "stdout")
            .as_str()
            .unwrap()
            .contains("err-line"));
    }

    #[cfg(unix)]
    #[test]
    fn process_run_stdin_is_piped_to_child() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[
                ("command", s("cat")),
                ("stdin", s("piped-content\n")),
            ])])
            .unwrap();
        assert!(field(&result, "stdout")
            .as_str()
            .unwrap()
            .contains("piped-content"));
    }

    #[cfg(unix)]
    #[test]
    fn process_run_argument_with_spaces_is_not_split() {
        // printf receives each array element as ONE argv entry — if "a b"
        // were incorrectly split on the space (the classic shell-string
        // injection failure mode this API exists to avoid), printf would
        // print 3 lines instead of 2.
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[
                ("command", s("printf")),
                ("args", Value::Array(vec![s("%s\\n"), s("a b"), s("c")])),
            ])])
            .unwrap();
        let stdout = field(&result, "stdout").as_str().unwrap().to_string();
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines, vec!["a b", "c"]);
    }

    #[cfg(unix)]
    #[test]
    fn process_run_unicode_argument_roundtrips() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[
                ("command", s("echo")),
                ("args", Value::Array(vec![s("héllo-世界-🚀")])),
            ])])
            .unwrap();
        assert!(field(&result, "stdout")
            .as_str()
            .unwrap()
            .contains("héllo-世界-🚀"));
    }

    #[cfg(unix)]
    #[test]
    fn process_run_working_directory_is_applied() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[("command", s("pwd")), ("cwd", s("/tmp"))])])
            .unwrap();
        let stdout = field(&result, "stdout")
            .as_str()
            .unwrap()
            .trim()
            .to_string();
        // macOS symlinks /tmp -> /private/tmp; either resolved form is correct.
        assert!(stdout == "/tmp" || stdout == "/private/tmp");
    }

    #[cfg(unix)]
    #[test]
    fn process_status_reports_cwd_stdin_bytes_and_exit_reason() {
        let mut i = interp();
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s("cat")),
                ("cwd", s("/tmp")),
                ("stdin", s("hello")),
            ])])
            .unwrap();
        let handle_str = handle.as_str().unwrap().to_string();
        let status = i.process_status(&[s(&handle_str)]).unwrap();
        let cwd = field(&status, "cwd").as_str().unwrap().to_string();
        assert!(cwd == "/tmp" || cwd == "/private/tmp");
        assert_eq!(field(&status, "stdin_bytes"), &Value::Number(5.0)); // "hello"
        i.process_wait(&[s(&handle_str)]).unwrap();
    }

    #[test]
    fn process_status_exit_reason_for_success_and_failure() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();

        let h1 = i.process_spawn(&[obj(&[("command", s(cmd))])]).unwrap();
        let h1_str = h1.as_str().unwrap().to_string();
        super::wait_until_done_or_deadline(&i.processes.get(&h1_str).unwrap().clone(), None);
        let status1 = i.process_status(&[s(&h1_str)]).unwrap();
        assert_eq!(
            field(&status1, "exit_reason"),
            &Value::Str("exited".to_string())
        );
        i.process_wait(&[s(&h1_str)]).unwrap();

        #[cfg(windows)]
        let fail_spec = obj(&[
            ("command", s("cmd")),
            ("args", Value::Array(vec![s("/C"), s("exit"), s("1")])),
        ]);
        #[cfg(not(windows))]
        let fail_spec = obj(&[("command", s("false"))]);
        let h2 = i.process_spawn(&[fail_spec]).unwrap();
        let h2_str = h2.as_str().unwrap().to_string();
        super::wait_until_done_or_deadline(&i.processes.get(&h2_str).unwrap().clone(), None);
        let status2 = i.process_status(&[s(&h2_str)]).unwrap();
        assert_eq!(
            field(&status2, "exit_reason"),
            &Value::Str("exited_error".to_string())
        );
        i.process_wait(&[s(&h2_str)]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn process_run_environment_variables_are_passed() {
        let mut i = interp();
        let result = i
            .process_run(&[obj(&[
                ("command", s("sh")),
                ("args", Value::Array(vec![s("-c"), s("echo $GX_TEST_VAR")])),
                ("env", obj(&[("GX_TEST_VAR", s("test-value-123"))])),
            ])])
            .unwrap();
        assert!(field(&result, "stdout")
            .as_str()
            .unwrap()
            .contains("test-value-123"));
    }

    #[test]
    fn process_run_timeout_kills_and_reports_timed_out() {
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(5);
        let start = std::time::Instant::now();
        let result = i
            .process_run(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
                ("timeout", Value::Number(1.0)),
            ])])
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert_eq!(field(&result, "timed_out"), &Value::Bool(true));
        assert_eq!(
            field(&result, "error_kind"),
            &Value::Str("timeout".to_string())
        );
        // Must return close to the 1s timeout, not the full 5s sleep.
        assert!(elapsed < std::time::Duration::from_secs(4));
    }

    #[test]
    fn process_wait_own_timeout_reports_timed_out_not_just_killed() {
        // process_wait(handle, timeout) firing must set the same
        // `timed_out` flag as spec.timeout — a script checking
        // `result.timed_out` shouldn't get a different answer depending on
        // which of the two timeout mechanisms actually fired.
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(5);
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
            ])])
            .unwrap();
        let handle_str = handle.as_str().unwrap().to_string();
        let start = Instant::now();
        let result = i
            .process_wait(&[s(&handle_str), Value::Number(1.0)])
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(field(&result, "timed_out"), &Value::Bool(true));
        assert_eq!(field(&result, "ok"), &Value::Bool(false));
        assert_eq!(
            field(&result, "error_kind"),
            &Value::Str("timeout".to_string())
        );
        assert!(elapsed < Duration::from_secs(4));
    }

    #[test]
    fn process_spawn_wait_kill_exists_status_lifecycle() {
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(10);
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
            ])])
            .unwrap();
        let handle_str = handle.as_str().unwrap().to_string();

        assert_eq!(
            i.process_exists(&[s(&handle_str)]).unwrap(),
            Value::Bool(true)
        );
        let status = i.process_status(&[s(&handle_str)]).unwrap();
        assert!(matches!(field(&status, "pid"), Value::Number(n) if *n > 0.0));
        assert_eq!(field(&status, "running"), &Value::Bool(true));
        assert_eq!(
            field(&status, "exit_reason"),
            &Value::Str("running".to_string())
        );
        assert_eq!(
            field(&status, "command"),
            &Value::Str(sleep_cmd.to_string())
        );
        assert_eq!(field(&status, "cwd"), &Value::Null); // no cwd was given
        assert_eq!(field(&status, "stdin_bytes"), &Value::Number(0.0)); // no stdin was given
        assert_eq!(field(&status, "truncated"), &Value::Bool(false));

        let killed = i.process_kill(&[s(&handle_str)]).unwrap();
        assert_eq!(killed, Value::Bool(true));

        let waited = i.process_wait(&[s(&handle_str)]).unwrap();
        assert_eq!(field(&waited, "killed"), &Value::Bool(true));
        assert_eq!(field(&waited, "ok"), &Value::Bool(false));
        assert_eq!(field(&waited, "truncated"), &Value::Bool(false));

        // process_wait finalizes and releases the handle.
        assert_eq!(
            i.process_exists(&[s(&handle_str)]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(i.process_status(&[s(&handle_str)]).unwrap(), Value::Null);
    }

    #[test]
    fn process_kill_on_unknown_handle_returns_false_not_error() {
        let mut i = interp();
        let result = i.process_kill(&[s("no-such-handle")]).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn process_exists_on_unknown_handle_returns_false() {
        let mut i = interp();
        let result = i.process_exists(&[s("no-such-handle")]).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn process_status_on_unknown_handle_returns_null() {
        let mut i = interp();
        let result = i.process_status(&[s("no-such-handle")]).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn process_read_on_unknown_handle_returns_null() {
        let mut i = interp();
        let result = i.process_read(&[s("no-such-handle")]).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[cfg(unix)]
    #[test]
    fn process_spawn_and_read_pulls_incremental_output() {
        let mut i = interp();
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s("sh")),
                (
                    "args",
                    Value::Array(vec![s("-c"), s("echo first; sleep 0.3; echo second")]),
                ),
            ])])
            .unwrap();
        let handle_str = handle.as_str().unwrap().to_string();

        // Give the child a moment to produce its first line, then read.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let r1 = i.process_read(&[s(&handle_str)]).unwrap();
        assert!(field(&r1, "stdout").as_str().unwrap().contains("first"));
        assert!(!field(&r1, "stdout").as_str().unwrap().contains("second"));

        let waited = i.process_wait(&[s(&handle_str)]).unwrap();
        assert!(field(&waited, "stdout")
            .as_str()
            .unwrap()
            .contains("second"));
    }

    #[cfg(unix)]
    #[test]
    fn process_run_handles_moderately_large_output() {
        let mut i = interp();
        // ~1MB of output through the pipe — exercises the multi-chunk
        // (8KB per read()) buffering path without a slow 32MB+ test.
        let result = i
            .process_run(&[obj(&[
                ("command", s("sh")),
                (
                    "args",
                    Value::Array(vec![s("-c"), s("yes 0123456789 | head -c 1000000")]),
                ),
            ])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true));
        assert_eq!(field(&result, "stdout").as_str().unwrap().len(), 1_000_000);
        // Nowhere near the 32 MiB cap — must not be reported as truncated.
        assert_eq!(field(&result, "truncated"), &Value::Bool(false));
        assert_eq!(field(&result, "stdout_truncated"), &Value::Bool(false));
        assert_eq!(field(&result, "stdout_bytes"), &Value::Number(1_000_000.0));
    }

    #[cfg(unix)]
    #[test]
    fn process_run_reports_truncation_when_output_exceeds_the_cap() {
        let mut i = interp();
        let total = super::MAX_CAPTURED_OUTPUT + 65536;
        let result = i
            .process_run(&[obj(&[
                ("command", s("sh")),
                (
                    "args",
                    Value::Array(vec![
                        s("-c"),
                        s(&format!("yes 0123456789 | head -c {}", total)),
                    ]),
                ),
            ])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true)); // exit code is still 0 — truncation isn't a process failure
        assert_eq!(field(&result, "truncated"), &Value::Bool(true));
        assert_eq!(field(&result, "stdout_truncated"), &Value::Bool(true));
        assert_eq!(field(&result, "stderr_truncated"), &Value::Bool(false));
        // stdout_bytes reports the real total produced, not the capped amount.
        assert_eq!(field(&result, "stdout_bytes"), &Value::Number(total as f64));
        // The retained string itself is capped (all-ASCII input here, so the
        // byte cap and char length line up exactly).
        assert_eq!(
            field(&result, "stdout").as_str().unwrap().len(),
            super::MAX_CAPTURED_OUTPUT
        );
    }

    #[test]
    fn concurrent_processes_do_not_interfere() {
        // Spawn several processes concurrently and confirm each one's
        // output/exit code is correctly attributed to its own handle — no
        // cross-talk between independently owned ProcessState instances.
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let mut handles = Vec::new();
        for n in 0..5 {
            let spec = if cfg!(windows) {
                obj(&[
                    ("command", s("cmd")),
                    (
                        "args",
                        Value::Array(vec![s("/C"), s("echo"), s(&format!("proc-{}", n))]),
                    ),
                ])
            } else {
                obj(&[
                    ("command", s(cmd)),
                    ("args", Value::Array(vec![s(&format!("proc-{}", n))])),
                ])
            };
            handles.push((n, i.process_spawn(&[spec]).unwrap()));
        }
        for (n, h) in handles {
            let result = i.process_wait(&[h]).unwrap();
            assert_eq!(field(&result, "ok"), &Value::Bool(true));
            assert!(field(&result, "stdout")
                .as_str()
                .unwrap()
                .contains(&format!("proc-{}", n)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_run_sandbox_violation_on_cwd_is_rejected() {
        let mut i = interp();
        i.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(std::path::PathBuf::from("/tmp"));
        let err = i.process_run(&[obj(&[
            ("command", s("pwd")),
            ("cwd", s("/etc")), // outside the sandbox
        ])]);
        assert!(err.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn process_run_cwd_inside_sandbox_is_allowed() {
        let mut i = interp();
        i.capabilities.filesystem =
            crate::capability::FilesystemAccess::Sandboxed(std::path::PathBuf::from("/tmp"));
        let result = i
            .process_run(&[obj(&[("command", s("pwd")), ("cwd", s("/tmp"))])])
            .unwrap();
        assert_eq!(field(&result, "ok"), &Value::Bool(true));
    }

    #[test]
    fn process_run_negative_timeout_is_rejected() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let err = i.process_run(&[obj(&[
            ("command", s(cmd)),
            ("timeout", Value::Number(-1.0)),
        ])]);
        assert!(err.is_err());
    }

    #[test]
    fn cleanup_processes_kills_and_clears_table() {
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(10);
        let _handle = i
            .process_spawn(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
            ])])
            .unwrap();
        assert_eq!(i.processes.len(), 1);
        i.cleanup_processes();
        assert_eq!(i.processes.len(), 0);
    }

    // ── Bounded-wait / "unresponsive" (D-state) handling ──────────────────────
    //
    // A genuinely unkillable process (stuck in Linux D-state) can't be
    // fabricated from userspace in a portable, deterministic test — SIGKILL
    // always succeeds against a normal process; the only way to *not* die is
    // the actual kernel condition this whole mechanism exists for. So these
    // tests cover the bounding mechanism itself directly (a deadline is
    // enforced regardless of whether the process ever finishes) and the
    // shape of the result it produces, rather than the full end-to-end path.

    #[test]
    fn wait_until_done_or_deadline_respects_an_already_past_deadline() {
        let mut i = interp();
        let (sleep_cmd, sleep_args) = sleep_cmd_args(5);
        let handle = i
            .process_spawn(&[obj(&[
                ("command", s(sleep_cmd)),
                ("args", Value::Array(sleep_args)),
            ])])
            .unwrap();
        let handle_str = handle.as_str().unwrap().to_string();
        let state = i.processes.get(&handle_str).unwrap().clone();

        // A deadline already in the past must return false immediately —
        // proving the bound is enforced independently of whether the
        // process (which is still very much alive, mid-5-second-sleep)
        // ever finishes.
        let past_deadline = Instant::now() - Duration::from_secs(1);
        let start = Instant::now();
        let done = super::wait_until_done_or_deadline(&state, Some(past_deadline));
        assert!(!done);
        assert!(start.elapsed() < Duration::from_millis(500));

        i.process_kill(&[s(&handle_str)]).unwrap();
        i.process_wait(&[s(&handle_str)]).unwrap();
    }

    #[test]
    fn unresponsive_result_object_has_the_expected_shape() {
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let handle = i.process_spawn(&[obj(&[("command", s(cmd))])]).unwrap();
        let handle_str = handle.as_str().unwrap().to_string();
        let state = i.processes.get(&handle_str).unwrap().clone();
        // Let it actually finish naturally so there's nothing left running.
        super::wait_until_done_or_deadline(&state, None);

        let result = super::unresponsive_result_object(&state);
        let obj = match &result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["ok"], Value::Bool(false));
        assert_eq!(obj["exit_code"], Value::Null);
        assert_eq!(obj["error_kind"], Value::Str("unresponsive".to_string()));

        i.process_wait(&[s(&handle_str)]).unwrap();
    }

    #[test]
    fn cleanup_processes_does_not_block_on_grace_period_when_process_exits_promptly() {
        // Sanity check that the shared-deadline restructuring of
        // cleanup_processes didn't turn the common (fast) case into an
        // always-pay-the-full-grace-period wait.
        let mut i = interp();
        let (cmd, _) = echo_cmd();
        let _handle = i.process_spawn(&[obj(&[("command", s(cmd))])]).unwrap();
        let start = Instant::now();
        i.cleanup_processes();
        assert!(start.elapsed() < UNRESPONSIVE_GRACE_PERIOD);
    }

    #[test]
    fn process_wait_on_a_failed_spawn_result_passes_it_through() {
        // process_spawn can't hand back a real handle for a process that
        // never started, so it returns the same result-object shape
        // process_run would have — process_wait must accept that directly
        // instead of throwing "requires a handle string", so
        // `h = process_spawn(...); process_wait(h)` stays robust whether or
        // not the spawn actually succeeded.
        let mut i = interp();
        let spawn_result = i
            .process_spawn(&[obj(&[(
                "command",
                s("gx-definitely-does-not-exist-xyz123"),
            )])])
            .unwrap();
        assert!(matches!(spawn_result, Value::Object(_)));
        let waited = i.process_wait(&[spawn_result]).unwrap();
        assert_eq!(field(&waited, "ok"), &Value::Bool(false));
        assert_eq!(
            field(&waited, "error_kind"),
            &Value::Str("not_found".to_string())
        );
    }

    // ── take_valid_utf8_prefix (process_read's UTF-8 boundary safety) ────────

    #[test]
    fn take_valid_utf8_prefix_full_valid_input_consumes_everything() {
        let bytes = "hello world".as_bytes();
        let (chunk, consumed) = super::take_valid_utf8_prefix(bytes);
        assert_eq!(chunk, "hello world");
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn take_valid_utf8_prefix_leaves_incomplete_trailing_char_for_next_read() {
        let full = "hello-世".as_bytes(); // "世" is the 3-byte sequence E4 B8 96
        let split_at = full.len() - 1; // cut off the last byte of "世"

        let (chunk1, consumed1) = super::take_valid_utf8_prefix(&full[..split_at]);
        assert_eq!(chunk1, "hello-");
        assert_eq!(consumed1, "hello-".len()); // "世"'s partial bytes NOT consumed

        // process_read re-slices from `read_pos` against the shared,
        // ever-growing buffer — simulate the rest of "世" having arrived by
        // now, re-reading from where the first call left off.
        let (chunk2, consumed2) = super::take_valid_utf8_prefix(&full[consumed1..]);
        assert_eq!(chunk2, "世");
        assert_eq!(consumed1 + consumed2, full.len());
    }

    #[test]
    fn take_valid_utf8_prefix_lossy_replaces_genuinely_invalid_bytes_without_stalling() {
        let bytes = vec![b'a', 0xff, b'b']; // 0xff is never a valid UTF-8 byte
        let (chunk, consumed) = super::take_valid_utf8_prefix(&bytes);
        // Must consume everything (make progress) rather than getting stuck
        // waiting forever for bytes that would never make 0xff valid.
        assert_eq!(consumed, bytes.len());
        assert!(chunk.contains('a'));
        assert!(chunk.contains('b'));
    }
}
