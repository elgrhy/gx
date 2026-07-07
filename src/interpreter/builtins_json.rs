//! JSON ↔ GX Value conversion helpers.

use crate::value::Value;
use std::collections::HashMap;

pub fn json_to_gx_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_gx_value).collect()),
        serde_json::Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_gx_value(v)))
                .collect(),
        ),
    }
}

pub fn gx_value_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            // Serialize whole-number floats as JSON integers so that APIs (e.g. OpenAI)
            // that require integer fields don't reject values like 50.0.
            if n.fract() == 0.0 && n.abs() < 1e15 {
                serde_json::Value::Number(serde_json::Number::from(*n as i64))
            } else {
                serde_json::json!(n)
            }
        }
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(gx_value_to_json).collect()),
        Value::Object(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), gx_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Closure(params, _, _) => {
            serde_json::Value::String(format!("<fn({})>", params.join(", ")))
        }
    }
}

// Suppress unused-import warning (HashMap used implicitly in json_to_gx_value).
const _: Option<HashMap<String, Value>> = None;

/// Recursively estimate a `Value`'s JSON-serialized size, bailing out early
/// (returning `None`) the instant the running total exceeds `budget`.
///
/// This exists so a size-limited caller (e.g. `jwt_sign`'s payload cap) can
/// reject an oversized value *before* calling `gx_value_to_json` — which
/// allocates a full parallel `serde_json::Value` tree — and before
/// `serde_json::to_string`, which serializes it just to measure the result.
/// The estimate is deliberately approximate (it ignores JSON string-escaping
/// overhead, exact number formatting, etc.) and is used *only* as a
/// fast-reject pre-filter — a caller must still measure the real serialized
/// length precisely afterward for anything that passes this check. Because
/// escaping can make the true encoding larger than this estimate, treating a
/// `Some(_)` result as a guarantee of being within budget would be wrong;
/// treating a `None` result as "definitely reject without converting" is
/// safe either way, since a value already over budget by this generous an
/// estimate is certainly over budget for real.
pub fn estimate_json_size_within(value: &Value, budget: usize) -> Option<usize> {
    fn add(acc: &mut usize, n: usize, budget: usize) -> bool {
        *acc = acc.saturating_add(n);
        *acc <= budget
    }
    fn walk(value: &Value, acc: &mut usize, budget: usize) -> bool {
        match value {
            Value::Null => add(acc, 4, budget),             // "null"
            Value::Bool(_) => add(acc, 5, budget),          // "false"
            Value::Number(_) => add(acc, 24, budget),       // generous upper bound
            Value::Str(s) => add(acc, s.len() + 2, budget), // + 2 for quotes; ignores escaping overhead
            Value::Array(items) => {
                if !add(acc, 2, budget) {
                    return false;
                }
                for item in items {
                    if !walk(item, acc, budget) {
                        return false;
                    }
                }
                true
            }
            Value::Object(map) => {
                if !add(acc, 2, budget) {
                    return false;
                }
                for (k, v) in map {
                    if !add(acc, k.len() + 3, budget) {
                        return false;
                    }
                    if !walk(v, acc, budget) {
                        return false;
                    }
                }
                true
            }
            Value::Closure(..) => add(acc, 16, budget),
        }
    }
    let mut acc = 0usize;
    if walk(value, &mut acc, budget) {
        Some(acc)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_json_size_within_accepts_small_value() {
        let mut m = HashMap::new();
        m.insert("sub".to_string(), Value::Str("user-1".to_string()));
        m.insert("exp".to_string(), Value::Number(123.0));
        let v = Value::Object(m);
        assert!(estimate_json_size_within(&v, 1024).is_some());
    }

    #[test]
    fn estimate_json_size_within_rejects_oversized_value_without_full_conversion() {
        let huge = Value::Str("x".repeat(10_000_000));
        assert!(estimate_json_size_within(&huge, 8192).is_none());
    }

    #[test]
    fn estimate_json_size_within_rejects_oversized_nested_value() {
        let mut m = HashMap::new();
        m.insert("data".to_string(), Value::Str("y".repeat(100_000)));
        let v = Value::Object(m);
        assert!(estimate_json_size_within(&v, 8192).is_none());
    }

    #[test]
    fn estimate_json_size_within_accepts_empty_containers() {
        assert!(estimate_json_size_within(&Value::Array(vec![]), 8).is_some());
        assert!(estimate_json_size_within(&Value::Object(HashMap::new()), 8).is_some());
    }
}
