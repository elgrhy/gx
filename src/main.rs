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
        "init" | "new" => {
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
            let input = require_arg(&args, 2, "gx make <spec.gx|\"description\"> [--out <dir>]");
            let out_dir = args
                .iter()
                .position(|a| a == "--out" || a == "-o" || a == "--output")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            toolchain::make(input, out_dir)
        }
        "test" => {
            let path = args.get(2).map(|s| s.as_str());
            toolchain::test(path)
        }
        "repl" => cmd_repl(),
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
    }

    let mut interp = Interpreter::new();
    interp.base_path = Some(path.to_string());
    interp
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

fn cmd_repl() -> Result<(), String> {
    use std::io::{self, BufRead, Write};

    println!("GX {} — Interactive REPL", VERSION);
    println!("Type GX code and press Enter. Type 'exit' or Ctrl+C to quit.");
    println!();

    let stdin = io::stdin();
    let mut interp = Interpreter::new();

    loop {
        print!("gx> ");
        io::stdout().flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Err(_) => break,
            Ok(_) => {}
        }

        let line = line.trim();
        if line == "exit" || line == "quit" {
            break;
        }
        if line.is_empty() {
            continue;
        }

        // Wrap bare statements in a helper for execution
        let wrapped = if line.starts_with("helper")
            || line.starts_with("agent")
            || line.starts_with("function")
        {
            line.to_string()
        } else {
            format!(
                r#"helper "__repl__" {{
  brain {{
    plan {{ }}
    execute {{ {} }}
    remember {{ }}
    communicate {{ }}
  }}
}}"#,
                line
            )
        };

        match parse_file(&wrapped, "<repl>") {
            Ok(program) => {
                if let Err(e) = interp.run_program(&program) {
                    eprintln!("Error: {}", e);
                }
            }
            Err(e) => eprintln!("Parse error: {}", e),
        }
    }

    println!("Goodbye!");
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
    println!("  gx run <file.gx> [--debug]            Run a GX program");
    println!("  gx check <file.gx>                     Check syntax without running");
    println!("  gx init <name>                         Create a new GX project");
    println!("  gx build <file.gx> [-o name]           Build standalone launcher");
    println!("  gx install <js.pkg|py.pkg>             Install a package");
    println!("  gx fmt <file.gx>                       Format GX source code");
    println!("  gx make <spec.gx|\"description\"> [--out dir]  Generate a complete project");
    println!("  gx test [dir]                          Run test files");
    println!("  gx repl                                Interactive REPL");
    println!("  gx version                             Show version");
    println!("  gx help                                Show this help");
    println!();
    println!("EXAMPLES:");
    println!("  gx run main.gx");
    println!("  gx init my-agent && cd my-agent && gx run main.gx");
    println!(
        "  gx make \"a REST todo API with Node.js\"
  gx make spec.gx --out my-project"
    );
    println!("  gx install js.axios");
    println!("  gx repl");
    println!();
    println!("AI PROVIDERS (set env vars):");
    println!("  OPENAI_API_KEY=sk-...      OpenAI (gpt-4o-mini default)");
    println!("  ANTHROPIC_API_KEY=sk-...   Anthropic Claude");
    println!("  OLLAMA_URL=http://...      Ollama local (default: localhost:11434)");
    println!();
    println!("BUILT-IN FUNCTIONS:");
    println!("  Math:    sqrt, pow, abs, floor, ceil, round, clamp, min, max, random, pi, e");
    println!("  String:  len, trim, split, contains, replace, pad_left, pad_right, repeat");
    println!("  Array:   push, pop, sort, reverse, slice, join, unique, sum, min, max");
    println!("  Object:  keys, values, entries, merge, has");
    println!("  JSON:    json_parse, json_stringify");
    println!("  HTTP:    http_get, http_post, http_put, http_delete");
    println!("  File:    read_file, write_file, append_file, file_exists, list_dir");
    println!("  Env:     env(\"NAME\"), env(\"NAME\", \"default\")");
    println!("  Util:    base64_encode, base64_decode, html_escape, url_encode");
    println!();
    println!("LANGUAGE:");
    println!("  agent \"name\" {{ ... }}        Define an AI agent");
    println!("  when started {{ ... }}        Run on startup");
    println!("  remember {{ key = val }}      Agent memory");
    println!("  brain {{ plan execute");
    println!("         remember communicate }}");
    println!("  result = ask openai {{ prompt: \"...\" }}");
    println!("  while cond {{ ... }}           While loop");
    println!("  break / continue              Loop control");
    println!("  assert cond \"message\"         Assertions");
    println!("  value ?? default              Null coalescing");
    println!("  use js.axios                  Import npm package");
    println!("  use py.requests               Import Python package");
    println!();
    println!("  Docs: docs/  |  Examples: docs/examples/");
}
