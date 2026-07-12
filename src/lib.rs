//! GX Runtime Library — public API for embedding GX in other programs
//! and for `gx build` to produce standalone binaries.
//!
//! **Stack size**: GX's tree-walking interpreter costs several real Rust
//! stack frames per one GX function/closure call (`eval_call` →
//! `call_user_function`/`call_closure_with_capture` → `run_stmts` →
//! `run_stmt` → `eval_expr` → back to `eval_call` for a recursive call).
//! `Interpreter::run_program`/`run_source` enforce a recursion-depth limit
//! (`MAX_CALL_DEPTH`, currently 1000) specifically so unbounded GX
//! recursion is a graceful, catchable GX error rather than a process-
//! aborting stack overflow — but that guarantee only holds if the calling
//! thread has enough real stack to reach that depth safely. The `gx`
//! binary's own `main()`, `task_spawn`'s worker threads, and the HTTP
//! server's worker pool all run on a thread with an explicitly enlarged
//! stack (`interpreter::WORKER_THREAD_STACK_SIZE`, 256 MiB) for exactly
//! this reason — confirmed empirically that the platform default thread
//! stack overflows (a real, uncatchable process abort) after well under
//! 100 levels of GX recursion.
//!
//! **A caller embedding this crate directly and invoking
//! `run_source`/`Interpreter::run_program` on its own thread should do the
//! same**, rather than calling it on a thread with an unknown or small
//! default stack. `std::thread::Builder::new().stack_size(...)` is the
//! tool for this; `interpreter::WORKER_THREAD_STACK_SIZE` is the size this
//! crate's own entry points use.

// Many AST fields and bridge helpers are reserved for future phases.
#![allow(dead_code)]

pub mod ai;
pub mod ast;
pub mod bridge;
pub mod capability;
pub mod checker;
pub mod diagnostics;
pub mod diagnostics_render;
pub mod indent_parser;
pub mod interpreter;
pub mod lexer;
#[cfg(not(target_arch = "wasm32"))]
pub mod package;
pub mod parser;
pub mod toolchain;
pub mod value;
pub mod wasm;

use indent_parser::is_indent_syntax;
use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

/// Parse GX source using whichever syntax the file uses.
pub fn parse_source(source: &str) -> Result<crate::ast::Program, String> {
    if is_indent_syntax(source) {
        indent_parser::parse(source)
    } else {
        let tokens = Lexer::new(source).tokenize()?;
        Parser::new(tokens).parse()
    }
}

/// Run GX source code from a string. Prints output to stdout.
/// Returns Ok(()) or an error message.
pub fn run_source(source: &str) -> Result<(), String> {
    let program = parse_source(source)?;
    let mut interp = Interpreter::new();
    interp.run_program(&program)
}

/// Check GX source syntax without executing.
pub fn check_source(source: &str) -> Result<usize, String> {
    let program = parse_source(source)?;
    Ok(program.helpers.len())
}

/// Converts a script-supplied "seconds" value into a `Duration`, safe
/// against every input `Duration::from_secs_f64` itself panics on: NaN,
/// negative values, and any magnitude exceeding `Duration::MAX` — including
/// `Infinity`, reachable from an ordinary GX numeric literal with enough
/// digits to overflow `f64` during parsing (no division-by-zero trick
/// needed). Every `sleep()`/`timeout`/`task_wait` call site across the
/// interpreter converts a script-provided seconds value this way instead of
/// calling `Duration::from_secs_f64` directly, so a value that used to crash
/// the whole process now just clamps to `[0, max]` — an unbounded
/// sleep/timeout being its own denial-of-service is exactly why callers
/// pass a real `max` rather than `Duration::MAX`.
pub fn clamp_duration_secs(secs: f64, max: std::time::Duration) -> std::time::Duration {
    if secs.is_nan() {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f64(secs.clamp(0.0, max.as_secs_f64()))
}

#[cfg(test)]
mod clamp_duration_secs_tests {
    use super::clamp_duration_secs;
    use std::time::Duration;

    #[test]
    fn nan_clamps_to_zero_instead_of_panicking() {
        assert_eq!(
            clamp_duration_secs(f64::NAN, Duration::from_secs(60)),
            Duration::ZERO
        );
    }

    #[test]
    fn positive_infinity_clamps_to_max_instead_of_panicking() {
        assert_eq!(
            clamp_duration_secs(f64::INFINITY, Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn negative_infinity_clamps_to_zero_instead_of_panicking() {
        assert_eq!(
            clamp_duration_secs(f64::NEG_INFINITY, Duration::from_secs(60)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_finite_value_far_beyond_duration_max_clamps_to_max_instead_of_panicking() {
        assert_eq!(
            clamp_duration_secs(1e300, Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn a_negative_value_clamps_to_zero() {
        assert_eq!(
            clamp_duration_secs(-5.0, Duration::from_secs(60)),
            Duration::ZERO
        );
    }

    #[test]
    fn an_ordinary_in_range_value_passes_through_unchanged() {
        assert_eq!(
            clamp_duration_secs(3.5, Duration::from_secs(60)),
            Duration::from_secs_f64(3.5)
        );
    }
}
