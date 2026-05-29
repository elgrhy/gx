//! Miscellaneous helper utilities used across the interpreter.

use super::Signal;
use crate::ast::*;
use crate::value::Value;

/// Serialize a GX value to a JSON string (used in HTTP/email helpers).
pub(super) fn value_to_json(v: &Value) -> String {
    serde_json::to_string(&super::builtins_json::gx_value_to_json(v))
        .unwrap_or_else(|_| "null".into())
}

/// Strip HTML tags, decode common entities, collapse whitespace.
pub(super) fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let mut result = String::new();
    let mut prev_ws = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result.trim().to_string()
}

/// Returns true if the helper's brain accesses `input`, meaning it's designed
/// to be called via `spawn agent` rather than run standalone.
pub(super) fn helper_is_callable_only(h: &HelperDef) -> bool {
    if let Some(brain) = &h.brain {
        stmts_use_input(&brain.plan)
            || stmts_use_input(&brain.execute)
            || stmts_use_input(&brain.remember)
            || stmts_use_input(&brain.communicate)
    } else {
        false
    }
}

pub(super) fn stmts_use_input(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_uses_input)
}

fn stmt_uses_input(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Assign { value, .. } => expr_uses_input(value),
        Stmt::PlusAssign { value, .. }
        | Stmt::MinusAssign { value, .. }
        | Stmt::MulAssign { value, .. }
        | Stmt::DivAssign { value, .. } => expr_uses_input(value),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches
                .iter()
                .any(|(c, b)| expr_uses_input(c) || stmts_use_input(b))
                || else_body.as_deref().is_some_and(stmts_use_input)
        }
        Stmt::ForEach { iter, body, .. } => expr_uses_input(iter) || stmts_use_input(body),
        Stmt::While {
            condition, body, ..
        } => expr_uses_input(condition) || stmts_use_input(body),
        Stmt::TryCatch {
            try_body,
            catch_body,
            ..
        } => stmts_use_input(try_body) || stmts_use_input(catch_body),
        Stmt::Log { value, .. }
        | Stmt::Output { value, .. }
        | Stmt::Say { value, .. }
        | Stmt::Return {
            value: Some(value), ..
        }
        | Stmt::Assert {
            condition: value, ..
        }
        | Stmt::Wait { ms: value, .. } => expr_uses_input(value),
        Stmt::Expr { expr, .. } => expr_uses_input(expr),
        Stmt::Think {
            prompt,
            temperature,
            min_confidence,
            ..
        } => {
            expr_uses_input(prompt)
                || temperature.as_ref().is_some_and(expr_uses_input)
                || min_confidence.as_ref().is_some_and(expr_uses_input)
        }
        Stmt::Observe { bindings, .. } => bindings.iter().any(|(_, v)| expr_uses_input(v)),
        Stmt::Act { body, .. } => stmts_use_input(body),
        Stmt::LoopUntil {
            condition, body, ..
        } => expr_uses_input(condition) || stmts_use_input(body),
        Stmt::RepeatTimes { count, body, .. } => expr_uses_input(count) || stmts_use_input(body),
        Stmt::Parallel { branches, .. } => branches.iter().any(|b| stmts_use_input(b)),
        _ => false,
    }
}

pub(super) fn expr_uses_input(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(n) => n == "input",
        Expr::FieldAccess { object, .. } => expr_uses_input(object),
        Expr::Index { object, index } => expr_uses_input(object) || expr_uses_input(index),
        Expr::Call { callee, args } => expr_uses_input(callee) || args.iter().any(expr_uses_input),
        Expr::BinOp { left, right, .. } => expr_uses_input(left) || expr_uses_input(right),
        Expr::Not(inner) => expr_uses_input(inner),
        Expr::Array(items) => items.iter().any(expr_uses_input),
        Expr::Object(pairs) => pairs.iter().any(|(_, v)| expr_uses_input(v)),
        Expr::Interpolated(parts) => parts.iter().any(|p| match p {
            InterpolatedPart::Expr(e) => expr_uses_input(e),
            _ => false,
        }),
        Expr::CallAgent { name, input, .. } => {
            expr_uses_input(name) || input.iter().any(|(_, v)| expr_uses_input(v))
        }
        Expr::ParallelMap(branches) => branches.iter().any(|(_, v)| expr_uses_input(v)),
        _ => false,
    }
}

/// Parse GX source using whichever syntax the file uses.
pub(super) fn parse_gx_source(source: &str, path: &str) -> Result<crate::ast::Program, String> {
    if crate::indent_parser::is_indent_syntax(source) {
        crate::indent_parser::parse(source).map_err(|e| format!("{}: {}", path, e))
    } else {
        let tokens = crate::lexer::Lexer::new(source)
            .tokenize()
            .map_err(|e| format!("{}: {}", path, e))?;
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| format!("{}: {}", path, e))
    }
}

/// Classify an error message string into a typed error kind name.
pub(super) fn infer_error_kind(msg: &str) -> &'static str {
    let lower = msg.to_lowercase();
    if lower.contains("json") || lower.contains("parse") || lower.contains("invalid") {
        "JsonParseError"
    } else if lower.contains("network")
        || lower.contains("connection")
        || lower.contains("timeout")
        || lower.contains("http")
    {
        "NetworkError"
    } else if lower.contains("permission") || lower.contains("access denied") {
        "PermissionError"
    } else if lower.contains("not found") || lower.contains("no such file") {
        "NotFoundError"
    } else if lower.contains("assert") {
        "AssertionError"
    } else {
        "RuntimeError"
    }
}

/// Resolve `..` and `.` without touching the filesystem (no symlink resolution).
pub(super) fn normalize_path_no_symlink(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out.iter().collect()
}

/// Cron expression field matcher. Supports: *, n, */n, n-m
pub(super) fn cron_field_matches(field: &str, value: u64, min: u64, max: u64) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(step) = field.strip_prefix("*/") {
        let step: u64 = step.parse().unwrap_or(1);
        return step > 0 && value.is_multiple_of(step);
    }
    if field.contains('-') {
        let parts: Vec<&str> = field.splitn(2, '-').collect();
        if parts.len() == 2 {
            let lo: u64 = parts[0].parse().unwrap_or(min);
            let hi: u64 = parts[1].parse().unwrap_or(max);
            return value >= lo && value <= hi;
        }
    }
    field.parse::<u64>().map(|n| n == value).unwrap_or(false)
}

/// Match a 5-field cron expression against a Unix timestamp.
pub(super) fn cron_matches(expr: &str, unix_secs: u64) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() < 5 {
        return false;
    }
    let total_minutes = unix_secs / 60;
    let minute = total_minutes % 60;
    let total_hours = total_minutes / 60;
    let hour = total_hours % 24;
    let total_days = total_hours / 24;
    let dow = (total_days + 4) % 7;
    let day_of_year = total_days % 365;
    let dom = day_of_year % 31 + 1;
    let month = day_of_year / 31 + 1;

    cron_field_matches(fields[0], minute, 0, 59)
        && cron_field_matches(fields[1], hour, 0, 23)
        && cron_field_matches(fields[2], dom, 1, 31)
        && cron_field_matches(fields[3], month, 1, 12)
        && cron_field_matches(fields[4], dow, 0, 6)
}

// Suppress the Signal unused-import warning in this module (it's imported for
// the return type of Signal-free helpers above that return plain types).
const _: Option<Signal> = None;
