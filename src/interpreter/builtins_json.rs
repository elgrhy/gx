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
        Value::Number(n) => serde_json::json!(n),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(gx_value_to_json).collect()),
        Value::Object(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), gx_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Closure(params, _) => {
            serde_json::Value::String(format!("<fn({})>", params.join(", ")))
        }
    }
}

// Suppress unused-import warning (HashMap used implicitly in json_to_gx_value).
const _: Option<HashMap<String, Value>> = None;
