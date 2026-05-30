//! Data format builtins — CSV, YAML, TOML parse and stringify.
//! Enterprise data lives in many formats; GX handles them all natively.

use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

// ── CSV ───────────────────────────────────────────────────────────────────────

/// csv_parse(text, has_header?) → array<object> | array<array>
/// With has_header=true (default): returns array of objects keyed by column name.
/// With has_header=false: returns array of string arrays.
pub fn csv_parse_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("csv_parse(text, has_header?)".into()))?;

    let has_header = args.get(1).map(|v| v.is_truthy()).unwrap_or(true);

    let delimiter = args
        .get(2)
        .and_then(|v| v.as_str().map(|s| s.chars().next().unwrap_or(',')))
        .unwrap_or(',');

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(has_header)
        .delimiter(delimiter as u8)
        .from_reader(text.as_bytes());

    if has_header {
        let headers: Vec<String> = reader
            .headers()
            .map_err(|e| Signal::Error(format!("csv_parse: bad headers: {}", e)))?
            .iter()
            .map(String::from)
            .collect();

        let mut rows = Vec::new();
        for (i, result) in reader.records().enumerate() {
            let record =
                result.map_err(|e| Signal::Error(format!("csv_parse: row {}: {}", i + 2, e)))?;
            let mut map = HashMap::new();
            for (j, field) in record.iter().enumerate() {
                let key = headers
                    .get(j)
                    .cloned()
                    .unwrap_or_else(|| format!("col{}", j));
                // Auto-coerce numerics
                let val = if let Ok(n) = field.parse::<f64>() {
                    Value::Number(n)
                } else if field == "true" {
                    Value::Bool(true)
                } else if field == "false" {
                    Value::Bool(false)
                } else if field.is_empty() || field == "null" || field == "NULL" {
                    Value::Null
                } else {
                    Value::Str(field.to_string())
                };
                map.insert(key, val);
            }
            rows.push(Value::Object(map));
        }
        Ok(Value::Array(rows))
    } else {
        let mut rows = Vec::new();
        for (i, result) in reader.records().enumerate() {
            let record =
                result.map_err(|e| Signal::Error(format!("csv_parse: row {}: {}", i + 1, e)))?;
            let row: Vec<Value> = record.iter().map(|f| Value::Str(f.to_string())).collect();
            rows.push(Value::Array(row));
        }
        Ok(Value::Array(rows))
    }
}

/// csv_stringify(array, include_header?) → string
pub fn csv_stringify_impl(args: &[Value]) -> Result<Value, Signal> {
    let data = args.first().cloned().unwrap_or(Value::Null);
    let include_header = args.get(1).map(|v| v.is_truthy()).unwrap_or(true);

    let rows = match data {
        Value::Array(v) => v,
        _ => return Err(Signal::Error("csv_stringify: expected an array".into())),
    };

    let mut out = String::new();
    let mut header_written = false;
    let mut col_order: Vec<String> = Vec::new();

    for row in &rows {
        match row {
            Value::Object(map) => {
                if !header_written {
                    col_order = {
                        let mut keys: Vec<String> = map.keys().cloned().collect();
                        keys.sort();
                        keys
                    };
                    if include_header {
                        out.push_str(
                            &col_order
                                .iter()
                                .map(|k| csv_quote(k))
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                        out.push('\n');
                    }
                    header_written = true;
                }
                let line = col_order
                    .iter()
                    .map(|k| csv_quote(&map.get(k).unwrap_or(&Value::Null).to_string()))
                    .collect::<Vec<_>>()
                    .join(",");
                out.push_str(&line);
                out.push('\n');
            }
            Value::Array(cells) => {
                let line = cells
                    .iter()
                    .map(|v| csv_quote(&v.to_string()))
                    .collect::<Vec<_>>()
                    .join(",");
                out.push_str(&line);
                out.push('\n');
            }
            other => {
                out.push_str(&csv_quote(&other.to_string()));
                out.push('\n');
            }
        }
    }
    Ok(Value::Str(out))
}

fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ── YAML ──────────────────────────────────────────────────────────────────────

/// yaml_parse(text) → value
pub fn yaml_parse_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("yaml_parse(text)".into()))?;

    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| Signal::Error(format!("yaml_parse: {}", e)))?;

    Ok(yaml_to_gx(&yaml))
}

/// yaml_stringify(value) → string
pub fn yaml_stringify_impl(args: &[Value]) -> Result<Value, Signal> {
    let val = args.first().cloned().unwrap_or(Value::Null);
    let yaml = gx_to_yaml(&val);
    let text = serde_yaml::to_string(&yaml)
        .map_err(|e| Signal::Error(format!("yaml_stringify: {}", e)))?;
    Ok(Value::Str(text))
}

fn yaml_to_gx(v: &serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_yaml::Value::String(s) => Value::Str(s.clone()),
        serde_yaml::Value::Sequence(arr) => Value::Array(arr.iter().map(yaml_to_gx).collect()),
        serde_yaml::Value::Mapping(m) => {
            let map = m
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => n.to_string(),
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        _ => format!("{:?}", k),
                    };
                    (key, yaml_to_gx(v))
                })
                .collect();
            Value::Object(map)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_gx(&tagged.value),
    }
}

fn gx_to_yaml(v: &Value) -> serde_yaml::Value {
    match v {
        Value::Null => serde_yaml::Value::Null,
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                serde_yaml::Value::Number((*n as i64).into())
            } else {
                serde_yaml::Value::Number(serde_yaml::Number::from(*n))
            }
        }
        Value::Str(s) => serde_yaml::Value::String(s.clone()),
        Value::Array(arr) => serde_yaml::Value::Sequence(arr.iter().map(gx_to_yaml).collect()),
        Value::Object(map) => {
            let mut m = serde_yaml::Mapping::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                m.insert(serde_yaml::Value::String(k.clone()), gx_to_yaml(&map[k]));
            }
            serde_yaml::Value::Mapping(m)
        }
        Value::Closure(params, _, _) => {
            serde_yaml::Value::String(format!("<fn({})>", params.join(", ")))
        }
    }
}

// ── TOML ──────────────────────────────────────────────────────────────────────

/// toml_parse(text) → value
pub fn toml_parse_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("toml_parse(text)".into()))?;

    let toml_val: toml::Value =
        toml::from_str(&text).map_err(|e| Signal::Error(format!("toml_parse: {}", e)))?;

    Ok(toml_to_gx(&toml_val))
}

/// toml_stringify(value) → string
pub fn toml_stringify_impl(args: &[Value]) -> Result<Value, Signal> {
    let val = args.first().cloned().unwrap_or(Value::Null);
    let toml_val = gx_to_toml(&val).ok_or_else(|| {
        Signal::Error(
            "toml_stringify: value cannot be represented as TOML root (must be an object)".into(),
        )
    })?;
    let text = toml::to_string_pretty(&toml_val)
        .map_err(|e| Signal::Error(format!("toml_stringify: {}", e)))?;
    Ok(Value::Str(text))
}

fn toml_to_gx(v: &toml::Value) -> Value {
    match v {
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Integer(n) => Value::Number(*n as f64),
        toml::Value::Float(f) => Value::Number(*f),
        toml::Value::String(s) => Value::Str(s.clone()),
        toml::Value::Datetime(dt) => Value::Str(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.iter().map(toml_to_gx).collect()),
        toml::Value::Table(table) => {
            let map = table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_gx(v)))
                .collect();
            Value::Object(map)
        }
    }
}

fn gx_to_toml(v: &Value) -> Option<toml::Value> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                Some(toml::Value::Integer(*n as i64))
            } else {
                Some(toml::Value::Float(*n))
            }
        }
        Value::Str(s) => Some(toml::Value::String(s.clone())),
        Value::Array(arr) => {
            let items: Vec<toml::Value> = arr.iter().filter_map(gx_to_toml).collect();
            Some(toml::Value::Array(items))
        }
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(tv) = gx_to_toml(&map[k]) {
                    table.insert(k.clone(), tv);
                }
            }
            Some(toml::Value::Table(table))
        }
        Value::Closure(params, _, _) => {
            Some(toml::Value::String(format!("<fn({})>", params.join(", "))))
        }
    }
}
