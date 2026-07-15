//! Indentation-based parser for the GX progressive syntax.
//!
//! Supports three levels that all compile to the same AST:
//!
//! ## Level 1 — Pure intent
//! ```gx
//! Agent greeter
//! name = "World"
//! "Hello {name}"
//! ```
//!
//! ## Level 2 — Named behaviors
//! ```gx
//! Agent greeter
//! name = "World"
//!
//! Greet:
//!   "Hello {name}"
//!
//! On start:
//!   Greet
//! ```
//!
//! ## Level 3 — Explicit brain cycle
//! ```gx
//! Agent greeter
//! name = "World"
//!
//! Plan:
//!   action = "greet"
//!
//! Execute:
//!   If action == "greet"
//!     result = "Hello {name}"
//!
//! Remember:
//!   last = result
//!
//! Communicate:
//!   result
//! ```

use crate::ast::*;
use crate::lexer::Lexer;
use crate::parser::Parser;

// ── Detection ─────────────────────────────────────────────────────────────────

/// Returns true if the source should be parsed with the indentation-based parser.
///
/// Detection uses a **positive indicator on the file's first significant line**
/// rather than scanning the whole file, and rather than the absence of braces —
/// so that a valid brace-syntax file that happens to have no braces (e.g. only
/// variable assignments) is never mis-detected as progressive syntax.
///
/// Every real progressive-syntax construct (`Plan:`/`Execute:`/`Remember:`/
/// `Communicate:`, `On start:`, named behavior blocks) only ever appears
/// *under* an `Agent`/`Helper` header — see every example in this module's
/// doc comment and every fixture in `tests/`. So the file is progressive if
/// and only if its first non-blank, non-comment line is:
/// 1. An un-quoted `Agent Name` or `Helper Name` declaration (no `"`, no `{`
///    on the same line as the keyword), or
/// 2. A bare `Agent` / `Helper` keyword with no name (level-1 minimal syntax
///    where the agent block has no body header).
///
/// This used to scan **every** line of the file for those two patterns plus
/// two more (a bare `plan:`/`execute:`/`remember:`/`communicate:` line, or an
/// `on ...:` line) anywhere at all — which meant a brace-syntax file with a
/// line shaped like `On error:` in a comment, a multi-line string, or an
/// object-literal key *anywhere in the file* (not just the top) would
/// silently reroute the **entire file** to this parser, which then silently
/// drops any top-level construct it doesn't recognize (see
/// `parse_top_level`). Checking only the first real line closes that hole:
/// a brace-syntax file can contain any of those shapes deep inside a
/// function body without the file's parse mode changing underneath it.
pub fn is_indent_syntax(source: &str) -> bool {
    for raw in source.lines() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        let lower = t.to_lowercase();

        // Unquoted Agent/Helper declaration: `Agent name` / `Helper name`
        if lower.starts_with("agent ") || lower.starts_with("helper ") {
            let rest = t[t.find(' ').unwrap_or(0) + 1..].trim_start();
            return !rest.starts_with('"') && !rest.starts_with('{');
        }

        // Bare `Agent` / `Helper` keyword with no name (level-1 minimal syntax)
        if lower == "agent" || lower == "helper" {
            return !t.contains('{');
        }

        // Any other first real line means brace syntax.
        return false;
    }
    false
}

// ── Logical lines ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ILine {
    no: usize, // 1-based source line number
    indent: usize,
    text: String, // trimmed
}

fn collect_lines(source: &str) -> Vec<ILine> {
    source
        .lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let text = raw.trim_end();
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                return None;
            }
            let indent = text.len() - trimmed.len();
            Some(ILine {
                no: i + 1,
                indent,
                text: trimmed.to_string(),
            })
        })
        .collect()
}

// ── Block helpers ─────────────────────────────────────────────────────────────

/// Collect consecutive lines whose indentation is strictly greater than `base`.
fn sub_block(lines: &[ILine], start: usize, base: usize) -> &[ILine] {
    let mut end = start;
    while end < lines.len() && lines[end].indent > base {
        end += 1;
    }
    &lines[start..end]
}

/// True if `text` is a block header (ends with `:` and isn't an operator like `>=`)
fn is_block_header(text: &str) -> bool {
    if !text.ends_with(':') {
        return false;
    }
    // Reject lines that are full statements ending in a colon inside an expression
    // (e.g.  `plan = { action: "go" }` — brace syntax should not reach here)
    // A block header has no `=` before the colon and no braces.
    let body = &text[..text.len() - 1];
    !body.contains('{') && !body.contains('=') || {
        // "On start:" has no = but IS a block header
        let lower = text.to_lowercase();
        lower.starts_with("on ")
            || lower == "plan:"
            || lower == "execute:"
            || lower == "remember:"
            || lower == "communicate:"
    }
}

// ── Expression / statement helpers ───────────────────────────────────────────

fn parse_expr_str(s: &str, line_no: usize) -> Result<Expr, String> {
    let tokens = Lexer::new(s)
        .tokenize()
        .map_err(|e| format!("Line {}: {}", line_no, e))?;
    Parser::new(tokens)
        .parse_one_expr()
        .map_err(|e| format!("Line {}: {}", line_no, e))
}

/// If `text` is unambiguously a `<name>(<one arg>):` block header, parse it
/// with the real expression grammar and return that single argument.
/// Returns `None` (not an error) whenever `text` could plausibly be
/// something else instead — it doesn't start with `<name>(` at all, *or*
/// it has no trailing `:` — so callers fall through to ordinary statement
/// parsing rather than hijacking it. That fallback matters because neither
/// `db_transaction` nor `span` is a reserved word: the brace parser only
/// ever intercepts `db_transaction(...)`/`span(...)` when a `{` follows,
/// so e.g. a user's own `function span(x) { ... }` called as a bare
/// `span(5)` statement still resolves as a normal call there — this must
/// match that, not error out just because the text happens to start the
/// same way a block header would. A trailing `:` is a much stronger
/// signal (bare statements essentially never end a line with one), so
/// once that's present, a malformed header (wrong argument count, called
/// through something other than a plain name) is treated as a genuine
/// mistake worth a specific error rather than a silent fallback.
fn parse_call_header(text: &str, name: &str, line_no: usize) -> Result<Option<Expr>, String> {
    let trimmed = text.trim();
    if !trimmed
        .to_lowercase()
        .starts_with(&format!("{}(", name.to_lowercase()))
    {
        return Ok(None);
    }
    let Some(header) = trimmed.strip_suffix(':') else {
        return Ok(None);
    };
    let expr = parse_expr_str(header, line_no)?;
    match expr {
        Expr::Call { callee, mut args } if args.len() == 1 => match *callee {
            Expr::Ident(n) if n.eq_ignore_ascii_case(name) => Ok(Some(args.remove(0))),
            _ => Err(format!(
                "Line {}: expected `{}(...)`, got a call to something else",
                line_no, name
            )),
        },
        _ => Err(format!(
            "Line {}: `{}(...)` must be called with exactly one argument",
            line_no, name
        )),
    }
}

/// Parse a one-liner statement (no sub-blocks). Returns the parsed Stmt.
/// String literals become Say statements. Bare identifiers stay as Expr stmts
/// (the interpreter handles zero-arg function auto-call at runtime).
fn parse_inline_stmt(text: &str, line_no: usize, auto_output: bool) -> Result<Stmt, String> {
    let lower = text.to_lowercase();

    // Re-run
    if lower == "re-run" {
        return Ok(Stmt::ReRun { line: line_no });
    }
    // Escalate
    if lower == "escalate" || lower == "escalate to human" {
        return Ok(Stmt::EscalateToHuman { line: line_no });
    }
    // Return
    if lower == "return" {
        return Ok(Stmt::Return {
            value: None,
            line: line_no,
        });
    }
    if lower.starts_with("return ") {
        let val = parse_expr_str(text[7..].trim(), line_no)?;
        return Ok(Stmt::Return {
            value: Some(val),
            line: line_no,
        });
    }

    // Log()/say/output — already handled by existing parser
    // Delegate to existing single-stmt parser
    let tokens = Lexer::new(text)
        .tokenize()
        .map_err(|e| format!("Line {}: {}", line_no, e))?;
    let stmt = Parser::new(tokens)
        .parse_one_stmt()
        .map_err(|e| format!("Line {}: {}", line_no, e))?;

    // In auto_output mode (Communicate block) or for bare string literals,
    // upgrade Expr{Str/Interpolated/Ident} to Say
    if auto_output {
        if let Stmt::Expr { expr, line } = stmt {
            return Ok(Stmt::Say { value: expr, line });
        }
    } else {
        // Always upgrade bare string literals to Say
        if let Stmt::Expr {
            expr: Expr::Str(_) | Expr::Interpolated(_),
            line,
        } = &stmt
        {
            return Ok(Stmt::Say {
                value: match &stmt {
                    Stmt::Expr { expr, .. } => expr.clone(),
                    _ => unreachable!(),
                },
                line: *line,
            });
        }
    }

    Ok(stmt)
}

// ── Recursive statement parser ────────────────────────────────────────────────

/// Parse a slice of ILines (all at the same indentation level relative to their parent)
/// into a Vec<Stmt>. `auto_output` causes bare expressions to become Say.
fn parse_stmts(lines: &[ILine], auto_output: bool) -> Result<Vec<Stmt>, String> {
    let mut stmts = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (stmt, consumed) = parse_one_stmt(lines, i, auto_output)?;
        stmts.push(stmt);
        i += consumed;
    }
    Ok(stmts)
}

/// Parse one statement starting at `lines[start]`.
/// Returns (stmt, lines_consumed).
fn parse_one_stmt(
    lines: &[ILine],
    start: usize,
    auto_output: bool,
) -> Result<(Stmt, usize), String> {
    let line = &lines[start];
    let text = &line.text;
    let lower = text.to_lowercase();

    // ── If / Else ─────────────────────────────────────────────────────────────
    if lower.starts_with("if ") {
        let cond_src = text[3..].trim();
        let cond = parse_expr_str(cond_src, line.no)?;
        let body_block = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(body_block, auto_output)?;
        let consumed = 1 + body_block.len();

        // Check for Else / Else if
        let next = start + consumed;
        let mut branches = vec![(cond, body)];
        let mut else_body = None;
        let mut total_consumed = consumed;

        let mut cur = next;
        while cur < lines.len() && lines[cur].indent == line.indent {
            let nl = lines[cur].text.to_lowercase();
            if nl.starts_with("else if ") {
                let cond2 = parse_expr_str(lines[cur].text[8..].trim(), lines[cur].no)?;
                let b2 = sub_block(lines, cur + 1, lines[cur].indent);
                let body2 = parse_stmts(b2, auto_output)?;
                total_consumed += 1 + b2.len();
                cur += 1 + b2.len();
                branches.push((cond2, body2));
            } else if nl == "else" {
                let b3 = sub_block(lines, cur + 1, lines[cur].indent);
                else_body = Some(parse_stmts(b3, auto_output)?);
                total_consumed += 1 + b3.len();
                break;
            } else {
                break;
            }
        }

        return Ok((
            Stmt::If {
                branches,
                else_body,
                line: line.no,
            },
            total_consumed,
        ));
    }

    // ── For ───────────────────────────────────────────────────────────────────
    if lower.starts_with("for ") {
        let rest = text[4..].trim();
        // Strip optional "each"
        let rest = rest.strip_prefix("each ").map(str::trim).unwrap_or(rest);
        // Expect: VAR in EXPR
        let in_pos = rest
            .to_lowercase()
            .find(" in ")
            .ok_or_else(|| format!("Line {}: expected 'in' in for loop", line.no))?;
        let var = rest[..in_pos].trim().to_string();
        let iter_src = rest[in_pos + 4..].trim();
        let iter = parse_expr_str(iter_src, line.no)?;
        let body_block = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(body_block, auto_output)?;
        let consumed = 1 + body_block.len();
        return Ok((
            Stmt::ForEach {
                var,
                iter,
                body,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── While ─────────────────────────────────────────────────────────────────
    if lower.starts_with("while ") || lower == "while true" {
        let cond_src = text[6..].trim();
        let cond = parse_expr_str(cond_src, line.no)?;
        let body_block = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(body_block, auto_output)?;
        let consumed = 1 + body_block.len();
        return Ok((
            Stmt::While {
                condition: cond,
                body,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── Break / Continue / Return ─────────────────────────────────────────────
    if lower == "break" {
        return Ok((Stmt::Break { line: line.no }, 1));
    }
    if lower == "continue" {
        return Ok((Stmt::Continue { line: line.no }, 1));
    }
    if lower.starts_with("return ") || lower == "return" {
        let expr = if text.len() > 7 {
            parse_expr_str(text[7..].trim(), line.no)?
        } else {
            crate::ast::Expr::Null
        };
        return Ok((
            Stmt::Return {
                value: Some(expr),
                line: line.no,
            },
            1,
        ));
    }

    // ── Assert ────────────────────────────────────────────────────────────────
    if lower.starts_with("assert ") {
        let rest = text[7..].trim();
        // Try to split off a trailing string message: assert EXPR "message"
        let (cond_src, msg_src) = if let Some(q) = rest.rfind('"') {
            if let Some(q2) = rest[..q].rfind('"') {
                let msg = &rest[q2..=q];
                let cond = rest[..q2].trim();
                (cond, Some(msg))
            } else {
                (rest, None)
            }
        } else {
            (rest, None)
        };
        let cond = parse_expr_str(cond_src, line.no)?;
        let message = msg_src.map(|m| parse_expr_str(m, line.no)).transpose()?;
        return Ok((
            Stmt::Assert {
                condition: cond,
                message,
                line: line.no,
            },
            1,
        ));
    }

    // ── Try / Catch ───────────────────────────────────────────────────────────
    if lower == "try:" || lower == "try" {
        let try_block = sub_block(lines, start + 1, line.indent);
        let try_body = parse_stmts(try_block, auto_output)?;
        let consumed = 1 + try_block.len();
        let catch_idx = start + consumed;
        if catch_idx < lines.len() {
            let cl = lines[catch_idx].text.to_lowercase();
            if cl.starts_with("catch") {
                // Typed catch (`catch NetworkError e:`) vs plain `catch e:`
                // is distinguished by whether the first identifier after
                // `catch` starts with an uppercase letter — the same rule
                // brace syntax's `parse_try_catch` uses. This must run on
                // the *original*-case text: `cl` above is lowercased only
                // to match the `catch` keyword itself case-insensitively;
                // using it here as well would make every catch look
                // untyped and silently produce a bogus catch-variable name
                // (e.g. `"networkerror e"`) instead of ever matching the
                // declared error kind.
                let original = lines[catch_idx].text.trim();
                let after_catch = original["catch".len()..].trim();
                let after_catch = after_catch.strip_suffix(':').unwrap_or(after_catch).trim();
                let mut tokens = after_catch.split_whitespace();
                let first = tokens.next();
                let second = tokens.next();
                let (catch_kind, catch_var) = match (first, second) {
                    (Some(f), Some(v)) if f.chars().next().is_some_and(|c| c.is_uppercase()) => {
                        (Some(f.to_string()), v.to_string())
                    }
                    (Some(f), _) => (None, f.to_string()),
                    (None, _) => (None, "err".to_string()),
                };
                let catch_block = sub_block(lines, catch_idx + 1, lines[catch_idx].indent);
                let catch_body = parse_stmts(catch_block, auto_output)?;
                let total = consumed + 1 + catch_block.len();
                return Ok((
                    Stmt::TryCatch {
                        try_body,
                        catch_kind,
                        catch_var,
                        catch_body,
                        line: line.no,
                    },
                    total,
                ));
            }
        }
        return Ok((
            Stmt::TryCatch {
                try_body,
                catch_kind: None,
                catch_var: "err".to_string(),
                catch_body: Vec::new(),
                line: line.no,
            },
            consumed,
        ));
    }

    // ── db_transaction(path): / span("name"): ────────────────────────────────
    // Brace syntax only ever accepted these two as `keyword(expr) { body }` —
    // reachable there, but a first-class progressive-syntax script had no
    // way to use either at all. `parse_call_header` parses the header with
    // the real expression grammar (so `db_transaction(env("DB_PATH")):`
    // works, not just a string literal), then confirms it's a one-argument
    // call to the expected name before pulling out that argument.
    if let Some(path) = parse_call_header(text, "db_transaction", line.no)? {
        let body_block = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(body_block, auto_output)?;
        let consumed = 1 + body_block.len();
        return Ok((
            Stmt::DbTransaction {
                path,
                body,
                line: line.no,
            },
            consumed,
        ));
    }
    if let Some(name) = parse_call_header(text, "span", line.no)? {
        let body_block = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(body_block, auto_output)?;
        let consumed = 1 + body_block.len();
        return Ok((
            Stmt::Span {
                name,
                body,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── Parallel ─────────────────────────────────────────────────────────────
    // Each statement inside the block is its own concurrent branch — same
    // semantics as brace syntax's `parallel { stmt1 stmt2 ... }`, where the
    // body is parsed one full statement at a time (so a branch can itself
    // be a multi-line nested block, e.g. an `if:`) rather than one per raw
    // source line.
    if lower == "parallel:" || lower == "parallel" {
        let body_block = sub_block(lines, start + 1, line.indent);
        let stmts = parse_stmts(body_block, auto_output)?;
        let branches: Vec<Vec<Stmt>> = stmts.into_iter().map(|s| vec![s]).collect();
        let consumed = 1 + body_block.len();
        return Ok((
            Stmt::Parallel {
                branches,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── Serve on port N ──────────────────────────────────────────────────────────
    // serve on port 3000
    //   route GET "/"
    //     <body>
    if lower.starts_with("serve") {
        // Parse "serve [on] [port] <N>" — extract the port number token
        let port_src = lower
            .split_whitespace()
            .find(|w| !matches!(*w, "serve" | "on" | "port"))
            .unwrap_or("3000");
        let port_expr = parse_expr_str(
            if port_src.is_empty() {
                "3000"
            } else {
                port_src
            },
            line.no,
        )?;
        let children = sub_block(lines, start + 1, line.indent);
        let mut routes = Vec::new();
        let mut ci = 0;
        while ci < children.len() {
            let cl = &children[ci];
            let cl_lower = cl.text.to_lowercase();
            if cl_lower.starts_with("route ") {
                let rest = cl.text[6..].trim();
                let mut parts = rest.splitn(2, ' ');
                let method = parts.next().unwrap_or("GET").to_uppercase();
                let path_raw = parts
                    .next()
                    .unwrap_or("\"/\"")
                    .trim()
                    .trim_matches('"')
                    .to_string();
                // Body is either the next indented block or a single behavior name on same line
                let route_body: Vec<crate::ast::Stmt>;
                let sub = sub_block(children, ci + 1, cl.indent);
                if sub.is_empty() {
                    // inline: route GET "/" BehaviorName
                    // treat trailing token as a behavior call if it looks like an identifier
                    route_body = Vec::new();
                    ci += 1;
                } else {
                    route_body = parse_stmts(sub, false)?;
                    ci += 1 + sub.len();
                }
                routes.push(crate::ast::RouteDecl {
                    method,
                    path: format!("/{}", path_raw.trim_start_matches('/')),
                    body: route_body,
                    line: cl.no,
                });
            } else {
                ci += 1;
            }
        }
        let consumed = 1 + children.len();
        return Ok((
            Stmt::Serve {
                port: port_expr,
                routes,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── Respond stream (SSE) ─────────────────────────────────────────────────────
    // respond stream
    //   sse_send("event", { ... })
    if lower == "respond stream" {
        let children = sub_block(lines, start + 1, line.indent);
        let body = parse_stmts(children, false)?;
        let consumed = 1 + children.len();
        return Ok((
            Stmt::RespondStream {
                body,
                line: line.no,
            },
            consumed,
        ));
    }

    // ── Respond ────────────────────────────────────────────────────────────────
    // respond html "..."  |  respond json { ... }  |  respond "..."
    if lower.starts_with("respond ") {
        let rest = text[8..].trim();
        let (format, value_src) = if let Some(s) = rest.strip_prefix("html ") {
            ("html".to_string(), s.trim().to_string())
        } else if let Some(s) = rest.strip_prefix("json ") {
            ("json".to_string(), s.trim().to_string())
        } else if let Some(s) = rest.strip_prefix("text ") {
            ("text".to_string(), s.trim().to_string())
        } else {
            ("text".to_string(), rest.to_string())
        };
        // Optional status code prefix: respond html 200 "..."
        let (status, value_src) = if let Some(first) = value_src.split_whitespace().next() {
            if let Ok(n) = first.parse::<u16>() {
                (n, value_src[first.len()..].trim().to_string())
            } else {
                (200, value_src)
            }
        } else {
            (200, value_src)
        };
        let value = parse_expr_str(&value_src, line.no)?;
        return Ok((
            Stmt::Respond {
                format,
                value,
                status,
                line: line.no,
            },
            1,
        ));
    }

    // ── Everything else — inline ──────────────────────────────────────────────
    let stmt = parse_inline_stmt(text, line.no, auto_output)?;
    Ok((stmt, 1))
}

// ── Top-level program parser ─────────────────────────────────────────────────

pub fn parse(source: &str) -> Result<Program, String> {
    let lines = collect_lines(source);
    let mut idx = 0;

    let mut file_imports: Vec<FileImport> = Vec::new();
    let mut imports: Vec<ImportDecl> = Vec::new();
    let mut functions: Vec<FunctionDef> = Vec::new();
    let mut helpers: Vec<HelperDef> = Vec::new();

    while idx < lines.len() {
        let line = &lines[idx];
        if line.indent != 0 {
            // By the time control returns here, `parse_agent` has already
            // consumed every line belonging to the agent/helper block it
            // parsed, so a stray indented line at this point is always a
            // real structural mistake (e.g. content indented under nothing,
            // or a block whose header line wasn't recognized) — silently
            // skipping it used to hide the mistake entirely instead of
            // reporting it.
            return Err(format!(
                "Line {}: unexpected indentation on `{}` — expected a top-level `import`, `use`, `agent`, or `helper` declaration",
                line.no, line.text
            ));
        }
        let lower = line.text.to_lowercase();

        if lower.starts_with("import ") {
            let path = line.text[7..].trim().trim_matches('"').to_string();
            // Parse optional `import "path" as alias`
            let (import_path, import_alias) = if let Some(as_pos) = path.to_lowercase().find(" as ")
            {
                let p = path[..as_pos].trim().trim_matches('"').to_string();
                let a = path[as_pos + 4..].trim().to_string();
                (p, Some(a))
            } else {
                (path.trim_matches('"').to_string(), None)
            };
            file_imports.push(FileImport {
                path: import_path,
                alias: import_alias,
                line: line.no,
            });
            idx += 1;
        } else if lower.starts_with("use ") {
            let rest = line.text[4..].trim();
            if let Some(dot) = rest.find('.') {
                imports.push(ImportDecl {
                    namespace: rest[..dot].to_string(),
                    package: rest[dot + 1..].to_string(),
                    path: None,
                    line: line.no,
                });
            }
            idx += 1;
        } else if lower.starts_with("agent ") || lower.starts_with("helper ") {
            let (helper, new_fns, new_idx) = parse_agent(&lines, idx)?;
            helpers.push(helper);
            functions.extend(new_fns);
            idx = new_idx;
        } else {
            // Progressive syntax only recognizes `import`, `use`, `agent`,
            // and `helper` at the top level. This used to be `idx += 1`
            // (silently skip and move on), which meant a whole top-level
            // construct the parser didn't recognize — including an entire
            // brace-syntax file misrouted here by an over-eager
            // `is_indent_syntax` match — would vanish with no error and no
            // warning, producing a program that silently did nothing.
            return Err(format!(
                "Line {}: unrecognized top-level statement `{}` — progressive syntax only allows `import`, `use`, `agent`, or `helper` at the top level",
                line.no, line.text
            ));
        }
    }

    Ok(Program {
        file_imports,
        imports,
        functions,
        tools: Vec::new(),
        helpers,
        top_level_brain: None,
        top_level_stmts: Vec::new(),
    })
}

// ── Agent parser ──────────────────────────────────────────────────────────────

fn parse_agent(
    lines: &[ILine],
    start: usize,
) -> Result<(HelperDef, Vec<FunctionDef>, usize), String> {
    let header = &lines[start];
    let name_raw = header.text[header.text.find(' ').unwrap_or(0) + 1..]
        .trim()
        .trim_matches('"')
        .to_string();

    // Collect everything after the Agent header up to the next Agent/Helper at the same level
    let mut body_end = start + 1;
    while body_end < lines.len() {
        let l = &lines[body_end];
        if l.indent <= header.indent {
            let lower = l.text.to_lowercase();
            if lower.starts_with("agent ") || lower.starts_with("helper ") {
                break; // next agent definition
            }
        }
        body_end += 1;
    }
    let body_slice = &lines[start + 1..body_end];
    let end_idx = body_end;

    let mut memory: Vec<MemoryEntry> = Vec::new();
    let mut when_blocks: Vec<WhenBlock> = Vec::new();
    let mut extracted_fns: Vec<FunctionDef> = Vec::new();
    let mut plan_stmts: Vec<Stmt> = Vec::new();
    let mut execute_stmts: Vec<Stmt> = Vec::new();
    let mut remember_stmts: Vec<Stmt> = Vec::new();
    let mut communicate_stmts: Vec<Stmt> = Vec::new();
    let mut implicit_stmts: Vec<Stmt> = Vec::new();
    let mut has_brain_phases = false;
    let mut goal: Option<String> = None;
    let mut retry: Option<u32> = None;
    let mut on_error: Option<String> = None;

    let mut i = 0;
    while i < body_slice.len() {
        let bl = &body_slice[i];

        // Must be at the "immediate body" indentation (one level below Agent)
        // (sub_block already filters by > header.indent, so everything here qualifies)

        if is_block_header(&bl.text) {
            let block_name_raw = bl.text[..bl.text.len() - 1].trim(); // strip trailing ':'
            let block_lower = block_name_raw.to_lowercase();

            let sub = sub_block(body_slice, i + 1, bl.indent);
            let sub_len = sub.len();

            match block_lower.as_str() {
                "plan" => {
                    has_brain_phases = true;
                    plan_stmts = parse_stmts(sub, false)?;
                }
                "execute" => {
                    has_brain_phases = true;
                    execute_stmts = parse_stmts(sub, false)?;
                }
                "remember" => {
                    has_brain_phases = true;
                    remember_stmts = parse_stmts(sub, false)?;
                }
                "communicate" => {
                    has_brain_phases = true;
                    communicate_stmts = parse_stmts(sub, true)?;
                }
                _ if block_lower.starts_with("on ") => {
                    // Operates on `raw_event` (original case, sliced only
                    // at pure-ASCII keyword boundaries measured on
                    // `raw_lower`) rather than the lowercased text end to
                    // end — matching how typed `catch` above must preserve
                    // case for anything that becomes a real expression or
                    // string. Lowercasing everything first (the previous
                    // behavior) is why `on x changes:`/`on cron "...":`
                    // used to silently fall through to the generic
                    // `WhenTrigger::Expr` arm as a mangled fake
                    // identifier (`"x changes"`, `"cron \"...\""") that
                    // could never evaluate truthy — a trigger that
                    // silently never fires, not a parse error.
                    let raw_event = block_name_raw[3..].trim();
                    let raw_lower = raw_event.to_lowercase();
                    let trigger = if raw_lower == "start" || raw_lower == "started" {
                        WhenTrigger::Started
                    } else if raw_lower.starts_with("message ") {
                        let raw_msg = raw_event["message ".len()..].trim();
                        WhenTrigger::Message(raw_msg.trim_matches('"').to_string())
                    } else if raw_lower.starts_with("cron ") {
                        let raw_cron = raw_event["cron ".len()..].trim();
                        WhenTrigger::Cron(raw_cron.trim_matches('"').to_string())
                    } else if raw_lower.ends_with(" changes") {
                        let expr_src = raw_event[..raw_event.len() - " changes".len()].trim();
                        WhenTrigger::Changes(parse_expr_str(expr_src, bl.no)?)
                    } else {
                        // Any other expression (`on count > 5:`, not just
                        // a bare identifier) — brace syntax's
                        // `parse_when_block` accepts a full expression
                        // here via `self.parse_expr()`; the previous
                        // `Expr::Ident(other.to_string())` only ever
                        // supported the bare-identifier case.
                        WhenTrigger::Expr(parse_expr_str(raw_event, bl.no)?)
                    };
                    let body = parse_stmts(sub, false)?;
                    when_blocks.push(WhenBlock {
                        trigger,
                        body,
                        line: bl.no,
                    });
                }
                _ => {
                    // Named behavior block → extract as function
                    let fn_body = parse_stmts(sub, false)?;
                    extracted_fns.push(FunctionDef {
                        name: block_name_raw.to_string(),
                        params: Vec::new(),
                        body: fn_body,
                        line: bl.no,
                    });
                }
            }
            i += 1 + sub_len;
        } else if let Some(value) = strip_agent_field_prefix(&bl.text, "goal") {
            goal = Some(value.trim_matches('"').to_string());
            i += 1;
        } else if let Some(value) = strip_agent_field_prefix(&bl.text, "retry") {
            retry = value.parse::<u32>().ok();
            i += 1;
        } else if let Some(value) = strip_agent_field_prefix(&bl.text, "on_error") {
            on_error = Some(value.to_string());
            i += 1;
        } else if looks_like_memory_entry(&bl.text) {
            // `key = value` at agent level → memory entry
            if let Some((key, val_src)) = split_assign(&bl.text) {
                let val = parse_expr_str(val_src.trim(), bl.no)?;
                memory.push(MemoryEntry {
                    key,
                    value: val,
                    line: bl.no,
                });
            } else {
                implicit_stmts.push(parse_inline_stmt(&bl.text, bl.no, false)?);
            }
            i += 1;
        } else {
            // Bare statement at agent level
            let (stmt, consumed) = parse_one_stmt(body_slice, i, false)?;
            implicit_stmts.push(stmt);
            i += consumed;
        }
    }

    // Build brain or fall back to when-started
    let brain = if has_brain_phases {
        Some(BrainBlock {
            plan: plan_stmts,
            execute: execute_stmts,
            remember: remember_stmts,
            communicate: communicate_stmts,
            line: header.no,
        })
    } else {
        None
    };

    // If there are implicit statements and no explicit On start block, wrap them
    if !implicit_stmts.is_empty() {
        let has_start = when_blocks
            .iter()
            .any(|w| matches!(w.trigger, WhenTrigger::Started));
        if !has_start {
            when_blocks.insert(
                0,
                WhenBlock {
                    trigger: WhenTrigger::Started,
                    body: implicit_stmts,
                    line: header.no,
                },
            );
        }
    }

    Ok((
        HelperDef {
            name: name_raw,
            goal,
            can_do: Vec::new(),
            memory,
            receive_block: Vec::new(),
            brain,
            recipes: Vec::new(),
            objectives: Vec::new(),
            when_blocks,
            retry,
            timeout_ms: None,
            on_error,
            functions: Vec::new(),
            line: header.no,
        },
        extracted_fns,
        end_idx,
    ))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// True if this line looks like a bare memory assignment: `ident = expr`
/// (not a comparison like `x == y`)
fn looks_like_memory_entry(text: &str) -> bool {
    if let Some(eq_pos) = text.find('=') {
        // Make sure it's `=` not `==`, `!=`, `<=`, `>=`
        let prev = text[..eq_pos].chars().last();
        let next = text[eq_pos + 1..].chars().next();
        if matches!(prev, Some('!' | '<' | '>')) || next == Some('=') {
            return false;
        }
        // LHS must be a plain identifier (possibly with dots for memory.x)
        let lhs = text[..eq_pos].trim();
        lhs.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    } else {
        false
    }
}

/// Split `"key = value_expr"` into `("key", "value_expr")`
fn split_assign(text: &str) -> Option<(String, &str)> {
    let eq_pos = text.find('=')?;
    let prev = text[..eq_pos].chars().last();
    let next = text[eq_pos + 1..].chars().next();
    if matches!(prev, Some('!' | '<' | '>')) || next == Some('=') {
        return None;
    }
    let key = text[..eq_pos].trim().to_string();
    let val = &text[eq_pos + 1..];
    Some((key, val))
}

/// If `text` is `"<field>: <value>"` (case-insensitive on `field`), return
/// `value` with its original case preserved. Used for the agent-header
/// fields (`goal:`, `retry:`, `on_error:`) that brace syntax parses as
/// simple `Field: value` lines inside `helper "x" { ... }` — these don't
/// end in `:` themselves (unlike `Plan:`/`On start:`/a named-behavior
/// block header), so `is_block_header` correctly never claims them, and
/// they don't contain `=`, so `looks_like_memory_entry` doesn't either;
/// without this, they fell through to the generic statement parser and
/// were a hard parse error (`goal: "..."` isn't valid expression syntax)
/// — every one of these three fields is genuinely read by the interpreter
/// (see `HelperDef.goal`/`.retry`/`.on_error`), so this was a real,
/// loudly-failing progressive-syntax gap, not dead surface like
/// `can_do`/`timeout` (parsed by the brace grammar too, but never read by
/// the interpreter — deliberately not given a progressive-syntax path
/// either, since "already supported by the runtime" doesn't hold for
/// them).
fn strip_agent_field_prefix<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let colon = text.find(':')?;
    if text[..colon].trim().eq_ignore_ascii_case(field) {
        Some(text[colon + 1..].trim())
    } else {
        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn run_src(src: &str) -> Result<(), String> {
        let prog = parse(src)?;
        crate::interpreter::Interpreter::new().run_program(&prog)
    }

    #[test]
    fn typed_catch_only_catches_a_matching_error_kind() {
        // Regression test: progressive-syntax `catch <Kind> e:` used to be
        // lowercased before extraction, so `catch_kind` was always parsed
        // as `None` — every typed catch silently behaved like a catch-all,
        // never actually filtering by kind. `assert false "boom"` always
        // raises `AssertionError` (see the interpreter's hardcoded mapping
        // for `Signal::AssertFail`), so a `catch NetworkError e:` around it
        // must NOT match and must propagate the failure.
        let err = run_src(
            r#"
Agent typed_catch_mismatch
On start:
  try:
    assert false "boom"
  catch NetworkError e:
    say "wrongly caught"
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("boom"),
            "error should propagate uncaught: {}",
            err
        );
    }

    #[test]
    fn typed_catch_catches_a_matching_error_kind() {
        run_src(
            r#"
Agent typed_catch_match
On start:
  try:
    assert false "boom"
  catch AssertionError e:
    say "correctly caught: {e.message}"
"#,
        )
        .unwrap();
    }

    #[test]
    fn on_cron_parses_to_a_cron_trigger_not_a_dead_expr_identifier() {
        // Regression test: `On cron "...":` used to be lowercased whole
        // and fall through to the generic `WhenTrigger::Expr` arm as a
        // fake identifier literally named `cron "* * * * *"` — a trigger
        // that could never evaluate truthy, so the block silently never
        // ran. No parse error, just permanently dead code.
        let prog = parse("Agent cron_test\nOn cron \"*/5 * * * *\":\n  say \"tick\"\n").unwrap();
        let when = &prog.helpers[0].when_blocks[0];
        match &when.trigger {
            WhenTrigger::Cron(expr) => assert_eq!(expr, "*/5 * * * *"),
            other => panic!("expected WhenTrigger::Cron, got {:?}", other),
        }
    }

    #[test]
    fn on_changes_parses_to_a_changes_trigger_not_a_dead_expr_identifier() {
        // Same regression class as the cron test above: `On x changes:`
        // used to become a fake identifier named `"x changes"`.
        let prog = parse("Agent changes_test\nOn count changes:\n  say \"changed\"\n").unwrap();
        let when = &prog.helpers[0].when_blocks[0];
        match &when.trigger {
            WhenTrigger::Changes(Expr::Ident(name)) => assert_eq!(name, "count"),
            other => panic!(
                "expected WhenTrigger::Changes(Ident(\"count\")), got {:?}",
                other
            ),
        }
    }

    #[test]
    fn on_message_and_on_start_still_parse_correctly_after_the_case_preserving_fix() {
        // The cron/changes fix reworked this whole branch to stop
        // lowercasing everything up front — make sure the two triggers
        // that already worked weren't broken by that change.
        let prog = parse("Agent msg_test\nOn message \"ping\":\n  say \"pong\"\n").unwrap();
        match &prog.helpers[0].when_blocks[0].trigger {
            WhenTrigger::Message(event) => assert_eq!(event, "ping"),
            other => panic!("expected WhenTrigger::Message, got {:?}", other),
        }

        let prog2 = parse("Agent start_test\nOn start:\n  say \"go\"\n").unwrap();
        assert!(matches!(
            prog2.helpers[0].when_blocks[0].trigger,
            WhenTrigger::Started
        ));
    }

    #[test]
    fn on_a_general_boolean_expression_parses_as_a_real_expression_not_just_an_identifier() {
        // Brace syntax's `when <expr> { }` accepts any expression, not
        // just a bare identifier (`when memory.count > 5 { }` is valid) —
        // the previous progressive-syntax fallback only ever produced
        // `Expr::Ident(the_whole_line)`, which would have made
        // `count > 5` parse as a single (invalid) identifier named
        // `"count > 5"` instead of a real comparison expression.
        let prog = parse("Agent expr_test\nOn count > 5:\n  say \"big\"\n").unwrap();
        match &prog.helpers[0].when_blocks[0].trigger {
            WhenTrigger::Expr(Expr::BinOp { op, .. }) => {
                assert_eq!(*op, BinOp::Gt);
            }
            other => panic!("expected WhenTrigger::Expr(BinOp Gt), got {:?}", other),
        }
    }

    #[test]
    fn goal_field_is_parsed_and_stored_in_memory_at_runtime() {
        // `goal:` was previously unparseable in progressive syntax at all
        // (a hard parse error — `goal: "..."` matches neither a block
        // header nor a `key = value` memory entry) despite being genuinely
        // read by the interpreter (`helper.goal` → `memory.goal`, see
        // `call_agent` in mod.rs). Checked behaviorally, not just at the
        // AST level, since the whole point is that it reaches `memory`.
        let prog = crate::indent_parser::parse(
            "Agent goal_test\ngoal: \"be helpful\"\nOn start:\n  assert memory.goal == \"be helpful\" \"goal must reach memory\"\n",
        )
        .unwrap();
        assert_eq!(prog.helpers[0].goal.as_deref(), Some("be helpful"));
        let mut interp = crate::interpreter::Interpreter::new();
        interp.run_program(&prog).unwrap();
    }

    #[test]
    fn retry_and_on_error_fields_are_parsed_and_reach_the_helper_def() {
        // Same gap as `goal:` above — `retry: N` and `on_error: policy`
        // are genuinely consumed by `call_agent`'s brain-cycle retry loop
        // (mod.rs), so this was a real (loudly failing) parity gap, not
        // dead surface like `can_do`/`timeout` (parsed by brace syntax
        // too, but never read anywhere — deliberately left unsupported).
        let prog = crate::indent_parser::parse(
            "Agent retry_test\nretry: 2\non_error: continue\nPlan:\n  x = 1\nExecute:\n  y = 2\nRemember:\n  memory.y = y\nCommunicate:\n  y\n",
        )
        .unwrap();
        assert_eq!(prog.helpers[0].retry, Some(2));
        assert_eq!(prog.helpers[0].on_error.as_deref(), Some("continue"));
    }

    #[test]
    fn test_detect_indent_syntax() {
        assert!(is_indent_syntax("Agent greeter\nname = \"World\"\n"));
        assert!(!is_indent_syntax("agent \"greeter\" {\n}\n"));
    }

    #[test]
    // Regression test for a real production bug (AgentX feedback, 2026-07):
    // a brace-syntax file using `agent` as a plain variable name deep inside
    // a function body used to misroute the *entire file* to the indentation
    // parser, because `is_indent_syntax` scanned every line instead of just
    // the first. The file would then silently parse to an empty program and
    // exit 0 with no output. See `Cargo.toml`/CHANGELOG for the fix.
    fn is_indent_syntax_ignores_agent_as_a_variable_name_deep_in_a_brace_file() {
        let src = r#"function f() {
  flag = false
  agent = "*"
  flag = (agent == "*")
  say flag
}
f()
"#;
        assert!(!is_indent_syntax(src));
    }

    #[test]
    // Same class of bug as above, triggered via the `Plan:`/`Execute:`/
    // `Remember:`/`Communicate:`/`On ...:` indicators instead of `agent`:
    // any of these shapes appearing anywhere in a brace-syntax file (inside
    // a string, an object-literal key, etc.) used to misroute the whole
    // file. They must now only matter as the file's very first line.
    fn is_indent_syntax_ignores_brain_keywords_and_on_lines_deep_in_a_brace_file() {
        let msg = r#"function retry_logic() {
  message = "On error: retry the request"
  say message
}
retry_logic()
"#;
        assert!(!is_indent_syntax(msg));

        let obj = r#"function build_step() {
  step = {
    execute:
      true
  }
  say step
}
build_step()
"#;
        assert!(!is_indent_syntax(obj));
    }

    #[test]
    // `parse()`'s top-level loop used to silently skip (`idx += 1`) any
    // top-level line it didn't recognize as `import`/`use`/`agent`/`helper`,
    // producing an empty `Program` with no error at all. A bare `Agent`
    // token with no name (matched by `is_indent_syntax`'s "level-1 minimal
    // syntax" indicator, but never actually handled by `parse_agent`, which
    // always expects a name after `agent `) is a reliable way to reach that
    // branch: it must now be a clear error instead of a silently empty
    // program that exits 0.
    fn parse_errors_on_an_unrecognized_top_level_line_instead_of_silently_dropping_it() {
        let src = "Agent\nsay \"hi\"\n";
        let err = parse(src).unwrap_err();
        assert!(
            err.contains("unrecognized top-level statement"),
            "expected a clear top-level error, got: {}",
            err
        );
    }

    #[test]
    fn test_level1_simple() {
        run_src(
            r#"
Agent greeter
name = "World"
"Hello {name}"
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_level2_behaviors() {
        run_src(
            r#"
Agent greeter
name = "World"

Greet:
  "Hello {name}"

On start:
  Greet
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_level3_brain_cycle() {
        run_src(
            r#"
Agent counter
count = 0

Plan:
  action = "increment"

Execute:
  If action == "increment"
    count += 1

Remember:
  memory.count = count

Communicate:
  count
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_mixed_mode() {
        run_src(
            r#"
Agent smart
name = "Ahmed"

Think:
  If name == "Ahmed"
    result = "Welcome back"

Plan:
  Think

Communicate:
  result
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_on_start_event() {
        run_src(
            r#"
Agent bot
greeting = "hi"

On start:
  say "Agent started"
  say greeting
"#,
        )
        .unwrap();
    }

    // ── db_transaction / span / parallel in progressive syntax ──────────────
    // Regression tests: these three blocks previously existed only in the
    // brace parser — a first-class progressive-syntax script had no way to
    // use transactions, diagnostics spans, or concurrent branches at all.

    #[test]
    fn span_block_parses_and_runs_in_progressive_syntax() {
        run_src(
            r#"
Agent demo

On start:
  span("checkout"):
    memory.ran = true
  say "ran={memory.ran}"
"#,
        )
        .unwrap();
    }

    #[test]
    fn db_transaction_block_parses_in_progressive_syntax() {
        let dir = std::env::temp_dir().join(format!(
            "gx_indent_db_transaction_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir
            .join("t.db")
            .to_string_lossy()
            .into_owned()
            .replace('\\', "\\\\");
        run_src(&format!(
            r#"
Agent demo

On start:
  db_transaction("{db}"):
    db_exec("{db}", "CREATE TABLE t (id INTEGER)")
    db_exec("{db}", "INSERT INTO t (id) VALUES (1)")
  rows = db_query("{db}", "SELECT * FROM t")
  assert len(rows) == 1 "the transaction's writes must be visible after it commits"
"#,
        ))
        .unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn db_transaction_block_rolls_back_on_error_in_progressive_syntax() {
        let dir = std::env::temp_dir().join(format!(
            "gx_indent_db_transaction_rollback_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir
            .join("t.db")
            .to_string_lossy()
            .into_owned()
            .replace('\\', "\\\\");
        let err = run_src(&format!(
            r#"
Agent demo

On start:
  db_transaction("{db}"):
    db_exec("{db}", "CREATE TABLE t (id INTEGER)")
    db_exec("{db}", "INSERT INTO t (id) VALUES (1)")
    db_exec("{db}", "this is not valid sql")
"#,
        ))
        .unwrap_err();
        assert!(!err.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parallel_block_parses_and_runs_all_branches_in_progressive_syntax() {
        run_src(
            r#"
Agent demo

On start:
  parallel:
    memory.a = 1
    memory.b = 2
    memory.c = 3
  assert memory.a == 1
  assert memory.b == 2
  assert memory.c == 3
"#,
        )
        .unwrap();
    }

    #[test]
    fn parse_call_header_returns_none_for_a_non_matching_line() {
        assert!(parse_call_header("if x > 0:", "db_transaction", 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn parse_call_header_rejects_more_than_one_argument() {
        let err = parse_call_header("db_transaction(a, b):", "db_transaction", 1).unwrap_err();
        assert!(err.contains("exactly one argument"));
    }

    #[test]
    fn parse_call_header_falls_through_when_theres_no_trailing_colon() {
        // Regression test: neither `db_transaction` nor `span` is a
        // reserved word (the brace parser only intercepts them when a `{`
        // follows) — a bare `db_transaction("x")` statement with no
        // trailing `:` must fall through to ordinary call-statement
        // parsing (e.g. a user's own same-named function), not be
        // rejected just because the text happens to start the same way a
        // block header would.
        assert!(
            parse_call_header("db_transaction(\"x\")", "db_transaction", 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_variable_named_span_can_be_assigned_and_read_without_being_hijacked() {
        // Regression test for a real bug this fix also closes in the
        // brace parser (src/parser.rs), not just here: neither
        // `db_transaction` nor `span` is reserved, but the brace parser
        // used to treat *any* identifier token named "span" as the start
        // of a `span(...) { ... }` block unconditionally — so a plain
        // `span = 5` assignment failed with "expected LParen, got Eq"
        // before ever reaching the block-vs-plain-identifier question at
        // all. A 1-token lookahead (is the very next token `(`?) fixes
        // this specific, more common case (bare reference/assignment).
        //
        // A narrower case remains open on both parsers: calling a
        // same-named function/closure with parens but *no* trailing
        // `{ ... }` (e.g. `span(21)` alone, no block) still gets
        // mis-parsed, since distinguishing that from `span("x"):` (this
        // block form) requires arbitrary lookahead/backtracking past the
        // argument expression to see whether a block follows — out of
        // scope for this fix; call it something else if you need to.
        run_src(
            r#"
Agent demo

On start:
  span = 5
  memory.doubled = span * 2
"#,
        )
        .unwrap();
    }
}
