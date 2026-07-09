#![allow(dead_code)]

mod ai;
mod ast;
mod bridge;
mod capability;
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
            cmd_run(
                file,
                debug,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
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
            cmd_eval(
                src,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
            )
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
            cmd_run(
                file,
                debug,
                allow_shell,
                allow_process,
                allow_internal_http,
                no_sandbox,
                no_limit,
                deny,
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

    let program = parse_file(&source, path)?;
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

    interp
        .run_program(&program)
        .map_err(|e| format!("{}: {}", path, e))
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
) -> Result<(), String> {
    let program = parse_file(src, "<eval>")?;

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

    interp
        .run_program(&program)
        .map_err(|e| format!("<eval>: {}", e))
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
    println!("  gx -e '<source>'                       Run inline GX source (no temp file)");
    println!("  gx check <file.gx>                     Check syntax without running");
    println!("  gx init <name>                         Create a new GX project");
    println!("  gx build <file.gx> [-o name] [--allow-shell|--allow-process|--allow-internal-http|--deny <r>]");
    println!("                                          Build standalone launcher (capability flags baked in)");
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
    println!("  JSON:    json_parse, json_stringify");
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
}
