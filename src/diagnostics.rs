//! The Diagnostics & Observability Runtime — the single place GX emits
//! structured logs, audit events, and trace spans. Every subsystem (HTTP,
//! Process, Database, Capability, and future Task/AI runtimes) reports
//! through this module instead of writing its own ad-hoc `eprintln!`.
//!
//! Two tiers, deliberately separate:
//!
//! 1. **Structured leveled logging + audit events** — always on, filtered
//!    only by a minimum level (default `info`). Cheap: a level comparison
//!    per call, and JSON formatting only for lines that pass the filter.
//!    Every [`crate::capability::Capabilities`] denial anywhere in the
//!    runtime emits a `warn`-level audit event through this tier
//!    automatically (see `Interpreter::authorize_and_audit` in
//!    `interpreter/mod.rs`) — centralized once, not duplicated at each of
//!    the ~15 call sites that check a capability.
//!
//! 2. **Spans, trace IDs, nested timing** — opt in via `--trace`, off by
//!    default. A trace ID is minted once per true entry point (a top-level
//!    script run, each incoming HTTP request) and inherited — never
//!    regenerated — by everything nested inside it: `spawn agent`,
//!    `parallel {}`, HTTP client calls, DB calls, process calls. When
//!    disabled, starting/ending a span is a single branch and nothing
//!    else — no ID generation, no allocation, no formatting, no I/O.
//!
//! Hand-rolled against `std` + the already-present `uuid`/`serde_json`
//! dependencies rather than adopting the `tracing` crate ecosystem: GX's
//! concurrency model is per-`Interpreter`-clone (HTTP workers, spawned
//! agents, `parallel {}` each get their own `Interpreter`, not a shared
//! async runtime), which doesn't map onto `tracing`'s thread-local/async
//! span-guard model without fighting it. Every previous milestone in this
//! program hand-rolled its mechanism against `std` rather than adopting a
//! general-purpose crate for the same reason.
//!
//! Output is JSON lines to stderr — trivially consumable by any external
//! log/metrics pipeline (Vector, Fluentd, a Datadog agent, ...) without GX
//! integrating against a specific backend, which is also the natural seed
//! for future metrics/distributed-tracing export: the schema doesn't need
//! to change, only what's listening on the other end of stderr.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.to_lowercase().as_str() {
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" | "warning" => Level::Warn,
            "error" => Level::Error,
            _ => return None,
        })
    }
}

/// How a span concluded — recorded on every `end_span` call so a trace
/// tree shows not just *how long* something took but whether it worked.
pub enum Outcome<'a> {
    Ok,
    Error(&'a str),
    /// Distinct from `Error`: the span's work was deliberately stopped by
    /// the Task Runtime (a `task_cancel()` call, a deadline, or a cancelled
    /// ancestor task) rather than failing on its own. Kept separate so a
    /// trace can distinguish "this broke" from "this was told to stop" —
    /// the Task Runtime milestone is what first produces this outcome.
    Cancelled,
}

/// An in-flight span. Deliberately owns everything it needs (no borrow of
/// `Diagnostics`) so starting a span doesn't hold a live mutable borrow of
/// the interpreter for the span's whole duration — call sites that need
/// `&mut self` for other work (e.g. resolving a pooled DB connection)
/// between `start_span` and `end_span` would otherwise hit a borrow
/// conflict. `id: None` means diagnostics are disabled; every other method
/// on `Diagnostics` treats that as the fast, no-op path.
pub struct SpanHandle {
    id: Option<String>,
    parent_id: Option<String>,
    trace_id: Option<String>,
    // `Cow` rather than `&'static str`: automatic instrumentation call
    // sites (HTTP/DB/Process) pass a literal (`Cow::Borrowed`, zero-cost);
    // the manual `span("name") { ... }` statement takes a script-computed
    // name that must be an owned `String` (`Cow::Owned`) — a leaked
    // `Box::leak` per call, the tempting alternative, would grow without
    // bound for a span inside a loop with a dynamic name.
    name: std::borrow::Cow<'static, str>,
    start: Instant,
}

impl SpanHandle {
    /// A handle representing "diagnostics disabled" — cheap to construct
    /// (no allocation), and `end_span` treats it as a complete no-op.
    fn disabled() -> Self {
        SpanHandle {
            id: None,
            parent_id: None,
            trace_id: None,
            name: std::borrow::Cow::Borrowed(""),
            start: Instant::now(),
        }
    }
}

/// The observability state carried on `Interpreter` — mirrors the exact
/// "config inherited, runtime state reset-but-context-preserved" pattern
/// already established for `Capabilities` (Capability Runtime milestone)
/// and the HTTP server's worker pool: see `for_child`.
#[derive(Clone)]
pub struct Diagnostics {
    /// Tier 2 master switch (spans). Tier 1 (leveled logging/audit) is
    /// always active, filtered only by `min_level`.
    enabled: bool,
    min_level: Level,
    trace_id: Option<String>,
    /// Active span IDs, innermost last — mirrors `Interpreter::call_stack`'s
    /// existing shape for the same kind of nesting problem.
    span_stack: Vec<String>,
}

impl Diagnostics {
    /// Safe default: spans off, only info-and-above logging — matches
    /// every other GX subsystem's "off/safe by default" convention.
    pub fn new() -> Self {
        Diagnostics {
            enabled: false,
            min_level: Level::Info,
            trace_id: None,
            span_stack: Vec::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_min_level(&mut self, level: Level) {
        self.min_level = level;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Produce the `Diagnostics` a spawned child (an HTTP worker, a
    /// `spawn agent` thread, a `parallel {}` branch) should start with:
    /// same config, but the child continues the *same* logical trace
    /// (spawning is a nested operation within the caller's request, not a
    /// new one) — its spans should nest under whatever was active in the
    /// parent at the moment of spawning, not become their own root.
    pub fn for_child(&self) -> Diagnostics {
        let mut child = Diagnostics {
            enabled: self.enabled,
            min_level: self.min_level,
            trace_id: self.trace_id.clone(),
            span_stack: Vec::new(),
        };
        if let Some(parent_span) = self.span_stack.last() {
            child.span_stack.push(parent_span.clone());
        }
        child
    }

    /// Mint a trace ID if none is active yet, and return it. Called at true
    /// entry points (a top-level script run, each incoming HTTP request) —
    /// everything nested inside inherits this one via `for_child`/the
    /// shared `Interpreter`, never regenerating it.
    pub fn ensure_trace_id(&mut self) -> String {
        if let Some(id) = &self.trace_id {
            return id.clone();
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.trace_id = Some(id.clone());
        id
    }

    /// Force a specific trace ID (used when an HTTP request should start a
    /// fresh trace regardless of whatever the server-level interpreter's
    /// own trace_id was, since each request is its own logical operation).
    pub fn reset_trace_id(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.trace_id = Some(id.clone());
        self.span_stack.clear();
        id
    }

    pub fn current_trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    pub fn current_span_id(&self) -> Option<&str> {
        self.span_stack.last().map(|s| s.as_str())
    }

    /// Start a span. When disabled, this is one branch and a cheap
    /// `Instant::now()` — no ID generation, no allocation. Accepts either a
    /// `&'static str` literal (every automatic instrumentation call site —
    /// zero-cost) or an owned `String` (the manual `span("name") { ... }`
    /// statement, whose name is script-computed).
    pub fn start_span(&mut self, name: impl Into<std::borrow::Cow<'static, str>>) -> SpanHandle {
        if !self.enabled {
            return SpanHandle::disabled();
        }
        let id = uuid::Uuid::new_v4().to_string();
        let parent_id = self.span_stack.last().cloned();
        let trace_id = self.trace_id.clone();
        self.span_stack.push(id.clone());
        SpanHandle {
            id: Some(id),
            parent_id,
            trace_id,
            name: name.into(),
            start: Instant::now(),
        }
    }

    /// End a span, recording its duration and outcome. No-op immediately if
    /// the handle is the disabled placeholder. Pops the span stack
    /// defensively — searches for the handle's own ID rather than assuming
    /// it's exactly the top, so a span that (despite best effort at call
    /// sites) never got matched to its `end_span` — e.g. an early `?`
    /// return between `start_span` and the intended `end_span` — degrades
    /// to "this one span's data is imprecise" rather than corrupting every
    /// later span's parent resolution.
    pub fn end_span(
        &mut self,
        handle: SpanHandle,
        outcome: Outcome,
        attrs: &[(&str, serde_json::Value)],
    ) {
        let Some(id) = &handle.id else {
            return;
        };
        if let Some(pos) = self.span_stack.iter().rposition(|s| s == id) {
            self.span_stack.truncate(pos);
        }
        if !self.enabled {
            return;
        }
        let duration_ms = handle.start.elapsed().as_secs_f64() * 1000.0;
        let (outcome_str, error_msg) = match outcome {
            Outcome::Ok => ("ok", None),
            Outcome::Error(msg) => ("error", Some(msg)),
            Outcome::Cancelled => ("cancelled", None),
        };
        let mut data = serde_json::Map::new();
        for (k, v) in attrs {
            data.insert((*k).to_string(), v.clone());
        }
        emit(serde_json::json!({
            "ts_ms": now_ms(),
            "kind": "span",
            "name": handle.name,
            "trace_id": handle.trace_id,
            "span_id": id,
            "parent_span_id": handle.parent_id,
            "duration_ms": duration_ms,
            "outcome": outcome_str,
            "error": error_msg,
            "data": data,
        }));
    }

    /// Tier 1: structured leveled log line. Always active — filtered only
    /// by `min_level`, independent of the span/tracing `enabled` switch.
    pub fn log(&self, level: Level, message: &str, data: Option<serde_json::Value>) {
        if level < self.min_level {
            return;
        }
        emit(serde_json::json!({
            "ts_ms": now_ms(),
            "kind": "log",
            "level": level.as_str(),
            "message": message,
            "trace_id": self.trace_id,
            "span_id": self.span_stack.last(),
            "data": data,
        }));
    }

    /// Tier 1: an audit event — a point-in-time record of something
    /// security/operationally significant (a capability denial today;
    /// naturally extensible to others later), always emitted at `warn`
    /// regardless of what a caller's own `min_level` filtering might do to
    /// routine logs, since these should stay visible in production by
    /// default.
    pub fn audit(&self, event: &str, data: serde_json::Value) {
        emit(serde_json::json!({
            "ts_ms": now_ms(),
            "kind": "audit",
            "level": Level::Warn.as_str(),
            "event": event,
            "trace_id": self.trace_id,
            "span_id": self.span_stack.last(),
            "data": data,
        }));
    }

    /// A free-form named event attached to the current trace/span context
    /// — what `trace_log()` (the pre-existing, previously-unreachable
    /// builtin) now emits through, kept for backward compatibility with
    /// its documented signature. Always active like the rest of tier 1.
    pub fn event(&self, name: &str, data: serde_json::Value) {
        emit(serde_json::json!({
            "ts_ms": now_ms(),
            "kind": "event",
            "name": name,
            "trace_id": self.trace_id,
            "span_id": self.span_stack.last(),
            "data": data,
        }));
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn emit(value: serde_json::Value) {
    eprintln!("{}", value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_start_span_is_a_cheap_noop_handle() {
        let mut d = Diagnostics::new();
        let h = d.start_span("test.op");
        assert!(h.id.is_none());
        assert!(d.current_span_id().is_none());
        d.end_span(h, Outcome::Ok, &[]);
        assert!(d.current_span_id().is_none());
    }

    #[test]
    fn enabled_span_nests_correctly() {
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        let outer = d.start_span("outer");
        assert_eq!(d.current_span_id(), outer.id.as_deref());
        let inner = d.start_span("inner");
        assert_eq!(inner.parent_id.as_deref(), outer.id.as_deref());
        assert_eq!(d.current_span_id(), inner.id.as_deref());
        d.end_span(inner, Outcome::Ok, &[]);
        assert_eq!(d.current_span_id(), outer.id.as_deref());
        d.end_span(outer, Outcome::Ok, &[]);
        assert!(d.current_span_id().is_none());
    }

    #[test]
    fn all_spans_in_one_trace_share_the_same_trace_id() {
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        let trace_id = d.ensure_trace_id();
        let outer = d.start_span("outer");
        let inner = d.start_span("inner");
        assert_eq!(outer.trace_id.as_deref(), Some(trace_id.as_str()));
        assert_eq!(inner.trace_id.as_deref(), Some(trace_id.as_str()));
        d.end_span(inner, Outcome::Ok, &[]);
        d.end_span(outer, Outcome::Ok, &[]);
    }

    #[test]
    fn ensure_trace_id_is_idempotent() {
        let mut d = Diagnostics::new();
        let a = d.ensure_trace_id();
        let b = d.ensure_trace_id();
        assert_eq!(a, b);
    }

    #[test]
    fn reset_trace_id_always_mints_a_new_one_and_clears_the_stack() {
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        let a = d.ensure_trace_id();
        let _span = d.start_span("leftover");
        let b = d.reset_trace_id();
        assert_ne!(a, b);
        assert!(d.current_span_id().is_none());
    }

    #[test]
    fn for_child_inherits_config_and_trace_but_resets_stack_to_current_parent() {
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        d.set_min_level(Level::Debug);
        let trace_id = d.ensure_trace_id();
        let outer = d.start_span("outer");

        let child = d.for_child();
        assert!(child.is_enabled());
        assert_eq!(child.current_trace_id(), Some(trace_id.as_str()));
        assert_eq!(child.current_span_id(), outer.id.as_deref());

        d.end_span(outer, Outcome::Ok, &[]);
    }

    #[test]
    fn for_child_with_no_active_span_has_an_empty_stack() {
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        d.ensure_trace_id();
        let child = d.for_child();
        assert!(child.current_span_id().is_none());
    }

    #[test]
    fn end_span_self_heals_when_a_nested_span_was_never_ended() {
        // Simulates the "an early `?` return skipped an inner end_span"
        // case: the inner span is leaked, but ending the outer one must
        // not panic or corrupt the stack for whatever comes after.
        let mut d = Diagnostics::new();
        d.set_enabled(true);
        let outer = d.start_span("outer");
        let _inner_leaked = d.start_span("inner"); // never ended
        d.end_span(outer, Outcome::Ok, &[]);
        assert!(d.current_span_id().is_none());

        // The diagnostics state must still be usable afterward.
        let next = d.start_span("next");
        assert!(next.id.is_some());
        d.end_span(next, Outcome::Ok, &[]);
    }

    #[test]
    fn log_level_filtering() {
        // No direct way to assert on stderr output without capturing it;
        // this test instead exercises every level to confirm none panics
        // and relies on log_level_ordering below for the actual filter
        // logic guarantee.
        let d = Diagnostics::new();
        d.log(
            Level::Debug,
            "should be filtered by default min_level",
            None,
        );
        d.log(Level::Info, "at the default min_level", None);
        d.log(Level::Warn, "above the default min_level", None);
        d.log(Level::Error, "above the default min_level", None);
    }

    #[test]
    fn log_level_ordering() {
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
    }

    #[test]
    fn level_parse_roundtrips() {
        for l in [Level::Debug, Level::Info, Level::Warn, Level::Error] {
            assert_eq!(Level::parse(l.as_str()), Some(l));
        }
        assert_eq!(Level::parse("nonsense"), None);
        assert_eq!(Level::parse("WARN"), Some(Level::Warn));
    }
}
