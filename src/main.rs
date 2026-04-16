mod lexer;
mod ast;
mod parser;
mod value;
mod interpreter;

use std::fs;
use std::path::Path;
use std::process;

use lexer::Lexer;
use parser::Parser;
use interpreter::Interpreter;

const VERSION: &str = "0.1.0";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage(&args[0]);
        process::exit(1);
    }

    match args[1].as_str() {
        "--version" | "-v" | "version" => {
            println!("gx {}", VERSION);
        }
        "--help" | "-h" | "help" => {
            print_help(&args[0]);
        }
        "run" => {
            let file = args.get(2).unwrap_or_else(|| {
                eprintln!("Error: gx run requires a file path");
                eprintln!("Usage: gx run <file.gx>");
                process::exit(1);
            });
            let debug = args.contains(&"--debug".to_string());
            run_file(file, debug);
        }
        "check" => {
            let file = args.get(2).unwrap_or_else(|| {
                eprintln!("Error: gx check requires a file path");
                process::exit(1);
            });
            check_file(file);
        }
        file if file.ends_with(".gx") => {
            let debug = args.contains(&"--debug".to_string());
            run_file(file, debug);
        }
        cmd => {
            eprintln!("gx: unknown command '{}'\n", cmd);
            print_usage(&args[0]);
            process::exit(1);
        }
    }
}

fn run_file(path: &str, debug: bool) {
    let source = read_source(path);

    if debug {
        eprintln!("[gx] Reading: {}", path);
    }

    let mut lexer = Lexer::new(&source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("GX Syntax Error in {}\n  {}", path, e);
            process::exit(1);
        }
    };

    if debug {
        eprintln!("[gx] Tokens: {}", tokens.len());
    }

    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("GX Parse Error in {}\n  {}", path, e);
            process::exit(1);
        }
    };

    if debug {
        eprintln!("[gx] Helpers: {}", program.helpers.len());
        for h in &program.helpers {
            eprintln!("[gx]   - {}", h.name);
        }
    }

    let mut interp = Interpreter::new();
    if let Err(e) = interp.run_program(&program) {
        eprintln!("GX Runtime Error in {}\n  {}", path, e);
        process::exit(1);
    }
}

fn check_file(path: &str) {
    let source = read_source(path);

    let mut lexer = Lexer::new(&source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {}", path, e);
            process::exit(1);
        }
    };

    match Parser::new(tokens).parse() {
        Ok(program) => {
            println!("{}: OK ({} helper{})",
                path,
                program.helpers.len(),
                if program.helpers.len() == 1 { "" } else { "s" }
            );
        }
        Err(e) => {
            eprintln!("{}: {}", path, e);
            process::exit(1);
        }
    }
}

fn read_source(path: &str) -> String {
    if !Path::new(path).exists() {
        eprintln!("gx: file not found: {}", path);
        process::exit(1);
    }
    match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gx: cannot read {}: {}", path, e);
            process::exit(1);
        }
    }
}

fn print_usage(prog: &str) {
    eprintln!("Usage: {} <command> [options]", prog);
    eprintln!("       {} <file.gx>", prog);
    eprintln!();
    eprintln!("Run '{} help' for more information.", prog);
}

fn print_help(prog: &str) {
    println!("GX Language v{}", VERSION);
    println!("Brain-first programming language for building transparent AI assistants");
    println!();
    println!("USAGE:");
    println!("    {} run <file.gx> [--debug]   Run a GX program", prog);
    println!("    {} check <file.gx>            Check syntax without running", prog);
    println!("    {} <file.gx>                  Shorthand for 'run'", prog);
    println!("    {} version                    Show version", prog);
    println!("    {} help                       Show this help", prog);
    println!();
    println!("EXAMPLES:");
    println!("    {} run docs/examples/hello_world.gx", prog);
    println!("    {} check main.gx", prog);
    println!("    {} run main.gx --debug", prog);
    println!();
    println!("LEARN MORE:");
    println!("    MASTER_PLAN.md   Full language roadmap");
    println!("    CLAUDE.md        Developer context");
    println!("    docs/examples/   Example GX programs");
}
