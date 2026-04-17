#![allow(dead_code)]

mod ai;
mod ast;
mod bridge;
mod indent_parser;
mod interpreter;
mod lexer;
mod parser;
mod toolchain;
mod value;

use std::fs;
use std::path::Path;
use std::process;

use indent_parser::is_indent_syntax;
use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    let result = match args[1].as_str() {
        "run" => {
            let file = require_arg(&args, 2, "gx run <file.gx>");
            let debug = args.contains(&"--debug".to_string());
            cmd_run(file, debug)
        }
        "check" => {
            let file = require_arg(&args, 2, "gx check <file.gx>");
            cmd_check(file)
        }
        "init" => {
            let name = require_arg(&args, 2, "gx init <project-name>");
            toolchain::init(name)
        }
        "build" => {
            let file = require_arg(&args, 2, "gx build <file.gx>");
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            toolchain::build(file, output)
        }
        "install" => {
            let pkg = require_arg(&args, 2, "gx install <js.package|py.package>");
            toolchain::install(pkg)
        }
        "fmt" => {
            let file = require_arg(&args, 2, "gx fmt <file.gx>");
            toolchain::fmt(file)
        }
        "make" => {
            let desc = require_arg(&args, 2, "gx make \"description of what to build\"");
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            toolchain::make(desc, output)
        }
        "test" => {
            let path = args.get(2).map(|s| s.as_str());
            toolchain::test(path)
        }
        "version" | "--version" | "-v" => {
            println!("gx {}", VERSION);
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        // Shorthand: gx file.gx
        file if file.ends_with(".gx") => {
            let debug = args.contains(&"--debug".to_string());
            cmd_run(file, debug)
        }
        cmd => {
            eprintln!("gx: unknown command '{}'\n", cmd);
            print_usage();
            process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn parse_file(source: &str, path: &str) -> Result<crate::ast::Program, String> {
    if is_indent_syntax(source) {
        indent_parser::parse(source).map_err(|e| format!("{}: {}", path, e))
    } else {
        let tokens = Lexer::new(source)
            .tokenize()
            .map_err(|e| format!("{}: {}", path, e))?;
        Parser::new(tokens)
            .parse()
            .map_err(|e| format!("{}: {}", path, e))
    }
}

fn cmd_run(path: &str, debug: bool) -> Result<(), String> {
    let source = read_file(path)?;
    if debug {
        eprintln!("[gx] file: {}", path);
        eprintln!(
            "[gx] syntax: {}",
            if is_indent_syntax(&source) {
                "indentation"
            } else {
                "brace"
            }
        );
    }

    let program = parse_file(&source, path)?;
    if debug {
        eprintln!("[gx] helpers: {}", program.helpers.len());
        for h in &program.helpers {
            eprintln!("[gx]   - {}", h.name);
        }
        if !program.imports.is_empty() {
            for i in &program.imports {
                eprintln!("[gx]   use {}.{}", i.namespace, i.package);
            }
        }
    }

    Interpreter::new()
        .run_program(&program)
        .map_err(|e| format!("{}: {}", path, e))
}

fn cmd_check(path: &str) -> Result<(), String> {
    let source = read_file(path)?;
    let program = parse_file(&source, path)?;

    println!(
        "{}: OK ({} helper{}, {} import{})",
        path,
        program.helpers.len(),
        if program.helpers.len() == 1 { "" } else { "s" },
        program.imports.len(),
        if program.imports.len() == 1 { "" } else { "s" },
    );
    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn read_file(path: &str) -> Result<String, String> {
    if !Path::new(path).exists() {
        return Err(format!("file not found: {}", path));
    }
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path, e))
}

fn require_arg<'a>(args: &'a [String], idx: usize, usage: &str) -> &'a str {
    args.get(idx).map(|s| s.as_str()).unwrap_or_else(|| {
        eprintln!("Error: missing argument\nUsage: {}", usage);
        process::exit(1);
    })
}

fn print_usage() {
    eprintln!("Usage: gx <command> [options]");
    eprintln!("       gx <file.gx>");
    eprintln!();
    eprintln!("Run 'gx help' for all commands.");
}

fn print_help() {
    println!("GX Language v{}", VERSION);
    println!("Brain-first programming language for building transparent AI assistants");
    println!();
    println!("USAGE:");
    println!("  gx run <file.gx> [--debug]        Run a GX program");
    println!("  gx check <file.gx>                 Check syntax without running");
    println!("  gx init <name>                     Create a new GX project");
    println!("  gx build <file.gx> [-o name]       Build standalone executable");
    println!("  gx install <js.pkg|py.pkg>         Install a package");
    println!("  gx fmt <file.gx>                   Format GX source code");
    println!("  gx make \"description\" [-o file]    AI-generate GX code from description");
    println!("  gx test [dir]                      Run test files in tests/ directory");
    println!("  gx version                         Show version");
    println!("  gx help                            Show this help");
    println!();
    println!("EXAMPLES:");
    println!("  gx run main.gx");
    println!("  gx init my-agent && cd my-agent && gx run main.gx");
    println!("  gx make \"a weather bot that checks London daily\" -o weather.gx");
    println!("  gx install js.axios");
    println!("  gx build main.gx && ./dist/main");
    println!();
    println!("AI PROVIDERS (set env vars):");
    println!("  OPENAI_API_KEY=sk-...      OpenAI (gpt-4o-mini default)");
    println!("  ANTHROPIC_API_KEY=sk-...   Anthropic Claude");
    println!("  OLLAMA_URL=http://...      Ollama local (default: localhost:11434)");
    println!();
    println!("LANGUAGE QUICK REFERENCE:");
    println!("  agent \"name\" {{ ... }}      Define an AI agent");
    println!("  when started {{ ... }}      Run on startup");
    println!("  remember {{ key = val }}    Agent memory");
    println!("  brain {{ plan execute      Full cognitive cycle");
    println!("         remember communicate }}");
    println!("  result = ask openai {{ prompt: \"...\" }}");
    println!("  say result.text            Print AI response");
    println!("  use js.axios               Import npm package");
    println!("  use py.requests            Import Python package");
    println!();
    println!("  Docs: MASTER_PLAN.md  |  Examples: docs/examples/");
}
