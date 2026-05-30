//! Regex builtins — production-grade pattern matching for LLM output parsing.
//! All functions return structured errors with the offending pattern included.

use super::Signal;
use crate::value::Value;
use regex::Regex;

fn compile(pattern: &str, fn_name: &str) -> Result<Regex, Signal> {
    Regex::new(pattern).map_err(|e| {
        Signal::Error(format!(
            "{}: invalid regex pattern '{}': {}",
            fn_name, pattern, e
        ))
    })
}

fn get_pattern(args: &[Value], fn_name: &str) -> Result<String, Signal> {
    args.get(1)
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error(format!("{}(text, pattern)", fn_name)))
}

fn get_text(args: &[Value]) -> String {
    args.first()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// regex_test(text, pattern) → bool
pub fn regex_test_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_test")?;
    let re = compile(&pat, "regex_test")?;
    Ok(Value::Bool(re.is_match(&text)))
}

/// regex_find(text, pattern) → string | null
/// Returns the first capture group if the pattern has groups, else the full match.
pub fn regex_find_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_find")?;
    let re = compile(&pat, "regex_find")?;
    match re.captures(&text) {
        None => Ok(Value::Null),
        Some(caps) => {
            // Prefer first capture group; fall back to full match
            let m = if caps.len() > 1 {
                caps.get(1)
            } else {
                caps.get(0)
            };
            Ok(m.map(|m| Value::Str(m.as_str().to_string()))
                .unwrap_or(Value::Null))
        }
    }
}

/// regex_find_all(text, pattern) → array<string>
/// Returns all capture-group-1 matches if the pattern has groups, else all full matches.
pub fn regex_find_all_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_find_all")?;
    let re = compile(&pat, "regex_find_all")?;
    let has_groups = re.captures_len() > 1;
    let results: Vec<Value> = if has_groups {
        re.captures_iter(&text)
            .filter_map(|c| c.get(1).map(|m| Value::Str(m.as_str().to_string())))
            .collect()
    } else {
        re.find_iter(&text)
            .map(|m| Value::Str(m.as_str().to_string()))
            .collect()
    };
    Ok(Value::Array(results))
}

/// regex_replace(text, pattern, replacement, replace_all?) → string
/// replace_all defaults to true. Use $1, $2 for back-references in replacement.
pub fn regex_replace_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_replace")?;
    let replacement = args
        .get(2)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let replace_all = args.get(3).map(|v| v.is_truthy()).unwrap_or(true);
    let re = compile(&pat, "regex_replace")?;
    let result = if replace_all {
        re.replace_all(&text, replacement.as_str()).to_string()
    } else {
        re.replace(&text, replacement.as_str()).to_string()
    };
    Ok(Value::Str(result))
}

/// regex_split(text, pattern) → array<string>
pub fn regex_split_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_split")?;
    let re = compile(&pat, "regex_split")?;
    let parts: Vec<Value> = re.split(&text).map(|s| Value::Str(s.to_string())).collect();
    Ok(Value::Array(parts))
}

/// regex_captures(text, pattern) → array<string> | null
/// Returns all capture groups (index 0 = full match, 1+ = groups), or null if no match.
pub fn regex_captures_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_captures")?;
    let re = compile(&pat, "regex_captures")?;
    match re.captures(&text) {
        None => Ok(Value::Null),
        Some(caps) => {
            let groups: Vec<Value> = caps
                .iter()
                .map(|m| {
                    m.map(|ma| Value::Str(ma.as_str().to_string()))
                        .unwrap_or(Value::Null)
                })
                .collect();
            Ok(Value::Array(groups))
        }
    }
}

/// regex_named_captures(text, pattern) → object | null
/// Returns named capture groups as an object, or null if no match.
pub fn regex_named_captures_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = get_text(args);
    let pat = get_pattern(args, "regex_named_captures")?;
    let re = compile(&pat, "regex_named_captures")?;
    match re.captures(&text) {
        None => Ok(Value::Null),
        Some(caps) => {
            let mut map = std::collections::HashMap::new();
            for name in re.capture_names().flatten() {
                if let Some(m) = caps.name(name) {
                    map.insert(name.to_string(), Value::Str(m.as_str().to_string()));
                }
            }
            if map.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(Value::Object(map))
            }
        }
    }
}
