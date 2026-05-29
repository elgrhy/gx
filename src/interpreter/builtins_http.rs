//! HTTP client builtins and SSRF protection.

use super::builtins_json::{gx_value_to_json, json_to_gx_value};
use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

/// Shared ureq agent with sane connect/read timeouts used by all HTTP builtins.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .build()
}

#[cfg(target_arch = "wasm32")]
pub(super) fn http_builtin(name: &str, _args: &[Value]) -> Result<Value, Signal> {
    let mut map = HashMap::new();
    map.insert("ok".into(), Value::Bool(false));
    map.insert(
        "error".into(),
        Value::Str(format!("{} not available in playground", name)),
    );
    Ok(Value::Object(map))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_builtin(name: &str, args: &[Value]) -> Result<Value, Signal> {
    let agent = http_agent();
    match name {
        "http_get" | "fetch" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_get requires a URL string".into()))?;
            let headers_val = args.get(1).cloned().unwrap_or(Value::Null);
            let mut req = agent.get(&url);
            if let Value::Object(headers) = headers_val {
                for (k, v) in &headers {
                    req = req.set(k, &v.to_string());
                }
            }
            match req.call() {
                Ok(resp) => {
                    let status = resp.status() as f64;
                    let body = resp.into_string().unwrap_or_default();
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(status < 400.0));
                    map.insert("status".into(), Value::Number(status));
                    map.insert("body".into(), Value::Str(body.clone()));
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                        map.insert("data".into(), json_to_gx_value(&json));
                    }
                    Ok(Value::Object(map))
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("status".into(), Value::Number(code as f64));
                    map.insert("body".into(), Value::Str(body));
                    map.insert("error".into(), Value::Str(format!("HTTP {}", code)));
                    Ok(Value::Object(map))
                }
                Err(e) => {
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("status".into(), Value::Number(0.0));
                    map.insert("error".into(), Value::Str(e.to_string()));
                    Ok(Value::Object(map))
                }
            }
        }
        "http_post" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_post requires a URL string".into()))?;
            let body_val = args.get(1).cloned().unwrap_or(Value::Null);
            let headers_val = args.get(2).cloned().unwrap_or(Value::Null);
            let mut req = agent.post(&url).set("Content-Type", "application/json");
            if let Value::Object(headers) = headers_val {
                for (k, v) in &headers {
                    req = req.set(k, &v.to_string());
                }
            }
            let json_body = gx_value_to_json(&body_val);
            match req.send_json(&json_body) {
                Ok(resp) => {
                    let status = resp.status() as f64;
                    let body = resp.into_string().unwrap_or_default();
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(status < 400.0));
                    map.insert("status".into(), Value::Number(status));
                    map.insert("body".into(), Value::Str(body.clone()));
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                        map.insert("data".into(), json_to_gx_value(&json));
                    }
                    Ok(Value::Object(map))
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("status".into(), Value::Number(code as f64));
                    map.insert("body".into(), Value::Str(body));
                    map.insert("error".into(), Value::Str(format!("HTTP {}", code)));
                    Ok(Value::Object(map))
                }
                Err(e) => {
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("error".into(), Value::Str(e.to_string()));
                    Ok(Value::Object(map))
                }
            }
        }
        "http_put" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_put requires a URL string".into()))?;
            let body_val = args.get(1).cloned().unwrap_or(Value::Null);
            let headers_val = args.get(2).cloned().unwrap_or(Value::Null);
            let json_body = gx_value_to_json(&body_val);
            let mut req = agent.put(&url).set("Content-Type", "application/json");
            if let Value::Object(headers) = headers_val {
                for (k, v) in &headers {
                    req = req.set(k, &v.to_string());
                }
            }
            match req.send_json(&json_body) {
                Ok(resp) => {
                    let status = resp.status() as f64;
                    let body = resp.into_string().unwrap_or_default();
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(status < 400.0));
                    map.insert("status".into(), Value::Number(status));
                    map.insert("body".into(), Value::Str(body.clone()));
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                        map.insert("data".into(), json_to_gx_value(&json));
                    }
                    Ok(Value::Object(map))
                }
                Err(e) => {
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("error".into(), Value::Str(e.to_string()));
                    Ok(Value::Object(map))
                }
            }
        }
        "http_delete" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_delete requires a URL string".into()))?;
            let headers_val = args.get(1).cloned().unwrap_or(Value::Null);
            let mut req = agent.delete(&url);
            if let Value::Object(headers) = headers_val {
                for (k, v) in &headers {
                    req = req.set(k, &v.to_string());
                }
            }
            match req.call() {
                Ok(resp) => {
                    let status = resp.status() as f64;
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(status < 400.0));
                    map.insert("status".into(), Value::Number(status));
                    Ok(Value::Object(map))
                }
                Err(e) => {
                    let mut map = HashMap::new();
                    map.insert("ok".into(), Value::Bool(false));
                    map.insert("error".into(), Value::Str(e.to_string()));
                    Ok(Value::Object(map))
                }
            }
        }
        _ => Err(Signal::Error(format!("Unknown HTTP builtin: {}", name))),
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn http_stream_builtin(_args: &[Value]) -> Result<Value, Signal> {
    Err(Signal::Error(
        "http_stream not available in playground".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_stream_builtin(args: &[Value]) -> Result<Value, Signal> {
    use std::io::{BufRead, BufReader};

    let opts = args.first().cloned().unwrap_or(Value::Null);
    let url = match &opts {
        Value::Object(m) => m
            .get("url")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        Value::Str(s) => s.clone(),
        _ => {
            return Err(Signal::Error(
                "http_stream: expected {url, method?, body?}".into(),
            ))
        }
    };
    let method = match &opts {
        Value::Object(m) => m
            .get("method")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "GET".into()),
        _ => "GET".into(),
    };
    let body_val = match &opts {
        Value::Object(m) => m.get("body").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };

    let resp = match method.to_uppercase().as_str() {
        "POST" => {
            let json_body = gx_value_to_json(&body_val);
            ureq::post(&url)
                .set("Content-Type", "application/json")
                .send_json(&json_body)
                .map_err(|e| Signal::Error(format!("http_stream POST failed: {}", e)))?
        }
        _ => ureq::get(&url)
            .call()
            .map_err(|e| Signal::Error(format!("http_stream GET failed: {}", e)))?,
    };

    let reader = BufReader::new(resp.into_reader());
    let mut chunks: Vec<Value> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(l) => chunks.push(Value::Str(l)),
            Err(e) => return Err(Signal::Error(format!("http_stream read error: {}", e))),
        }
    }
    Ok(Value::Array(chunks))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn http_upload_builtin(_args: &[Value]) -> Result<Value, Signal> {
    Err(Signal::Error(
        "http_upload not available in playground".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_upload_builtin(args: &[Value]) -> Result<Value, Signal> {
    let opts = args
        .first()
        .ok_or_else(|| Signal::Error("http_upload: expected {url, fields?, files?}".into()))?;
    let map = match opts {
        Value::Object(m) => m,
        _ => {
            return Err(Signal::Error(
                "http_upload: expected object argument".into(),
            ))
        }
    };
    let url = map
        .get("url")
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("http_upload: 'url' field required".into()))?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let boundary = format!(
        "GXBoundary{:016x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut body: Vec<u8> = Vec::new();
    if let Some(Value::Object(fields)) = map.get("fields") {
        for (k, v) in fields {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", k).as_bytes(),
            );
            body.extend_from_slice(v.to_string().as_bytes());
            body.extend_from_slice(b"\r\n");
        }
    }
    if let Some(Value::Object(files)) = map.get("files") {
        for (field_name, path_val) in files {
            let path = path_val.as_str().ok_or_else(|| {
                Signal::Error(format!(
                    "http_upload: file path for '{}' must be a string",
                    field_name
                ))
            })?;
            let file_data = std::fs::read(path).map_err(|e| {
                Signal::Error(format!("http_upload: cannot read '{}': {}", path, e))
            })?;
            let filename = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                    field_name, filename
                )
                .as_bytes(),
            );
            body.extend_from_slice(&file_data);
            body.extend_from_slice(b"\r\n");
        }
    }
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let content_type = format!("multipart/form-data; boundary={}", boundary);
    let resp = ureq::post(&url)
        .set("Content-Type", &content_type)
        .send_bytes(&body)
        .map_err(|e| Signal::Error(format!("http_upload failed: {}", e)))?;

    let status = resp.status() as f64;
    let resp_body = resp.into_string().unwrap_or_default();
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(status < 400.0));
    result.insert("status".into(), Value::Number(status));
    result.insert("body".into(), Value::Str(resp_body.clone()));
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp_body) {
        result.insert("data".into(), json_to_gx_value(&json));
    }
    Ok(Value::Object(result))
}

/// Reject URLs that target private / loopback / link-local ranges to prevent SSRF.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn check_url_safe(url: &str, allow_internal: bool) -> Result<(), Signal> {
    if allow_internal {
        return Ok(());
    }
    if url.starts_with("file://") {
        return Err(Signal::Error(
            "HTTP functions do not allow file:// URLs.".into(),
        ));
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host_port = rest.split('/').next().unwrap_or(rest);
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .unwrap_or(host_port)
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    let host_lower = host.to_lowercase();
    if host_lower == "localhost"
        || host_lower == "ip6-localhost"
        || host_lower.ends_with(".localhost")
        || host_lower == "0.0.0.0"
        || host_lower == "::1"
        || host_lower == "::"
    {
        return Err(Signal::Error(format!(
            "SSRF protection: requests to '{}' are blocked. \
             Use --allow-internal-http to allow internal network access.",
            host
        )));
    }
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4 {
        if let (Ok(a), Ok(b)) = (octets[0].parse::<u8>(), octets[1].parse::<u8>()) {
            let blocked = a == 127
                || a == 10
                || a == 0
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
                || (a == 100 && (64..=127).contains(&b));
            if blocked {
                return Err(Signal::Error(format!(
                    "SSRF protection: requests to private/internal address '{}' are blocked. \
                     Use --allow-internal-http to allow internal network access.",
                    host
                )));
            }
        }
    }
    Ok(())
}
