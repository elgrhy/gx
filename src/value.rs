//! GX runtime values — what variables hold at runtime.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    pub fn get_field(&self, field: &str) -> Value {
        match self {
            Value::Object(map) => map.get(field).cloned().unwrap_or(Value::Null),
            Value::Array(arr) => match field {
                "length" => Value::Number(arr.len() as f64),
                _ => Value::Null,
            },
            Value::Str(s) => match field {
                "length" => Value::Number(s.len() as f64),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    pub fn set_field(&mut self, field: &str, val: Value) -> Result<(), String> {
        match self {
            Value::Object(map) => { map.insert(field.to_string(), val); Ok(()) }
            other => Err(format!("Cannot set field '{}' on {}", field, other.type_name()))
        }
    }

    pub fn get_index(&self, idx: &Value) -> Value {
        match (self, idx) {
            (Value::Array(arr), Value::Number(n)) => {
                let i = *n as usize;
                arr.get(i).cloned().unwrap_or(Value::Null)
            }
            (Value::Object(map), Value::Str(key)) => {
                map.get(key).cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self { Value::Number(n) => Some(*n), _ => None }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self { Value::Str(s) => Some(s.as_str()), _ => None }
    }

    pub fn iter(&self) -> Result<Vec<Value>, String> {
        match self {
            Value::Array(arr) => Ok(arr.clone()),
            Value::Object(map) => Ok(map.keys().map(|k| Value::Str(k.clone())).collect()),
            Value::Str(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            other => Err(format!("Cannot iterate over {}", other.type_name())),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::Str(s) => write!(f, "{}", s),
            Value::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Object(map) => {
                write!(f, "{{")?;
                let mut first = true;
                for (k, v) in map {
                    if !first { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                    first = false;
                }
                write!(f, "}}")
            }
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            _ => false,
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
            (Value::Str(a), Value::Str(b)) => a.partial_cmp(b),
            _ => None,
        }
    }
}
