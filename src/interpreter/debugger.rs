//! Production Debugger Runtime — breakpoints, stepping, and an interactive
//! inspection prompt built directly on `Interpreter::run_stmt`, the same
//! per-statement checkpoint the Task Runtime already uses for cooperative
//! cancellation. No redesign of the tree-walking execution loop: pausing is
//! just another check `run_stmt` makes before running a statement, exactly
//! like the existing cancellation check right above it.
//!
//! This module holds the pure, easily-testable pieces (state, the pause
//! decision, command parsing). The actual interactive prompt — which needs
//! `&mut Interpreter`/`&mut Env` and blocking stdin reads — lives in
//! `Interpreter::debug_pause` in `mod.rs`, mirroring how `main.rs`'s REPL
//! keeps its own command parsing (`run_repl_command`) separate from the I/O
//! loop that drives it.

use std::collections::HashSet;

/// Whether — and how — the debugger is currently watching execution.
/// `Off` is the default and costs nothing beyond a single enum comparison
/// per statement, matching `Diagnostics::is_enabled()`'s "cheap when
/// unused" gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugMode {
    /// No debugger attached. `run_stmt`'s pause check short-circuits
    /// immediately.
    #[default]
    Off,
    /// A debug session is active (either a `--break` line list was given,
    /// or a `breakpoint()` call was hit at least once and later
    /// `continue`d) but not currently single-stepping — only lines in
    /// `break_lines` trigger a pause.
    Running,
    /// Single-stepping: the *next* statement anywhere pauses, regardless
    /// of `break_lines`.
    StepInto,
}

/// Debugger state carried on `Interpreter`. Deliberately flat (no `Option`
/// wrapper) — `mode: Off` is itself the "no debugger" state, so there is
/// always exactly one place to check rather than an `Option` layered on
/// top of an already-off mode.
#[derive(Debug, Clone, Default)]
pub struct DebugState {
    pub mode: DebugMode,
    /// External line-number breakpoints, e.g. from `gx run --break 4,9`.
    pub break_lines: HashSet<usize>,
    /// Expressions re-evaluated and printed at every pause, most recently
    /// added last — accumulated via the `watch <expr>` command.
    pub watches: Vec<String>,
}

impl DebugState {
    pub fn new() -> Self {
        Self::default()
    }

    /// What `mode` should become after the user resumes (`continue`) from
    /// a pause: back to actively watching `break_lines` if any are
    /// configured, or fully `Off` (zero per-statement overhead) if this
    /// debug session only ever existed because of an embedded
    /// `breakpoint()` call with no external breakpoints set.
    pub fn mode_after_continue(&self) -> DebugMode {
        if self.break_lines.is_empty() {
            DebugMode::Off
        } else {
            DebugMode::Running
        }
    }
}

/// Whether a statement at `line` should pause execution, given the current
/// debug state. Pure and independent of any I/O, so it's fully unit
/// testable without a fake stdin.
pub fn should_pause(state: &DebugState, line: usize) -> bool {
    match state.mode {
        DebugMode::Off => false,
        DebugMode::StepInto => true,
        DebugMode::Running => state.break_lines.contains(&line),
    }
}

/// A parsed debugger-prompt command. Mirrors `main.rs`'s REPL `:`-command
/// shape (a name plus an optional rest-of-line argument), but these are
/// bare words (`continue`, `step`, ...) rather than `:`-prefixed, since the
/// debugger prompt never needs to disambiguate against GX syntax the way
/// the REPL's ordinary input line does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugCommand {
    Continue,
    Step,
    Locals,
    Stack,
    Print(String),
    Watch(String),
    Quit,
    Help,
    /// Blank input at the prompt — re-prompt without printing anything.
    Empty,
    Unknown(String),
}

/// Parse one line typed at the `(gx-debug)` prompt. Every command accepts
/// both a short form (`c`, `s`, `l`, `bt`, `p`, `w`, `q`, `h`) and a long
/// form (`continue`, `step`, `locals`, `stack`, `print`, `watch`, `quit`,
/// `help`) — short forms for a developer who's already in a debugging
/// session and typing many commands, long forms for discoverability.
pub fn parse_debug_command(input: &str) -> DebugCommand {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return DebugCommand::Empty;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match cmd {
        "c" | "continue" => DebugCommand::Continue,
        "s" | "step" => DebugCommand::Step,
        "l" | "locals" => DebugCommand::Locals,
        "bt" | "stack" | "backtrace" => DebugCommand::Stack,
        "p" | "print" if !rest.is_empty() => DebugCommand::Print(rest.to_string()),
        "w" | "watch" if !rest.is_empty() => DebugCommand::Watch(rest.to_string()),
        "q" | "quit" => DebugCommand::Quit,
        "h" | "help" | "?" => DebugCommand::Help,
        _ => DebugCommand::Unknown(trimmed.to_string()),
    }
}

/// Parse a comma-separated `--break 4,9,12` CLI argument into line numbers.
/// Blank entries (a trailing comma, doubled commas) are skipped rather than
/// erroring — a minor formatting slip in a flag argument shouldn't abort
/// the whole command when the intent is unambiguous.
pub fn parse_break_lines(arg: &str) -> Result<HashSet<usize>, String> {
    let mut lines = HashSet::new();
    for part in arg.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let n: usize = part
            .parse()
            .map_err(|_| format!("--break: '{}' is not a valid line number", part))?;
        lines.insert(n);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pause_is_false_when_debugger_is_off() {
        let mut state = DebugState::new();
        state.break_lines.insert(5);
        assert!(!should_pause(&state, 5));
    }

    #[test]
    fn should_pause_true_at_a_matching_break_line_when_running() {
        let mut state = DebugState::new();
        state.mode = DebugMode::Running;
        state.break_lines.insert(5);
        assert!(should_pause(&state, 5));
        assert!(!should_pause(&state, 6));
    }

    #[test]
    fn should_pause_always_true_in_step_into_regardless_of_break_lines() {
        let mut state = DebugState::new();
        state.mode = DebugMode::StepInto;
        assert!(should_pause(&state, 1));
        assert!(should_pause(&state, 999));
    }

    #[test]
    fn mode_after_continue_goes_fully_off_with_no_break_lines() {
        let state = DebugState::new();
        assert_eq!(state.mode_after_continue(), DebugMode::Off);
    }

    #[test]
    fn mode_after_continue_stays_running_when_break_lines_remain() {
        let mut state = DebugState::new();
        state.break_lines.insert(3);
        assert_eq!(state.mode_after_continue(), DebugMode::Running);
    }

    #[test]
    fn parse_debug_command_accepts_short_and_long_forms() {
        assert_eq!(parse_debug_command("c"), DebugCommand::Continue);
        assert_eq!(parse_debug_command("continue"), DebugCommand::Continue);
        assert_eq!(parse_debug_command("s"), DebugCommand::Step);
        assert_eq!(parse_debug_command("step"), DebugCommand::Step);
        assert_eq!(parse_debug_command("l"), DebugCommand::Locals);
        assert_eq!(parse_debug_command("locals"), DebugCommand::Locals);
        assert_eq!(parse_debug_command("bt"), DebugCommand::Stack);
        assert_eq!(parse_debug_command("stack"), DebugCommand::Stack);
        assert_eq!(parse_debug_command("q"), DebugCommand::Quit);
        assert_eq!(parse_debug_command("quit"), DebugCommand::Quit);
        assert_eq!(parse_debug_command("h"), DebugCommand::Help);
        assert_eq!(parse_debug_command(""), DebugCommand::Empty);
        assert_eq!(parse_debug_command("   "), DebugCommand::Empty);
    }

    #[test]
    fn parse_debug_command_extracts_the_expression_for_print_and_watch() {
        assert_eq!(
            parse_debug_command("p x + 1"),
            DebugCommand::Print("x + 1".to_string())
        );
        assert_eq!(
            parse_debug_command("print user.name"),
            DebugCommand::Print("user.name".to_string())
        );
        assert_eq!(
            parse_debug_command("w total"),
            DebugCommand::Watch("total".to_string())
        );
    }

    #[test]
    fn parse_debug_command_print_and_watch_need_an_argument() {
        // `p`/`w` with nothing after it isn't a valid Print/Watch — falls
        // through to Unknown rather than trying to evaluate an empty
        // expression.
        assert_eq!(
            parse_debug_command("p"),
            DebugCommand::Unknown("p".to_string())
        );
        assert_eq!(
            parse_debug_command("w"),
            DebugCommand::Unknown("w".to_string())
        );
    }

    #[test]
    fn parse_debug_command_unknown_for_garbage_input() {
        assert_eq!(
            parse_debug_command("frobnicate"),
            DebugCommand::Unknown("frobnicate".to_string())
        );
    }

    #[test]
    fn parse_break_lines_collects_every_number() {
        let lines = parse_break_lines("4,9,12").unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines.contains(&4) && lines.contains(&9) && lines.contains(&12));
    }

    #[test]
    fn parse_break_lines_tolerates_whitespace_and_trailing_commas() {
        let lines = parse_break_lines(" 4, 9, ").unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines.contains(&4) && lines.contains(&9));
    }

    #[test]
    fn parse_break_lines_rejects_a_non_numeric_entry() {
        assert!(parse_break_lines("4,abc,9").is_err());
    }
}
