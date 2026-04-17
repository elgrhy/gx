//! GX Runtime Library — public API for embedding GX in other programs
//! and for `gx build` to produce standalone binaries.

// Many AST fields and bridge helpers are reserved for future phases.
#![allow(dead_code)]

pub mod ai;
pub mod ast;
pub mod bridge;
pub mod interpreter;
pub mod lexer;
pub mod parser;
pub mod toolchain;
pub mod value;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

/// Run GX source code from a string. Prints output to stdout.
/// Returns Ok(()) or an error message.
pub fn run_source(source: &str) -> Result<(), String> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    Interpreter::new().run_program(&program)
}

/// Check GX source syntax without executing.
pub fn check_source(source: &str) -> Result<usize, String> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    Ok(program.helpers.len())
}
