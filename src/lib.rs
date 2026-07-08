//! GX Runtime Library — public API for embedding GX in other programs
//! and for `gx build` to produce standalone binaries.

// Many AST fields and bridge helpers are reserved for future phases.
#![allow(dead_code)]

pub mod ai;
pub mod ast;
pub mod bridge;
pub mod capability;
pub mod indent_parser;
pub mod interpreter;
pub mod lexer;
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
