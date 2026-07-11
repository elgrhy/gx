#![allow(dead_code)]

mod ai;
mod ast;
mod bridge;
mod capability;
mod diagnostics;
mod diagnostics_render;
mod indent_parser;
mod interpreter;
mod lexer;
#[cfg(not(target_arch = "wasm32"))]
mod lsp;
#[cfg(not(target_arch = "wasm32"))]
mod package;
mod parser;
mod toolchain;
mod value;

use std::fs;
use std::path::Path;
use std::process;

use indent_parser::is_indent_syntax;
use interpreter::{Env, Interpreter};
use lexer::Lexer;
use parser::Parser;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Converts a script-supplied "seconds" value into a `Duration`, safe
/// against every input `Duration::from_secs_f64` itself panics on: NaN,
/// negative values, and any magnitude exceeding `Duration::MAX` — including
/// `Infinity`, reachable from an ordinary GX numeric literal with enough
/// digits to overflow `f64` during parsing. Duplicated from `lib.rs`
/// (which has the test coverage for it) rather than shared, deliberately:
/// this binary target declares its own separate `mod` tree instead of
/// depending on the `gxlang` library crate, so `crate::` inside
/// `interpreter`/`ai`/`toolchain` resolves to *this* crate root when
/// compiled as part of the `gx` binary — the same reason
/// `capability::normalize_path_no_symlink` predates this comment as its
/// own small, deliberately-duplicated helper.
fn clamp_duration_secs(secs: f64, max: std::time::Duration) -> std::time::Duration {
    if secs.is_nan() {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f64(secs.clamp(0.0, max.as_secs_f64()))
}

/// The tree-walking interpreter's own call chain (`eval_call` →
/// `call_user_function` → `run_stmts` → `run_stmt` → `eval_expr` → back to
/// `eval_call` for a recursive GX call) costs several real Rust stack
/// frames per one GX function call. Empirically, the platform default main-
/// thread stack overflowed after well under 100 levels of GX recursion in a
/// debug build (and not dramatically more in release) — a real, silent,
/// unrecoverable process abort (`fatal runtime error: stack overflow`, not
/// a catchable panic) that any accidentally-unbounded recursive GX function
/// would hit. Running the interpreter on a dedicated thread with a much
/// larger stack raises that ceiling to a level real recursive GX programs
/// are very unlikely to reach in practice; `Interpreter`'s own explicit
/// recursion-depth guard (`MAX_CALL_DEPTH` in `interpreter/mod.rs`) is the
/// actual safety net that turns "still too deep" into a graceful, catchable
/// GX error well before this larger — but still finite — stack is
/// exhausted, regardless of platform or build profile.
const INTERPRETER_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() {
    let child = std::thread::Builder::new()
        .stack_size(INTERPRETER_STACK_SIZE)
        .spawn(run)
        .expect("failed to spawn the main interpreter thread");
    match child.join() {
        Ok(()) => {}
        // A panic already printed its own message; match the exit code a
        // top-level panic would have produced if it had happened directly
        // on the main thread.
        Err(_) => process::exit(101),
    }
}

fn run() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    // `gx <command> --help`/`-h` — previously only the bare `gx help`/`gx
    // --help`/`gx -h` (no command) worked; `gx run --help` fell through to
    // ordinary argument parsing and produced a confusing "file not found:
    // --help" instead of ever showing usage. Checked for every recognized
    // command before any of its own argument parsing runs, so this always
    // wins regardless of where the flag appears among the command's own
    // arguments.
    if args.len() > 2 && args[2..].iter().any(|a| a == "--help" || a == "-h") {
        if let Some(usage) = command_usage(&args[1]) {
            println!("Usage: {}", usage);
            process::exit(0);
        }
    }

    let result = match args[1].as_str() {
        // `gx debug` is `gx run` with the Debugger Runtime wired up — same
        // flags, same execution path (see `cmd_run`), just named for
        // discoverability so a developer looking for "how do I debug a GX
        // script" finds a command that says exactly that, instead of only
        // ever discovering `--break` as an incidental flag on `run`.
        "run" | "debug" => {
            let file = require_arg(&args, 2, "gx run <file.gx> [--break line1,line2,...]");
            let debug = args.contains(&"--debug".to_string());
            let allow_shell = args.contains(&"--allow-shell".to_string());
            let allow_process = args.contains(&"--allow-process".to_string());
            let allow_internal_http = args.contains(&"--allow-internal-http".to_string());
            let no_sandbox = args.contains(&"--no-sandbox".to_string());
            let no_limit = args.contains(&"--no-limit".to_string());
            let deny = match parse_deny_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            let diagnostics = match parse_diagnostics_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            let break_lines = match parse_break_flag(&args) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            cmd_run(
                file,
                debug,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
                diagnostics,
                break_lines,
            )
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
            let allow_shell = args.contains(&"--allow-shell".to_string());
            let allow_process = args.contains(&"--allow-process".to_string());
            let allow_internal_http = args.contains(&"--allow-internal-http".to_string());
            let deny = match parse_deny_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            toolchain::build(
                file,
                output,
                allow_shell,
                allow_process,
                allow_internal_http,
                deny,
            )
        }
        "install" => {
            let offline = args.contains(&"--offline".to_string())
                || std::env::var("GX_OFFLINE").is_ok_and(|v| v != "0" && !v.is_empty());
            let pkg = args
                .get(2)
                .map(|s| s.as_str())
                .filter(|s| !s.starts_with("--"));
            match pkg {
                Some(pkg) => toolchain::install(pkg),
                None => toolchain::install_all(offline),
            }
        }
        "publish" => toolchain::publish(),
        "fmt" => {
            let target = require_arg(&args, 2, "gx fmt <file.gx|dir> [--check]");
            let check = args.contains(&"--check".to_string());
            toolchain::fmt(target, check)
        }
        "doc" => {
            let target = require_arg(&args, 2, "gx doc <file.gx|dir> [--out <file.md>]");
            let out = args
                .iter()
                .position(|a| a == "--out" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            toolchain::doc(target, out)
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
        "-e" | "eval" => {
            let src = require_arg(&args, 2, "gx -e '<source>'");
            let allow_shell = args.contains(&"--allow-shell".to_string());
            let allow_process = args.contains(&"--allow-process".to_string());
            let allow_internal_http = args.contains(&"--allow-internal-http".to_string());
            let no_sandbox = args.contains(&"--no-sandbox".to_string());
            let no_limit = args.contains(&"--no-limit".to_string());
            let deny = match parse_deny_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            let diagnostics = match parse_diagnostics_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            cmd_eval(
                src,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
                diagnostics,
            )
        }
        "repl" => {
            let diagnostics = match parse_diagnostics_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            cmd_repl(diagnostics)
        }
        #[cfg(not(target_arch = "wasm32"))]
        "lsp" => {
            lsp::run();
            Ok(())
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
            let allow_shell = args.contains(&"--allow-shell".to_string());
            let allow_process = args.contains(&"--allow-process".to_string());
            let allow_internal_http = args.contains(&"--allow-internal-http".to_string());
            let no_sandbox = args.contains(&"--no-sandbox".to_string());
            let no_limit = args.contains(&"--no-limit".to_string());
            let deny = match parse_deny_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            let diagnostics = match parse_diagnostics_flags(&args) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            let break_lines = match parse_break_flag(&args) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };
            cmd_run(
                file,
                debug,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
                diagnostics,
                break_lines,
            )
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

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    path: &str,
    debug: bool,
    allow_shell: bool,
    allow_process: bool,
    allow_internal_http: bool,
    no_sandbox: bool,
    no_limit: bool,
    deny: Vec<capability::Resource>,
    diagnostics: diagnostics::Diagnostics,
    break_lines: std::collections::HashSet<usize>,
) -> Result<(), String> {
    // Support `gx run -` to read source from stdin (used by `gx build` launchers).
    let source = if path == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("cannot read from stdin: {}", e))?;
        s
    } else {
        read_file(path)?
    };
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

    let program = parse_file(&source, path)
        .map_err(|e| diagnostics_render::render_diagnostic(&e, path, &source))?;
    if debug {
        eprintln!("[gx] helpers: {}", program.helpers.len());
        for h in &program.helpers {
            eprintln!("[gx]   - {}", h.name);
        }
    }

    let mut interp = Interpreter::new();
    interp.base_path = Some(path.to_string());
    interp.capabilities.shell = allow_shell;
    interp.capabilities.process = allow_process;
    interp.capabilities.internal_network = allow_internal_http;
    interp.no_loop_limit = no_limit;

    // The directory a manifest/sandbox would be rooted at: the script's own
    // directory, or cwd when reading from stdin (`gx run -`).
    let script_path = if path == "-" {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path))
    };
    let script_dir = if path == "-" {
        script_path.clone()
    } else {
        script_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf()
    };
    let script_dir = std::fs::canonicalize(&script_dir).unwrap_or(script_dir);

    // Sandbox: restrict file I/O to the directory containing the script.
    if !no_sandbox {
        interp.capabilities.filesystem =
            capability::FilesystemAccess::Sandboxed(script_dir.clone());
    }

    // Load gx.json's dependency/capability declarations independent of
    // sandboxing — file-access confinement and the allowlists governing
    // bridges/AI/process/network are different concerns, and disabling one
    // (`--no-sandbox`) must not silently disable the other.
    load_capability_manifest(&mut interp.capabilities, &script_dir);

    for resource in deny {
        interp.capabilities.deny(resource);
    }

    interp.diagnostics = diagnostics;
    interp.diagnostics.ensure_trace_id();

    if !break_lines.is_empty() {
        interp.debug.mode = interpreter::DebugMode::Running;
        interp.debug.break_lines = break_lines;
    }

    interp.run_program(&program).map_err(|e| {
        diagnostics_render::render_diagnostic(&format!("{}: {}", path, e), path, &source)
    })
}

#[allow(clippy::too_many_arguments)]
fn cmd_eval(
    src: &str,
    allow_shell: bool,
    allow_process: bool,
    allow_internal_http: bool,
    no_sandbox: bool,
    no_limit: bool,
    deny: Vec<capability::Resource>,
    diagnostics: diagnostics::Diagnostics,
) -> Result<(), String> {
    let program = parse_file(src, "<eval>")
        .map_err(|e| diagnostics_render::render_diagnostic(&e, "<eval>", src))?;

    let mut interp = Interpreter::new();
    interp.capabilities.shell = allow_shell;
    interp.capabilities.process = allow_process;
    interp.capabilities.internal_network = allow_internal_http;
    interp.no_loop_limit = no_limit;

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);

    // Sandbox relative file I/O to the current working directory for inline scripts.
    if !no_sandbox {
        interp.capabilities.filesystem = capability::FilesystemAccess::Sandboxed(cwd.clone());
    }

    // Previously `gx -e`/`gx eval` never loaded gx.json at all, unlike
    // `gx run` — so the same script behaved differently (allowlists
    // silently unenforced) depending only on which command ran it. Loading
    // it here, from cwd, makes both commands consistent.
    load_capability_manifest(&mut interp.capabilities, &cwd);

    for resource in deny {
        interp.capabilities.deny(resource);
    }

    interp.diagnostics = diagnostics;
    interp.diagnostics.ensure_trace_id();

    interp.run_program(&program).map_err(|e| {
        diagnostics_render::render_diagnostic(&format!("<eval>: {}", e), "<eval>", src)
    })
}

fn cmd_check(path: &str) -> Result<(), String> {
    let source = read_file(path)?;
    let program = parse_file(&source, path)
        .map_err(|e| diagnostics_render::render_diagnostic(&e, path, &source))?;

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

/// `~/.gx_history`, following the `GX_STATE_DIR`/`GX_PACKAGE_CACHE_DIR`
/// convention already used elsewhere for this kind of per-user file.
fn repl_history_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(std::path::PathBuf::from(home).join(".gx_history"))
}

/// Whether `buffer` is *not yet finished* — an unclosed `{`/`(`/`[` — and
/// the REPL should switch to a `... ` continuation prompt instead of
/// attempting to parse it. Runs the real lexer (not a hand-rolled counter)
/// so string literals and comments are handled exactly as the language
/// defines them; a `{` inside a string token is never mistaken for a real
/// brace, because it's never emitted as an `LBrace` token in the first
/// place.
///
/// This is the primary signal, not error-message sniffing: empirically,
/// the parser does not reliably fail with an "unterminated"/"Eof"-shaped
/// error for every kind of incomplete input (e.g. `function double(n) {`
/// with no closing brace and no body parses as a *valid, empty* function
/// today) — so waiting for a matching parse error to decide "keep
/// buffering" is unreliable. Counting bracket tokens directly sidesteps
/// that entirely: as long as depth is positive, the input is unambiguously
/// unclosed, regardless of what the parser would or wouldn't accept.
///
/// If the lexer itself fails (most commonly an unterminated string that
/// needs another line to close), fall back to checking the error text —
/// the lexer's own message for that case contains "unterminated".
fn input_is_incomplete(buffer: &str) -> bool {
    match Lexer::new(buffer).tokenize() {
        Ok(tokens) => {
            let mut depth = 0i32;
            for t in &tokens {
                match t.kind {
                    lexer::TokenKind::LBrace
                    | lexer::TokenKind::LParen
                    | lexer::TokenKind::LBracket => depth += 1,
                    lexer::TokenKind::RBrace
                    | lexer::TokenKind::RParen
                    | lexer::TokenKind::RBracket => depth -= 1,
                    _ => {}
                }
            }
            depth > 0
        }
        Err(e) => e.contains("unterminated") || e.contains("Unterminated"),
    }
}

/// Whether `program` needs the full `run_program` treatment (definitions —
/// `function`/`agent`/`helper`/`tool` — and/or `import`/`use`) rather than
/// being a plain sequence of statements a persistent REPL `Env` can run
/// directly. Definitions must keep going through `run_program` unchanged:
/// that's what registers them into `self.functions`/`self.helpers`/
/// `self.tools` (already persistent Interpreter fields, so this was never
/// the broken part) and, for a `helper`/`agent`, auto-runs it the same way
/// `gx run` would.
fn program_needs_full_run(program: &crate::ast::Program) -> bool {
    !program.file_imports.is_empty()
        || !program.imports.is_empty()
        || !program.functions.is_empty()
        || !program.tools.is_empty()
        || !program.helpers.is_empty()
        || program.top_level_brain.is_some()
}

fn cmd_repl(diagnostics: diagnostics::Diagnostics) -> Result<(), String> {
    use std::io::{self, BufRead, Write};

    println!("GX {} — Interactive REPL", VERSION);
    println!("Type GX code and press Enter. Type 'exit' or Ctrl+C to quit.");
    println!("Type :help for REPL commands, :help <name> to look up a builtin.");
    println!();

    let stdin = io::stdin();
    let mut interp = Interpreter::new();
    // Imports typed at the REPL (`import "./lib.gx"`) resolve relative to
    // the directory `gx repl` was launched from — the same convention
    // `gx run`/`gx -e` already use (the importing file/script's own
    // directory), with "the current directory" as the closest equivalent
    // for a script that was never a file to begin with.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    interp.base_path = Some(cwd.join("<repl>").to_string_lossy().into_owned());
    interp.diagnostics = diagnostics;
    interp.diagnostics.ensure_trace_id();

    // The persistent scope every bare statement runs against — see
    // `Interpreter::run_repl_stmts`'s doc comment for why this exists:
    // without it, a variable assigned on one line was invisible on the
    // next.
    let mut repl_env = Env::new();

    let history_path = repl_history_path();
    let mut history: Vec<String> = history_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();

    let mut buffer = String::new();
    loop {
        print!("{}", if buffer.is_empty() { "gx> " } else { "... " });
        io::stdout().flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Err(_) => break,
            Ok(_) => {}
        }
        let line = line.trim_end_matches(['\n', '\r']);

        if buffer.is_empty() {
            let trimmed = line.trim();
            if trimmed == "exit" || trimmed == "quit" {
                break;
            }
            if trimmed.is_empty() {
                continue;
            }
            if let Some(cmd) = trimmed.strip_prefix(':') {
                run_repl_command(cmd, &mut interp, &repl_env, &history);
                continue;
            }
        }

        buffer.push_str(line);
        buffer.push('\n');

        if input_is_incomplete(&buffer) {
            // Keep buffering — the continuation prompt (`... `) will show
            // on the next iteration.
            continue;
        }

        match parse_file(&buffer, "<repl>") {
            Ok(program) => {
                if let Some(p) = &history_path {
                    // Best-effort: a REPL that can't persist history is
                    // still a usable REPL, so a write failure here (a
                    // read-only home directory, disk full) is silently
                    // ignored rather than interrupting the session.
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                    {
                        let _ = writeln!(f, "{}", buffer.trim_end());
                    }
                }
                history.push(buffer.trim_end().to_string());

                if program_needs_full_run(&program) {
                    if let Err(e) = interp.run_program(&program) {
                        eprintln!("Error: {}", e);
                    }
                } else {
                    // Auto-print a bare expression's value (`5 + 5` shows
                    // `10`) the way most interactive interpreters do —
                    // but only when the line's last statement genuinely
                    // was one; an assignment or a `say` (which already
                    // prints on its own) has nothing worth echoing.
                    let auto_print = matches!(
                        program.top_level_stmts.last(),
                        Some(crate::ast::Stmt::Expr { .. })
                    );
                    match interp.run_repl_stmts(&program.top_level_stmts, &mut repl_env) {
                        Ok(v) if auto_print && !matches!(v, value::Value::Null) => {
                            println!("{}", v);
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                buffer.clear();
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                buffer.clear();
            }
        }
    }

    println!("Goodbye!");
    Ok(())
}

/// `:`-prefixed REPL meta-commands — never sent to the GX parser, so
/// there's no ambiguity with GX syntax to worry about (`:` never starts a
/// GX statement).
fn run_repl_command(cmd: &str, interp: &mut Interpreter, env: &Env, history: &[String]) {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim();
    let rest = parts.next().unwrap_or("").trim();

    match name {
        "help" if rest.is_empty() => {
            println!("REPL commands:");
            println!("  :help              show this list");
            println!("  :help <name>       look up documentation for a builtin");
            println!("  :vars              list variables currently in scope");
            println!("  :history           show input history for this session");
            println!("  :trace on|off      toggle diagnostics tracing (same as gx run --trace)");
            println!("  exit / quit        leave the REPL");
        }
        "help" => match lsp::builtin_docs::lookup(rest) {
            Some(doc) => println!("{}", doc),
            None => println!("No documentation found for '{}'.", rest),
        },
        "vars" => {
            let mut names: Vec<&String> =
                env.all_vars().keys().filter(|k| *k != "memory").collect();
            names.sort();
            if names.is_empty() {
                println!("(no variables yet)");
            } else {
                for n in names {
                    println!("  {} = {}", n, env.get(n));
                }
            }
        }
        "history" => {
            if history.is_empty() {
                println!("(empty)");
            } else {
                for (i, h) in history.iter().enumerate() {
                    println!("{:4}  {}", i + 1, h.replace('\n', "\n      "));
                }
            }
        }
        "trace" => match rest {
            "on" => {
                interp.diagnostics.set_enabled(true);
                interp.diagnostics.ensure_trace_id();
                println!("tracing on");
            }
            "off" => {
                interp.diagnostics.set_enabled(false);
                println!("tracing off");
            }
            _ => println!("usage: :trace on|off"),
        },
        _ => println!("Unknown command ':{}'. Type :help for a list.", cmd),
    }
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

/// The usage string for `gx <command> --help`, one entry per recognized
/// top-level command — kept in sync with each command's own `require_arg`
/// usage string (where it has one) so there's exactly one wording per
/// command, not two that can drift apart.
fn command_usage(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "run" => "gx run <file.gx> [--debug] [--break line1,line2,...] [--allow-shell] [--allow-process] [--allow-internal-http] [--no-sandbox] [--no-limit] [--deny <resource>] [--trace] [--log-level <level>]",
        "debug" => "gx debug <file.gx> [--break line1,line2,...] [--trace] [--log-level <level>] — alias for `gx run` with the Debugger Runtime available (see also the breakpoint() builtin)",
        "check" => "gx check <file.gx>",
        "init" | "new" => "gx init <project-name>",
        "build" => "gx build <file.gx> [-o <name>] [--allow-shell] [--allow-process] [--allow-internal-http] [--deny <resource>]",
        "install" => "gx install [<js.pkg|py.pkg>] [--offline]",
        "publish" => "gx publish",
        "fmt" => "gx fmt <file.gx|dir> [--check]",
        "doc" => "gx doc <file.gx|dir> [--out <file.md>]",
        "make" => "gx make <spec.gx|\"description\"> [--out <dir>]",
        "test" => "gx test [dir]",
        "-e" | "eval" => "gx -e '<source>' [--allow-shell] [--allow-process] [--allow-internal-http] [--no-sandbox] [--no-limit]",
        "repl" => "gx repl [--trace] [--log-level <level>]",
        "lsp" => "gx lsp",
        _ => return None,
    })
}

/// Parse every `--deny <resource>` occurrence (repeatable) into operator-level
/// capability denials — see `capability::Capabilities::deny`. These always
/// win over any grant from a CLI --allow-* flag or gx.json, by design: the
/// operator invoking `gx` has the final say over the script/manifest it runs.
fn parse_deny_flags(args: &[String]) -> Result<Vec<capability::Resource>, String> {
    let mut out = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if arg == "--deny" {
            let value = args.get(i + 1).ok_or("--deny requires a resource name")?;
            let resource = capability::Resource::parse(value).ok_or_else(|| {
                format!(
                    "--deny: unknown resource '{}'. Valid resources: shell, process, \
                     filesystem, internal_network, external_network, http_server, database, \
                     environment, ai, js, ts, py, binary, go, rust_bin.",
                    value
                )
            })?;
            out.push(resource);
        }
    }
    Ok(out)
}

/// Parse `--break line1,line2,...` — external, source-unmodified
/// breakpoints for the Debugger Runtime (see `interpreter::debugger`).
/// Absent entirely is the common case (no debugging requested) and yields
/// an empty set, which `cmd_run` treats as "don't touch `interp.debug` at
/// all" so a plain `gx run` keeps costing nothing extra.
fn parse_break_flag(args: &[String]) -> Result<std::collections::HashSet<usize>, String> {
    match args.iter().position(|a| a == "--break") {
        None => Ok(std::collections::HashSet::new()),
        Some(i) => {
            let value = args
                .get(i + 1)
                .ok_or("--break requires a comma-separated line list, e.g. --break 4,9")?;
            interpreter::debugger::parse_break_lines(value)
        }
    }
}

/// Parse `--trace` (enables spans — see `crate::diagnostics`) and
/// `--log-level <debug|info|warn|error>` (minimum level for structured
/// logging, independent of `--trace`; falls back to the `GX_LOG_LEVEL`
/// environment variable, then defaults to `info`). Shared by every entry
/// point (`gx run`, `gx eval`/`-e`, the `.gx` shorthand) so all three
/// behave identically.
fn parse_diagnostics_flags(args: &[String]) -> Result<crate::diagnostics::Diagnostics, String> {
    let mut diagnostics = crate::diagnostics::Diagnostics::new();
    if args.contains(&"--trace".to_string()) {
        diagnostics.set_enabled(true);
    }
    let level_str = args
        .iter()
        .position(|a| a == "--log-level")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("GX_LOG_LEVEL").ok());
    if let Some(s) = level_str {
        let level = parse_log_level(&s)?;
        diagnostics.set_min_level(level);
    }
    Ok(diagnostics)
}

fn parse_log_level(s: &str) -> Result<crate::diagnostics::Level, String> {
    crate::diagnostics::Level::parse(s).ok_or_else(|| {
        format!(
            "--log-level: unknown level '{}'. Valid levels: debug, info, warn, error.",
            s
        )
    })
}

/// Load `gx.json` from `dir` (if present) and apply it to `capabilities` —
/// shared by `cmd_run` and `cmd_eval` so both behave identically instead of
/// only one of them honoring the manifest. Deliberately independent of
/// `--no-sandbox`: file-sandboxing and the dependency/capability allowlists
/// are different concerns, and disabling one should not silently disable
/// the other.
fn load_capability_manifest(capabilities: &mut capability::Capabilities, dir: &Path) {
    let manifest_path = dir.join("gx.json");
    let Ok(content) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        capabilities.apply_manifest(&json);
    }
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
    println!("  gx run <file.gx> [--debug]                     Run a GX program");
    println!("  gx run <file.gx> --allow-shell                 Enable shell()/exec() builtins");
    println!("  gx run <file.gx> --allow-process               Enable process_run/process_spawn (recommended over shell())");
    println!(
        "  gx run <file.gx> --allow-internal-http         Allow HTTP to private/localhost IPs"
    );
    println!("  gx run <file.gx> --no-sandbox                  Disable file-path sandboxing");
    println!("  gx run <file.gx> --deny <resource>             Force-deny a capability, overriding gx.json (repeatable)");
    println!("  gx run <file.gx> --no-limit                    Remove while-loop iteration cap (for REPLs, infinite I/O loops)");
    println!("  gx debug <file.gx> [--break line1,line2,...]   Run with the interactive debugger (also: the breakpoint() builtin)");
    println!("  gx -e '<source>'                       Run inline GX source (no temp file)");
    println!("  gx check <file.gx>                     Check syntax without running");
    println!("  gx init <name>                         Create a new GX project");
    println!("  gx build <file.gx> [-o name] [--allow-shell|--allow-process|--allow-internal-http|--deny <r>]");
    println!("                                          Build standalone launcher (capability flags baked in)");
    println!("  gx install <js.pkg|py.pkg>             Install a package");
    println!("  gx install [--offline]                 Resolve gx.json's dependencies.gx and write gx.lock");
    println!("  gx publish                             Validate + hash this package, write a .gxpkg.json descriptor");
    println!("  gx fmt <file.gx|dir> [--check]         Format GX source code (--check: report only, don't write)");
    println!("  gx doc <file.gx|dir> [--out <file.md>] Generate a Markdown API reference");
    println!("  gx make <spec.gx|\"description\"> [--out dir]  Generate a complete project");
    println!("  gx test [dir]                          Run test files");
    println!("  gx repl                                Interactive REPL");
    println!("  gx lsp                                 Start the Language Server (stdio, for editor integration)");
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
    println!("CAPABILITY RUNTIME (gx.json):");
    println!("  \"dependencies\": {{ \"js\": [...], \"ts\": [...], \"py\": [...],");
    println!("                    \"binary\": [...], \"go\": [...], \"rust_bin\": [...],");
    println!("                    \"process\": [...], \"ai\": [...] }}   Restrict each to the listed names");
    println!("  \"capabilities\": {{ \"http_server\": false, \"database\": false,");
    println!("                     \"external_network\": false,");
    println!("                     \"env_deny\": [\"SECRET_NAME\"] }}   Restrict what was open by default");
    println!("  Declaring a list/false restricts; an undeclared key stays at its default.");
    println!("  gx.json can never grant shell/process/internal-network — only --allow-* can.");
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
    println!("  JSON:    json_parse, json_stringify, jsonl_parse, jsonl_stringify,");
    println!("           versioned_stringify(v, n), versioned_parse(s, n?)");
    println!("  Data:    data_import(path), data_export(path, v, schema?)  (.json/.yaml/.toml/.csv/.xml/.jsonl)");
    println!("  Template: render_template(template, data)  ({{{{dotted.path}}}} substitution)");
    println!("  HTTP:    http_get, http_post, http_put, http_delete, http_request,");
    println!("           http_stream, http_upload (opts: {{ timeout: seconds }})");
    println!("  Server:  serve on port N {{ route METHOD \"/path/:param\" {{ ... }} }},");
    println!(
        "           respond json|html|text [status], respond stream {{ sse_send(event, data) }}"
    );
    println!("  File:    read_file, write_file, append_file, file_exists, list_dir");
    println!("  Env:     env(\"NAME\"), env(\"NAME\", \"default\")");
    println!("  Util:    base64_encode, base64_decode, html_escape, url_encode");
    println!("  Stdlib:  truncate, token_count, tokens_used, write (no trailing newline)");
    println!("           dirname, basename, path_join, glob, group_by, url_parse");
    println!("  Crypto:  sha256, uuid, hmac_sha256, hmac_sha512, secure_compare,");
    println!("           secure_random, ed25519_generate_keypair, ed25519_sign,");
    println!("           ed25519_verify, jwt_sign, jwt_verify");
    println!("  Process: process_run, process_spawn, process_wait, process_kill,");
    println!("           process_exists, process_status, process_read (--allow-process)");
    println!("  Task:    task_spawn, task_wait, task_cancel, task_status,");
    println!("           task_emit(v), task_progress(handle)  (progress from a running task)");
    println!("  DB:      db_query, db_exec, db_transaction(path) {{ ... }}, db_migrate,");
    println!("           db_integrity_check, db_vacuum, db_backup (pooled, WAL, savepoints)");
    println!("  Testing: test(name, fn), before_each(fn), after_each(fn),");
    println!("           set_random_seed(n), test_temp_dir(), assert_golden(actual, path)");
    println!("  Config:  config_load({{ defaults, file, env_prefix, overrides, schema }})");
    println!("  Debug:   breakpoint() — or: gx debug <file.gx> --break line1,line2");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate the plain `gx` binary from inside one of the binary's own
    /// `#[test]`s. `CARGO_BIN_EXE_<name>` is only injected for tests in a
    /// target that *depends on* the gx binary (integration tests, other
    /// crates) — not for the gx binary's own unit tests, which every test
    /// in this module is. `cargo test`'s own executable lives at
    /// `target/<profile>/deps/gx-<hash>`; the plain binary `cargo build`
    /// produces sits two directories up, at `target/<profile>/gx`.
    fn gx_binary_path() -> std::path::PathBuf {
        let test_exe = std::env::current_exe().expect("current_exe");
        let gx_bin = test_exe
            .parent() // .../target/<profile>/deps
            .and_then(|p| p.parent()) // .../target/<profile>
            .map(|p| p.join(if cfg!(windows) { "gx.exe" } else { "gx" }))
            .expect("could not locate the gx binary next to the test executable");
        assert!(
            gx_bin.exists(),
            "expected a gx binary at {:?} — run `cargo build --bin gx` first",
            gx_bin
        );
        gx_bin
    }

    #[test]
    fn command_usage_covers_every_dispatched_command() {
        // Every command actually matched in main()'s dispatch (excluding
        // version/help, which have no arguments to need usage for) must
        // have a `gx <command> --help` entry — otherwise the flag silently
        // falls through to that command's own argument parsing instead of
        // showing usage, the exact bug this fix closes.
        for cmd in [
            "run", "debug", "check", "init", "new", "build", "install", "publish", "fmt", "doc",
            "make", "test", "-e", "eval", "repl", "lsp",
        ] {
            assert!(
                command_usage(cmd).is_some(),
                "command '{}' has no --help usage entry",
                cmd
            );
        }
    }

    #[test]
    fn command_usage_returns_none_for_an_unrecognized_command() {
        assert!(command_usage("not-a-real-command").is_none());
    }

    #[test]
    fn parse_deny_flags_collects_every_occurrence() {
        let args: Vec<String> = ["gx", "run", "f.gx", "--deny", "shell", "--deny", "database"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let denied = parse_deny_flags(&args).unwrap();
        assert_eq!(
            denied,
            vec![capability::Resource::Shell, capability::Resource::Database]
        );
    }

    #[test]
    fn parse_deny_flags_rejects_an_unknown_resource_name() {
        let args: Vec<String> = ["gx", "run", "f.gx", "--deny", "not-a-real-thing"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_deny_flags(&args).is_err());
    }

    #[test]
    fn parse_break_flag_returns_empty_when_absent() {
        let args: Vec<String> = ["gx", "run", "f.gx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_break_flag(&args).unwrap().is_empty());
    }

    #[test]
    fn parse_break_flag_collects_every_line_number() {
        let args: Vec<String> = ["gx", "run", "f.gx", "--break", "4,9,12"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let lines = parse_break_flag(&args).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines.contains(&4) && lines.contains(&9) && lines.contains(&12));
    }

    #[test]
    fn parse_break_flag_rejects_a_dangling_flag_with_no_value() {
        let args: Vec<String> = ["gx", "run", "f.gx", "--break"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_break_flag(&args).is_err());
    }

    #[test]
    fn parse_deny_flags_rejects_a_dangling_flag_with_no_value() {
        let args: Vec<String> = ["gx", "run", "f.gx", "--deny"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_deny_flags(&args).is_err());
    }

    #[test]
    fn parse_deny_flags_returns_empty_when_absent() {
        let args: Vec<String> = ["gx", "run", "f.gx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_deny_flags(&args).unwrap(), Vec::new());
    }

    #[test]
    fn load_capability_manifest_applies_gxjson_from_the_given_directory() {
        let dir =
            std::env::temp_dir().join(format!("gx_main_test_manifest_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("gx.json"),
            r#"{ "capabilities": { "http_server": false } }"#,
        )
        .unwrap();

        let mut caps = capability::Capabilities::new();
        assert!(caps
            .authorize(capability::Resource::HttpServer, None)
            .is_ok());
        load_capability_manifest(&mut caps, &dir);
        assert!(caps
            .authorize(capability::Resource::HttpServer, None)
            .is_err());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_capability_manifest_is_a_no_op_when_no_gxjson_exists() {
        let dir =
            std::env::temp_dir().join(format!("gx_main_test_no_manifest_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let mut caps = capability::Capabilities::new();
        load_capability_manifest(&mut caps, &dir);
        // Defaults untouched — no panic, no spurious restriction.
        assert!(caps
            .authorize(capability::Resource::HttpServer, None)
            .is_ok());
        assert!(caps.authorize(capability::Resource::Shell, None).is_err());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn input_is_incomplete_true_for_an_unclosed_function_body() {
        // The exact case that fooled the old error-message heuristic: this
        // parses as a *valid, empty* function today, with no parse error
        // at all to sniff — bracket counting is the only reliable signal.
        assert!(input_is_incomplete("function double(n) {\n"));
    }

    #[test]
    fn input_is_incomplete_true_for_unclosed_if_and_while_blocks() {
        assert!(input_is_incomplete("if x > 3 {\n"));
        assert!(input_is_incomplete("while i < 3 {\n  say i\n"));
    }

    #[test]
    fn input_is_incomplete_true_for_unclosed_paren_or_bracket() {
        assert!(input_is_incomplete("foo(1, 2\n"));
        assert!(input_is_incomplete("[1, 2,\n"));
    }

    #[test]
    fn input_is_incomplete_false_once_every_bracket_closes() {
        assert!(!input_is_incomplete(
            "function double(n) {\n  return n * 2\n}\n"
        ));
        assert!(!input_is_incomplete("x = 42\n"));
        assert!(!input_is_incomplete("say \"hi\"\n"));
    }

    #[test]
    fn input_is_incomplete_false_for_a_stray_closing_brace() {
        // Negative depth is a real error (mismatched brace), not
        // "keep waiting" — it must fall through to the parser so the user
        // sees an error instead of the REPL buffering forever.
        assert!(!input_is_incomplete("say \"hi\"\n}\n"));
    }

    #[test]
    fn input_is_incomplete_true_for_an_unterminated_string() {
        // The lexer itself fails on this input; the fallback path (reading
        // the lexer's own error text) must still say "keep buffering"
        // rather than surfacing a raw lexer error after every keystroke.
        assert!(input_is_incomplete("x = \"never closed\n"));
    }

    #[test]
    fn input_is_incomplete_ignores_braces_that_only_look_like_braces() {
        // A `{` inside a string literal must not be counted as a real
        // brace token — this is exactly why bracket-counting runs against
        // real lexer tokens instead of raw characters.
        assert!(!input_is_incomplete("say \"literal { brace\"\n"));
    }

    #[test]
    fn program_needs_full_run_false_for_bare_statements() {
        let src = "x = 1\ny = x + 1\n";
        let program = parse_file(src, "<test>").unwrap();
        assert!(!program_needs_full_run(&program));
    }

    #[test]
    fn program_needs_full_run_true_for_a_function_definition() {
        let src = "function f() {\n  return 1\n}\n";
        let program = parse_file(src, "<test>").unwrap();
        assert!(program_needs_full_run(&program));
    }

    #[test]
    fn assert_golden_update_env_var_overwrites_a_mismatched_file() {
        // Spawns the real `gx` binary with GX_UPDATE_GOLDEN=1 scoped to
        // that one child process's environment via Command::env, rather
        // than mutating the test process's own (shared, global)
        // environment with std::env::set_var — cargo test runs this
        // file's tests concurrently, and a process-wide env mutation could
        // race a differently-configured assert_golden test running on
        // another thread at the same moment.
        let dir = std::env::temp_dir().join(format!(
            "gx_assert_golden_cli_update_test_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let golden_path = dir.join("golden.txt");
        fs::write(&golden_path, "stale value").unwrap();
        let script_path = dir.join("script.gx");
        fs::write(
            &script_path,
            format!(
                "assert_golden(\"new value\", \"{}\")\n",
                golden_path.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let status = std::process::Command::new(gx_binary_path())
            .arg("run")
            .arg(&script_path)
            .arg("--no-sandbox")
            .env("GX_UPDATE_GOLDEN", "1")
            .status()
            .expect("failed to spawn gx binary");

        assert!(
            status.success(),
            "gx run with GX_UPDATE_GOLDEN=1 should succeed and rewrite the golden file"
        );
        assert_eq!(fs::read_to_string(&golden_path).unwrap(), "new value");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_load_env_prefix_overrides_a_default_with_type_coercion() {
        // Spawns the real gx binary with APP_PORT scoped to that one
        // child process's environment, for the same reason
        // assert_golden's GX_UPDATE_GOLDEN test above does: cargo test
        // runs this file's tests concurrently, and env vars are
        // process-global, so std::env::set_var here could race another
        // test reading the same variable on a different thread.
        let dir = std::env::temp_dir().join(format!(
            "gx_config_load_env_prefix_test_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("script.gx");
        fs::write(
            &script_path,
            r#"
c = config_load({ defaults: { port: 3000 }, env_prefix: "APP_" })
assert typeof(c.port) == "number" "env override must be coerced to match the default's type"
assert c.port == 9999 "APP_PORT should override the default port"
"#,
        )
        .unwrap();

        let status = std::process::Command::new(gx_binary_path())
            .arg("run")
            .arg(&script_path)
            .env("APP_PORT", "9999")
            .status()
            .expect("failed to spawn gx binary");

        assert!(
            status.success(),
            "config_load should apply and type-coerce the APP_PORT override"
        );

        fs::remove_dir_all(&dir).ok();
    }
}
