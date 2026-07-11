//! GX runtime values — what variables hold at runtime.

use crate::ast::Stmt;
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
    /// Closure: (params, body, captured_env)
    /// captured_env is a snapshot of the enclosing scope's local variables at
    /// the point the lambda was created — this is what enables closures to work.
    Closure(Vec<String>, Vec<Stmt>, HashMap<String, Value>),
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
            Value::Closure(..) => true,
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
            Value::Closure(..) => "function",
        }
    }

    pub fn get_field(&self, field: &str) -> Value {
        match self {
            Value::Object(map) => map.get(field).cloned().unwrap_or(Value::Null),
            Value::Array(arr) => match field {
                "length" | "len" => Value::Number(arr.len() as f64),
                _ => Value::Null,
            },
            Value::Str(s) => match field {
                "length" | "len" => Value::Number(s.len() as f64),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    pub fn set_field(&mut self, field: &str, val: Value) -> Result<(), String> {
        match self {
            Value::Object(map) => {
                map.insert(field.to_string(), val);
                Ok(())
            }
            other => Err(format!(
                "Cannot set field '{}' on {}",
                field,
                other.type_name()
            )),
        }
    }

    pub fn get_index(&self, idx: &Value) -> Value {
        match (self, idx) {
            (Value::Array(arr), Value::Number(n)) => {
                // A negative index that still lands before the start of the
                // array (e.g. `arr[-100]` on a 3-element array) must miss
                // just like a positive out-of-bounds index does (`arr[100]`
                // -> null) — previously `.max(0)` clamped it to index 0
                // instead, so `arr[-100]` silently returned the *first*
                // element rather than `null`.
                let i = if *n < 0.0 {
                    arr.len() as i64 + *n as i64
                } else {
                    *n as i64
                };
                if i < 0 {
                    Value::Null
                } else {
                    arr.get(i as usize).cloned().unwrap_or(Value::Null)
                }
            }
            (Value::Str(s), Value::Number(n)) => {
                let chars: Vec<char> = s.chars().collect();
                let i = if *n < 0.0 {
                    chars.len() as i64 + *n as i64
                } else {
                    *n as i64
                };
                if i < 0 {
                    Value::Null
                } else {
                    chars
                        .get(i as usize)
                        .map(|c| Value::Str(c.to_string()))
                        .unwrap_or(Value::Null)
                }
            }
            (Value::Object(map), Value::Str(key)) => map.get(key).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Str(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn iter(&self) -> Result<Vec<Value>, String> {
        match self {
            Value::Array(arr) => Ok(arr.clone()),
            Value::Object(map) => Ok(map.keys().map(|k| Value::Str(k.clone())).collect()),
            Value::Str(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            other => Err(format!("Cannot iterate over {}", other.type_name())),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Push an element onto an array, mutating in place. Returns error if not array.
    pub fn push_mut(&mut self, val: Value) -> Result<(), String> {
        match self {
            Value::Array(arr) => {
                arr.push(val);
                Ok(())
            }
            other => Err(format!("Cannot push onto {}", other.type_name())),
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
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Object(map) => {
                write!(f, "{{")?;
                let mut first = true;
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for k in keys {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, map[k])?;
                    first = false;
                }
                write!(f, "}}")
            }
            Value::Closure(params, _body, _captured) => write!(f, "<fn({})>", params.join(", ")),
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
            // `HashMap<String, Value>::eq` is already key/value-set
            // equality (order-independent) — exactly what `{x:1,y:2} ==
            // {y:2,x:1}` should mean in a language with no defined object
            // key order. Missing before this fix: every `(Object, Object)`
            // pair fell through to the catch-all `false` below, so no two
            // structurally-identical objects — and, transitively, no two
            // arrays or nested structures *containing* an object — were
            // ever `==` to each other, no matter their contents.
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Closure(..), _) | (_, Value::Closure(..)) => false,
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

    #[test]
    fn out_of_bounds_negative_array_index_misses_like_positive_out_of_bounds_does() {
        // Regression test: `arr[-100]` on a 3-element array used to clamp
        // to index 0 (`.max(0)`) and silently return the *first* element,
        // while `arr[100]` correctly returned `null` — an asymmetry a
        // caller doing bounds-agnostic negative indexing (`arr[-i]`) could
        // never safely detect.
        let arr = Value::Array(vec![
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(3.0),
        ]);
        assert_eq!(arr.get_index(&Value::Number(-100.0)), Value::Null);
        assert_eq!(arr.get_index(&Value::Number(100.0)), Value::Null);
        assert_eq!(arr.get_index(&Value::Number(-4.0)), Value::Null);
        assert_eq!(arr.get_index(&Value::Number(-3.0)), Value::Number(1.0));
        assert_eq!(arr.get_index(&Value::Number(-1.0)), Value::Number(3.0));
    }

    #[test]
    fn out_of_bounds_negative_string_index_misses_like_positive_out_of_bounds_does() {
        let s = Value::Str("abc".to_string());
        assert_eq!(s.get_index(&Value::Number(-100.0)), Value::Null);
        assert_eq!(s.get_index(&Value::Number(100.0)), Value::Null);
        assert_eq!(s.get_index(&Value::Number(-4.0)), Value::Null);
        assert_eq!(
            s.get_index(&Value::Number(-3.0)),
            Value::Str("a".to_string())
        );
    }

    #[test]
    fn structurally_identical_objects_are_equal() {
        // Regression test: PartialEq's match previously had no arm for
        // (Object, Object) at all, falling through to the catch-all
        // `false` — meaning `{x:1} == {x:1}` was always false in GX,
        // no matter how identical the two objects were.
        let a = obj(&[("x", Value::Number(1.0)), ("y", Value::Number(2.0))]);
        let b = obj(&[("x", Value::Number(1.0)), ("y", Value::Number(2.0))]);
        assert_eq!(a, b);
    }

    #[test]
    fn object_equality_is_independent_of_key_insertion_order() {
        let a = obj(&[("x", Value::Number(1.0)), ("y", Value::Number(2.0))]);
        let b = obj(&[("y", Value::Number(2.0)), ("x", Value::Number(1.0))]);
        assert_eq!(a, b);
    }

    #[test]
    fn objects_with_different_values_are_not_equal() {
        let a = obj(&[("x", Value::Number(1.0))]);
        let b = obj(&[("x", Value::Number(2.0))]);
        assert_ne!(a, b);
    }

    #[test]
    fn objects_with_different_keys_are_not_equal() {
        let a = obj(&[("x", Value::Number(1.0))]);
        let b = obj(&[("y", Value::Number(1.0))]);
        assert_ne!(a, b);
    }

    #[test]
    fn nested_objects_inside_arrays_compare_correctly() {
        // The bug was transitive: Vec<Value>::eq delegates to Value::eq
        // for each element, so an array containing an object was also
        // never equal to an identical array, even though the top-level
        // (Array, Array) arm was always correct on its own.
        let a = Value::Array(vec![obj(&[("x", Value::Number(1.0))])]);
        let b = Value::Array(vec![obj(&[("x", Value::Number(1.0))])]);
        assert_eq!(a, b);
    }

    #[test]
    fn closures_are_never_equal_even_to_themselves() {
        let c = Value::Closure(vec![], vec![], HashMap::new());
        assert_ne!(c, c.clone());
    }
}
