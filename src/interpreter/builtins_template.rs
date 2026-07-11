//! Template & Code Generation Runtime — `render_template(template, data)`.
//!
//! GX already has a full programming language for generating text: string
//! interpolation (`"{expr}"`), `while`/`for each`, and `write_file` already
//! let a script build up and emit arbitrary text or source code. That
//! covers "generate text from code I'm writing right now." What it can't
//! do is the other common shape: "render an *external* template — loaded
//! from a file or a string, written once, reused many times — against a
//! data object I only have at runtime," because `"{expr}"` interpolation
//! resolves against variables in scope at the exact point the string
//! literal appears in the source, not against an arbitrary value passed
//! into a function. `render_template` fills exactly that gap and nothing
//! more: `{{dotted.path}}` substitution against a data value, no
//! embedded control flow (no `{{#if}}`/`{{#each}}` mini-language) — a
//! caller who needs a repeated block already has `while`/`for each` for
//! that, one call to `render_template` per item plus ordinary string
//! concatenation. Deliberately not a web template engine: no HTML
//! auto-escaping (this is for source files, config files, and docs, not
//! rendering untrusted values into an HTML response), no expression
//! evaluation inside `{{ }}` (only a plain dotted path).

use super::Signal;
use crate::value::Value;

/// Resolve a dotted path (`"user.name"`, `"items.0.id"`) against `root`.
/// Each segment is looked up as an object field first; if the current
/// value is an array and the segment parses as a number, it's used as an
/// index instead — the same two access modes GX's own `.field`/`[index]`
/// syntax already covers, just driven by a runtime path string instead of
/// AST nodes. Returns `None` (not `Value::Null`) when the path doesn't
/// resolve, so the caller can tell "missing" apart from "present and
/// actually null."
fn resolve_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(current)
}

/// render_template(template, data) → string
///
/// Every `{{dotted.path}}` in `template` is replaced with the
/// corresponding value from `data`, stringified the same way string
/// interpolation already stringifies an embedded expression
/// (`Value::to_string`). A path that doesn't resolve is a rendering
/// error — a silently-blanked placeholder can produce syntactically
/// invalid generated code (`class {{name}} {` → `class  {`), which is a
/// worse failure mode than refusing to render at all. Literal `{{`/`}}`
/// in the output (e.g. generating a file that itself contains template
/// syntax) are written as `\{{`/`\}}`.
pub fn render_template_impl(args: &[Value]) -> Result<Value, Signal> {
    let template = args
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| Signal::Error("render_template(template, data)".into()))?;
    let data = args.get(1).cloned().unwrap_or(Value::Null);

    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // \{{ / \}} — escaped literal braces: consume the backslash, emit
        // the following two brace characters unchanged.
        if chars[i] == '\\'
            && i + 2 < chars.len()
            && ((chars[i + 1] == '{' && chars[i + 2] == '{')
                || (chars[i + 1] == '}' && chars[i + 2] == '}'))
        {
            out.push(chars[i + 1]);
            out.push(chars[i + 2]);
            i += 3;
            continue;
        }
        if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '{' {
            let start = i + 2;
            let mut end = start;
            let mut closed = false;
            while end + 1 < chars.len() {
                if chars[end] == '}' && chars[end + 1] == '}' {
                    closed = true;
                    break;
                }
                end += 1;
            }
            if !closed {
                return Err(Signal::Error(format!(
                    "render_template: unclosed '{{{{' starting at character {}",
                    i
                )));
            }
            let path: String = chars[start..end].iter().collect();
            let path = path.trim();
            if path.is_empty() {
                return Err(Signal::Error(format!(
                    "render_template: empty placeholder '{{{{}}}}' at character {}",
                    i
                )));
            }
            match resolve_path(&data, path) {
                Some(v) => out.push_str(&v.to_string()),
                None => {
                    return Err(Signal::Error(format!(
                        "render_template: '{}' not found in the given data",
                        path
                    )))
                }
            }
            i = end + 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Ok(Value::Str(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Object(m)
    }

    fn render(template: &str, data: Value) -> Result<String, Signal> {
        match render_template_impl(&[Value::Str(template.to_string()), data])? {
            Value::Str(s) => Ok(s),
            _ => unreachable!(),
        }
    }

    #[test]
    fn substitutes_a_simple_placeholder() {
        let data = obj(&[("name", Value::Str("Ada".to_string()))]);
        assert_eq!(render("Hello, {{name}}!", data).unwrap(), "Hello, Ada!");
    }

    #[test]
    fn substitutes_a_nested_dotted_path() {
        let data = obj(&[("user", obj(&[("name", Value::Str("Ada".to_string()))]))]);
        assert_eq!(render("Hi {{user.name}}", data).unwrap(), "Hi Ada");
    }

    #[test]
    fn substitutes_an_array_index_in_the_path() {
        let data = obj(&[("items", Value::Array(vec![Value::Str("first".to_string())]))]);
        assert_eq!(render("{{items.0}}", data).unwrap(), "first");
    }

    #[test]
    fn stringifies_a_number_the_same_way_interpolation_does() {
        let data = obj(&[("count", Value::Number(42.0))]);
        assert_eq!(render("n={{count}}", data).unwrap(), "n=42");
    }

    #[test]
    fn missing_path_is_a_loud_error_not_a_blank_substitution() {
        let data = obj(&[("name", Value::Str("Ada".to_string()))]);
        let err = render("{{missing}}", data).unwrap_err();
        let Signal::Error(msg) = err else {
            panic!("expected Signal::Error");
        };
        assert!(msg.contains("missing"), "message was: {}", msg);
    }

    #[test]
    fn unclosed_placeholder_is_an_error() {
        let data = Value::Null;
        assert!(render("{{name", data).is_err());
    }

    #[test]
    fn escaped_braces_are_emitted_literally_and_not_treated_as_a_placeholder() {
        let data = Value::Null;
        assert_eq!(
            render(r"use \{{ and \}} literally", data).unwrap(),
            "use {{ and }} literally"
        );
    }

    #[test]
    fn template_with_no_placeholders_passes_through_unchanged() {
        let data = Value::Null;
        assert_eq!(
            render("plain text, no braces here", data).unwrap(),
            "plain text, no braces here"
        );
    }

    #[test]
    fn multiple_placeholders_all_substitute() {
        let data = obj(&[
            ("a", Value::Str("X".to_string())),
            ("b", Value::Str("Y".to_string())),
        ]);
        assert_eq!(render("{{a}}-{{b}}-{{a}}", data).unwrap(), "X-Y-X");
    }
}
