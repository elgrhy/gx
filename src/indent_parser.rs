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
/// Heuristic: first meaningful line uses `Agent Name` or `Helper Name`
/// (no string literal, no opening brace on the same line).
pub fn is_indent_syntax(source: &str) -> bool {
    for raw in source.lines() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        let lower = t.to_lowercase();
        if lower.starts_with("agent ") || lower.starts_with("helper ") {
            // Old syntax: agent "name" {  or  agent "name"\n{
            // New syntax: Agent Name  (no quotes, no braces on same line)
            let rest = t[t.find(' ').unwrap_or(0) + 1..].trim_start();
            return !rest.starts_with('"') && !rest.starts_with('{');
        }
        // Top-level import/use is compatible with both — keep scanning
        if lower.starts_with("import ") || lower.starts_with("use ") {
            continue;
        }
        // Anything else at indent 0 that isn't a brace-style block → new syntax
        return !t.contains('{');
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
                let catch_var = cl
                    .strip_prefix("catch")
                    .map(str::trim)
                    .unwrap_or("err")
                    .to_string();
                let catch_var = if catch_var.is_empty() {
                    "err".to_string()
                } else {
                    catch_var
                };
                let catch_block = sub_block(lines, catch_idx + 1, lines[catch_idx].indent);
                let catch_body = parse_stmts(catch_block, auto_output)?;
                let total = consumed + 1 + catch_block.len();
                return Ok((
                    Stmt::TryCatch {
                        try_body,
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
                catch_var: "err".to_string(),
                catch_body: Vec::new(),
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
            idx += 1;
            continue;
        }
        let lower = line.text.to_lowercase();

        if lower.starts_with("import ") {
            let path = line.text[7..].trim().trim_matches('"').to_string();
            file_imports.push(FileImport {
                path,
                line: line.no,
            });
            idx += 1;
        } else if lower.starts_with("use ") {
            let rest = line.text[4..].trim();
            if let Some(dot) = rest.find('.') {
                imports.push(ImportDecl {
                    namespace: rest[..dot].to_string(),
                    package: rest[dot + 1..].to_string(),
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
            idx += 1;
        }
    }

    Ok(Program {
        file_imports,
        imports,
        functions,
        helpers,
        top_level_brain: None,
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
                    let event = block_lower[3..].trim().to_string();
                    let trigger = match event.as_str() {
                        "start" | "started" => WhenTrigger::Started,
                        other => WhenTrigger::Expr(Expr::Ident(other.to_string())),
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
            can_do: Vec::new(),
            memory,
            receive_block: Vec::new(),
            brain,
            recipes: Vec::new(),
            objectives: Vec::new(),
            when_blocks,
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn run_src(src: &str) -> Result<(), String> {
        let prog = parse(src)?;
        crate::interpreter::Interpreter::new().run_program(&prog)
    }

    #[test]
    fn test_detect_indent_syntax() {
        assert!(is_indent_syntax("Agent greeter\nname = \"World\"\n"));
        assert!(!is_indent_syntax("agent \"greeter\" {\n}\n"));
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
}
