//! GX Runtime Library — public API for embedding GX in other programs
//! and for `gx build` to produce standalone binaries.

// Many AST fields and bridge helpers are reserved for future phases.
#![allow(dead_code)]

pub mod lexer;
pub mod ast;
pub mod parser;
pub mod value;
pub mod interpreter;
pub mod ai;
pub mod bridge;
pub mod toolchain;

use lexer::Lexer;
use parser::Parser;
use interpreter::Interpreter;

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
