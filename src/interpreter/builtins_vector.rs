//! In-memory vector store with cosine similarity search.
//! Backed optionally by SQLite for persistence. Pure Rust, no external ML deps.

use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

/// Global store: store_name → list of (id, label, embedding)
static STORES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, VectorStore>>> =
    std::sync::OnceLock::new();

fn stores() -> std::sync::MutexGuard<'static, HashMap<String, VectorStore>> {
    STORES
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[derive(Default)]
struct VectorStore {
    entries: Vec<VectorEntry>,
}

struct VectorEntry {
    id: String,
    label: String,
    embedding: Vec<f32>,
    metadata: HashMap<String, Value>,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

fn extract_embedding(v: &Value) -> Result<Vec<f32>, Signal> {
    match v {
        Value::Array(arr) => {
            let mut floats = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::Number(n) => floats.push(*n as f32),
                    _ => {
                        return Err(Signal::Error(
                            "vector store: embedding must be an array of numbers".into(),
                        ))
                    }
                }
            }
            Ok(floats)
        }
        _ => Err(Signal::Error(
            "vector store: embedding must be an array of numbers".into(),
        )),
    }
}

/// vector_store_new(name?) → name string
pub fn vector_store_new_impl(args: &[Value]) -> Result<Value, Signal> {
    let name = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "default".to_string());
    stores().entry(name.clone()).or_default();
    Ok(Value::Str(name))
}

/// vector_store_add(store_name, id, embedding, label?, metadata?) → null
pub fn vector_store_add_impl(args: &[Value]) -> Result<Value, Signal> {
    let store_name = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| {
            Signal::Error("vector_store_add(store_name, id, embedding, label?, metadata?)".into())
        })?;
    let id = args
        .get(1)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            format!(
                "doc_{}",
                stores()
                    .get(&store_name)
                    .map(|s| s.entries.len())
                    .unwrap_or(0)
            )
        });
    let embedding = extract_embedding(args.get(2).unwrap_or(&Value::Null))?;
    let label = args
        .get(3)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| id.clone());
    let metadata = match args.get(4).cloned().unwrap_or(Value::Null) {
        Value::Object(m) => m,
        _ => HashMap::new(),
    };

    let mut stores = stores();
    let store = stores.entry(store_name).or_default();
    // Remove existing entry with same id
    store.entries.retain(|e| e.id != id);
    store.entries.push(VectorEntry {
        id,
        label,
        embedding,
        metadata,
    });
    Ok(Value::Null)
}

/// vector_store_search(store_name, query_embedding, top_k?) → array<{ id, label, score, metadata }>
pub fn vector_store_search_impl(args: &[Value]) -> Result<Value, Signal> {
    let store_name = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| {
            Signal::Error("vector_store_search(store_name, query_embedding, top_k?)".into())
        })?;
    let query = extract_embedding(args.get(1).unwrap_or(&Value::Null))?;
    let top_k = args.get(2).and_then(|v| v.as_number()).unwrap_or(5.0) as usize;

    let stores = stores();
    let store = match stores.get(&store_name) {
        Some(s) => s,
        None => return Ok(Value::Array(Vec::new())),
    };

    let mut scored: Vec<(f32, usize)> = store
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (cosine_similarity(&query, &e.embedding), i))
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);

    let results: Vec<Value> = scored
        .iter()
        .map(|(score, i)| {
            let entry = &store.entries[*i];
            let mut map = HashMap::new();
            map.insert("id".into(), Value::Str(entry.id.clone()));
            map.insert("label".into(), Value::Str(entry.label.clone()));
            map.insert("score".into(), Value::Number(*score as f64));
            map.insert("metadata".into(), Value::Object(entry.metadata.clone()));
            Value::Object(map)
        })
        .collect();

    Ok(Value::Array(results))
}

/// vector_store_delete(store_name, id) → null
pub fn vector_store_delete_impl(args: &[Value]) -> Result<Value, Signal> {
    let store_name = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("vector_store_delete(store_name, id)".into()))?;
    let id = args
        .get(1)
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("vector_store_delete: id required".into()))?;

    if let Some(store) = stores().get_mut(&store_name) {
        store.entries.retain(|e| e.id != id);
    }
    Ok(Value::Null)
}

/// vector_store_size(store_name) → number
pub fn vector_store_size_impl(args: &[Value]) -> Result<Value, Signal> {
    let store_name = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "default".to_string());
    let size = stores()
        .get(&store_name)
        .map(|s| s.entries.len())
        .unwrap_or(0);
    Ok(Value::Number(size as f64))
}

/// cosine_similarity(vec_a, vec_b) → number
pub fn cosine_similarity_impl(args: &[Value]) -> Result<Value, Signal> {
    let a = extract_embedding(args.first().unwrap_or(&Value::Null))?;
    let b = extract_embedding(args.get(1).unwrap_or(&Value::Null))?;
    Ok(Value::Number(cosine_similarity(&a, &b) as f64))
}
