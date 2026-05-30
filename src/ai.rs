//! GX AI Primitives — ask, embed, infer
//! Connectors: openai, anthropic, ollama (local)

use crate::value::Value;
use std::collections::HashMap;

/// Replace every occurrence of `key` in `s` with `[REDACTED]` so API keys
/// never appear in error messages or logs.
#[cfg(not(target_arch = "wasm32"))]
fn redact(s: &str, key: &str) -> String {
    if key.is_empty() {
        return s.to_string();
    }
    s.replace(key, "[REDACTED]")
}

// ── WASM stubs (no HTTP in browser) ──────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
pub fn ask_ai(provider: &str, _model: Option<&str>, params: &HashMap<String, Value>) -> Value {
    let prompt = params
        .get("prompt")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let mut map = HashMap::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert(
        "text".into(),
        Value::Str(format!(
            "[Playground] AI call to {} received prompt: \"{}\"\nTo use real AI, run GX locally with your API key.",
            provider,
            &prompt[..prompt.len().min(80)]
        )),
    );
    map.insert("confidence".into(), Value::Number(1.0));
    map.insert("tokens_used".into(), Value::Number(0.0));
    map.insert("provider".into(), Value::Str(provider.to_string()));
    Value::Object(map)
}

#[cfg(target_arch = "wasm32")]
pub fn embed_text(_text: &str) -> Value {
    let mut map = HashMap::new();
    map.insert("text".into(), Value::Str(String::new()));
    map.insert(
        "error".into(),
        Value::Str("embed not available in playground".into()),
    );
    map.insert("confidence".into(), Value::Number(0.0));
    map.insert("tokens_used".into(), Value::Number(0.0));
    map.insert("provider".into(), Value::Str("openai".into()));
    map.insert("ok".into(), Value::Bool(false));
    Value::Object(map)
}

#[cfg(target_arch = "wasm32")]
pub fn infer_classifier(_input: &str, _classes: &[String], _provider: &str) -> Value {
    let mut map = HashMap::new();
    map.insert("text".into(), Value::Str(String::new()));
    map.insert(
        "error".into(),
        Value::Str("infer not available in playground".into()),
    );
    map.insert("confidence".into(), Value::Number(0.0));
    map.insert("tokens_used".into(), Value::Number(0.0));
    map.insert("provider".into(), Value::Str("openai".into()));
    map.insert("ok".into(), Value::Bool(false));
    Value::Object(map)
}

// ── Native implementation ─────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub struct AiResponse {
    pub text: String,
    pub confidence: f64,
    pub tokens_used: u64,
    pub model: String,
    pub provider: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl AiResponse {
    pub fn into_value(self) -> Value {
        let mut map = HashMap::new();
        map.insert("text".into(), Value::Str(self.text));
        map.insert("confidence".into(), Value::Number(self.confidence));
        map.insert("tokens_used".into(), Value::Number(self.tokens_used as f64));
        map.insert("model".into(), Value::Str(self.model));
        map.insert("provider".into(), Value::Str(self.provider));
        map.insert("ok".into(), Value::Bool(true));
        Value::Object(map)
    }

    pub fn error(provider: &str, msg: String) -> Value {
        let mut map = HashMap::new();
        map.insert("text".into(), Value::Str(String::new()));
        map.insert("error".into(), Value::Str(msg));
        map.insert("confidence".into(), Value::Number(0.0));
        map.insert("tokens_used".into(), Value::Number(0.0));
        map.insert("provider".into(), Value::Str(provider.to_string()));
        map.insert("ok".into(), Value::Bool(false));
        Value::Object(map)
    }
}

/// A tool schema for AI function calling.
#[cfg(not(target_arch = "wasm32"))]
pub struct AiTool {
    pub name: String,
    pub description: String,
    /// JSON Schema compatible parameter definitions
    pub parameters: serde_json::Value,
}

/// Callback for streaming chunks: called with each text delta as it arrives.
#[cfg(not(target_arch = "wasm32"))]
pub type ChunkCallback = Box<dyn Fn(&str) + Send>;

/// Call an AI model. Returns a Value::Object with text, confidence, tokens_used, model, provider, ok.
/// Extended in v0.4.0: supports streaming (stream: true) and tool definitions.
#[cfg(not(target_arch = "wasm32"))]
pub fn ask_ai(provider: &str, model: Option<&str>, params: &HashMap<String, Value>) -> Value {
    let prompt = params
        .get("prompt")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let system = params
        .get("system")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    let max_tokens = params
        .get("max_tokens")
        .and_then(|v| v.as_number())
        .unwrap_or(1024.0) as u32;

    let temperature = params
        .get("temperature")
        .and_then(|v| v.as_number())
        .unwrap_or(0.7);

    let stream = params.get("stream").map(|v| v.is_truthy()).unwrap_or(false);

    // Tool schemas passed in as JSON-compatible objects
    let tools_val = params.get("tools").cloned();

    match provider {
        "openai" => {
            if stream {
                ask_openai_streaming(
                    model.unwrap_or("gpt-4o-mini"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                )
            } else {
                ask_openai_with_tools(
                    model.unwrap_or("gpt-4o-mini"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                    tools_val.as_ref(),
                )
            }
        }
        "anthropic" => {
            if stream {
                ask_anthropic_streaming(
                    model.unwrap_or("claude-sonnet-4-6"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                )
            } else {
                ask_anthropic_with_tools(
                    model.unwrap_or("claude-sonnet-4-6"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                    tools_val.as_ref(),
                )
            }
        }
        "ollama" => ask_ollama(model.unwrap_or("llama3"), &prompt, system.as_deref()),
        other => AiResponse::error(
            other,
            format!(
                "Unknown AI provider '{}'. Use: openai, anthropic, ollama",
                other
            ),
        ),
    }
}

/// Embed text — returns an array of floats (vector embedding).
#[cfg(not(target_arch = "wasm32"))]
pub fn embed_text(text: &str) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("openai", "OPENAI_API_KEY not set".into()),
    };

    let body = serde_json::json!({
        "model": "text-embedding-3-small",
        "input": text
    });

    match ureq::post("https://api.openai.com/v1/embeddings")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => match resp.into_json::<serde_json::Value>() {
            Ok(json) => {
                if let Some(embedding) = json["data"][0]["embedding"].as_array() {
                    let floats: Vec<Value> = embedding
                        .iter()
                        .filter_map(|v| v.as_f64().map(Value::Number))
                        .collect();
                    Value::Array(floats)
                } else {
                    AiResponse::error("openai", "No embedding in response".into())
                }
            }
            Err(e) => AiResponse::error("openai", format!("Parse error: {}", e)),
        },
        Err(e) => AiResponse::error("openai", format!("Request failed: {}", e)),
    }
}

/// Classify input into one of the provided classes.
#[cfg(not(target_arch = "wasm32"))]
pub fn infer_classifier(input: &str, classes: &[String], provider: &str) -> Value {
    let classes_str = classes.join(", ");
    let prompt = format!(
        "Classify the following text into exactly one of these categories: {}\n\n\
         Text: \"{}\"\n\n\
         Respond with only the category name and a confidence score (0.0-1.0) in JSON format: \
         {{\"label\": \"...\", \"confidence\": 0.0}}",
        classes_str, input
    );

    let mut params = HashMap::new();
    params.insert("prompt".into(), Value::Str(prompt));
    params.insert("max_tokens".into(), Value::Number(100.0));
    params.insert("temperature".into(), Value::Number(0.1));

    let response = ask_ai(provider, None, &params);

    if let Value::Object(ref map) = response {
        if let Some(Value::Str(text)) = map.get("text") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text.trim()) {
                let label = json["label"].as_str().unwrap_or("unknown").to_string();
                let confidence = json["confidence"].as_f64().unwrap_or(0.5);
                let mut result = HashMap::new();
                result.insert("label".into(), Value::Str(label));
                result.insert("confidence".into(), Value::Number(confidence));
                result.insert("ok".into(), Value::Bool(true));
                return Value::Object(result);
            }
        }
    }

    response
}

// ── OpenAI with tool use ──────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_openai_with_tools(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    tools: Option<&Value>,
) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("openai", "OPENAI_API_KEY environment variable not set.\nGet your key at https://platform.openai.com".into()),
    };

    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    messages.push(serde_json::json!({"role": "user", "content": prompt}));

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature
    });

    // Inject tool definitions if provided
    if let Some(tools_val) = tools {
        let tools_json = crate::interpreter::gx_value_to_json(tools_val);
        if let serde_json::Value::Array(ref arr) = tools_json {
            if !arr.is_empty() {
                body["tools"] = tools_json.clone();
                body["tool_choice"] = serde_json::json!("auto");
            }
        }
    }

    match ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => parse_openai_response_with_tools(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error("openai", format!("HTTP {}: {}", code, truncate(&body, 200)))
        }
        Err(e) => AiResponse::error(
            "openai",
            redact(&format!("Request failed: {}", e), &api_key),
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_openai_response_with_tools(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let choice = &json["choices"][0];
            let message = &choice["message"];
            let text = message["content"].as_str().unwrap_or("").to_string();
            let tokens = json["usage"]["total_tokens"].as_u64().unwrap_or(0);
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");

            let mut result = HashMap::new();
            result.insert("text".into(), Value::Str(text.clone()));
            result.insert("tokens_used".into(), Value::Number(tokens as f64));
            result.insert("model".into(), Value::Str(model.to_string()));
            result.insert("provider".into(), Value::Str("openai".into()));
            result.insert("ok".into(), Value::Bool(true));
            result.insert("confidence".into(), Value::Number(0.9));
            result.insert(
                "finish_reason".into(),
                Value::Str(finish_reason.to_string()),
            );

            // Surface tool calls as a structured field
            if let Some(tool_calls) = message["tool_calls"].as_array() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        let mut m = HashMap::new();
                        m.insert(
                            "id".into(),
                            Value::Str(tc["id"].as_str().unwrap_or("").to_string()),
                        );
                        m.insert(
                            "name".into(),
                            Value::Str(tc["function"]["name"].as_str().unwrap_or("").to_string()),
                        );
                        let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                        let args_val = serde_json::from_str::<serde_json::Value>(args_str)
                            .map(|j| crate::interpreter::json_to_gx_value(&j))
                            .unwrap_or(Value::Null);
                        m.insert("arguments".into(), args_val);
                        Value::Object(m)
                    })
                    .collect();
                result.insert("tool_calls".into(), Value::Array(calls));
            }

            Value::Object(result)
        }
        Err(e) => AiResponse::error("openai", format!("Failed to parse response: {}", e)),
    }
}

// ── OpenAI streaming ──────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_openai_streaming(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return AiResponse::error(
                "openai",
                "OPENAI_API_KEY environment variable not set.".into(),
            )
        }
    };

    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    messages.push(serde_json::json!({"role": "user", "content": prompt}));

    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": true
    });

    let resp = match ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let b = redact(&r.into_string().unwrap_or_default(), &api_key);
            return AiResponse::error("openai", format!("HTTP {}: {}", code, truncate(&b, 200)));
        }
        Err(e) => return AiResponse::error("openai", redact(&e.to_string(), &api_key)),
    };

    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(resp.into_reader());
    let mut full_text = String::new();
    let mut tokens = 0u64;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() || line == "data: [DONE]" {
            continue;
        }
        let data = line.strip_prefix("data: ").unwrap_or(&line);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(delta) = json["choices"][0]["delta"]["content"].as_str() {
                full_text.push_str(delta);
                // Print chunk immediately for real-time streaming
                print!("{}", delta);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            if let Some(u) = json["usage"]["total_tokens"].as_u64() {
                tokens = u;
            }
        }
    }
    println!(); // newline after streaming

    let confidence = adjust_confidence_for_hedging(0.9, &full_text);
    AiResponse {
        text: full_text,
        confidence,
        tokens_used: tokens,
        model: model.into(),
        provider: "openai".into(),
    }
    .into_value()
}

// ── Anthropic with tool use ───────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_anthropic_with_tools(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    tools: Option<&Value>,
) -> Value {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("anthropic", "ANTHROPIC_API_KEY environment variable not set.\nGet your key at https://console.anthropic.com".into()),
    };

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [{"role": "user", "content": prompt}]
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }
    if let Some(tools_val) = tools {
        let tools_json = crate::interpreter::gx_value_to_json(tools_val);
        if let serde_json::Value::Array(ref arr) = tools_json {
            if !arr.is_empty() {
                body["tools"] = tools_json.clone();
            }
        }
    }

    match ureq::post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => parse_anthropic_response_with_tools(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error(
                "anthropic",
                format!("HTTP {}: {}", code, truncate(&body, 200)),
            )
        }
        Err(e) => AiResponse::error(
            "anthropic",
            redact(&format!("Request failed: {}", e), &api_key),
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_anthropic_response_with_tools(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let stop_reason = json["stop_reason"].as_str().unwrap_or("end_turn");
            let tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0)
                + json["usage"]["output_tokens"].as_u64().unwrap_or(0);

            let mut text = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();

            if let Some(content) = json["content"].as_array() {
                for block in content {
                    match block["type"].as_str() {
                        Some("text") => text.push_str(block["text"].as_str().unwrap_or("")),
                        Some("tool_use") => {
                            let mut m = HashMap::new();
                            m.insert(
                                "id".into(),
                                Value::Str(block["id"].as_str().unwrap_or("").to_string()),
                            );
                            m.insert(
                                "name".into(),
                                Value::Str(block["name"].as_str().unwrap_or("").to_string()),
                            );
                            let args = crate::interpreter::json_to_gx_value(&block["input"]);
                            m.insert("arguments".into(), args);
                            tool_calls.push(Value::Object(m));
                        }
                        _ => {}
                    }
                }
            }

            let confidence = adjust_confidence_for_hedging(0.9, &text);
            let mut result = HashMap::new();
            result.insert("text".into(), Value::Str(text));
            result.insert("tokens_used".into(), Value::Number(tokens as f64));
            result.insert("model".into(), Value::Str(model.to_string()));
            result.insert("provider".into(), Value::Str("anthropic".into()));
            result.insert("ok".into(), Value::Bool(true));
            result.insert("confidence".into(), Value::Number(confidence));
            result.insert("stop_reason".into(), Value::Str(stop_reason.to_string()));
            if !tool_calls.is_empty() {
                result.insert("tool_calls".into(), Value::Array(tool_calls));
            }
            Value::Object(result)
        }
        Err(e) => AiResponse::error("anthropic", format!("Failed to parse response: {}", e)),
    }
}

// ── Anthropic streaming ───────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_anthropic_streaming(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Value {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return AiResponse::error(
                "anthropic",
                "ANTHROPIC_API_KEY environment variable not set.".into(),
            )
        }
    };

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }

    let resp = match ureq::post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(r) => r,
        Err(e) => return AiResponse::error("anthropic", redact(&e.to_string(), &api_key)),
    };

    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(resp.into_reader());
    let mut full_text = String::new();
    let mut tokens = 0u64;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let data = line
            .trim()
            .strip_prefix("data: ")
            .unwrap_or("")
            .trim()
            .to_string();
        if data.is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
            match json["type"].as_str() {
                Some("content_block_delta") => {
                    if let Some(text) = json["delta"]["text"].as_str() {
                        full_text.push_str(text);
                        print!("{}", text);
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                }
                Some("message_delta") => {
                    if let Some(u) = json["usage"]["output_tokens"].as_u64() {
                        tokens += u;
                    }
                }
                _ => {}
            }
        }
    }
    println!();
    let confidence = adjust_confidence_for_hedging(0.9, &full_text);
    AiResponse {
        text: full_text,
        confidence,
        tokens_used: tokens,
        model: model.into(),
        provider: "anthropic".into(),
    }
    .into_value()
}

// ── OpenAI ────────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_openai(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("openai", "OPENAI_API_KEY environment variable not set.\nGet your key at https://platform.openai.com".into()),
    };

    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    messages.push(serde_json::json!({"role": "user", "content": prompt}));

    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature
    });

    match ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => parse_openai_response(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error("openai", format!("HTTP {}: {}", code, truncate(&body, 200)))
        }
        Err(e) => AiResponse::error(
            "openai",
            redact(&format!("Request failed: {}", e), &api_key),
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_openai_response(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let text = json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tokens = json["usage"]["total_tokens"].as_u64().unwrap_or(0);
            let confidence = match json["choices"][0]["finish_reason"].as_str() {
                Some("stop") => 0.9,
                Some("length") => 0.7,
                _ => 0.8,
            };
            let confidence = adjust_confidence_for_hedging(confidence, &text);
            AiResponse {
                text,
                confidence,
                tokens_used: tokens,
                model: model.into(),
                provider: "openai".into(),
            }
            .into_value()
        }
        Err(e) => AiResponse::error("openai", format!("Failed to parse response: {}", e)),
    }
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_anthropic(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Value {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("anthropic", "ANTHROPIC_API_KEY environment variable not set.\nGet your key at https://console.anthropic.com".into()),
    };

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [{"role": "user", "content": prompt}]
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }

    match ureq::post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => parse_anthropic_response(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error(
                "anthropic",
                format!("HTTP {}: {}", code, truncate(&body, 200)),
            )
        }
        Err(e) => AiResponse::error(
            "anthropic",
            redact(&format!("Request failed: {}", e), &api_key),
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_anthropic_response(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let text = json["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0)
                + json["usage"]["output_tokens"].as_u64().unwrap_or(0);
            let confidence = adjust_confidence_for_hedging(0.9, &text);
            AiResponse {
                text,
                confidence,
                tokens_used: tokens,
                model: model.into(),
                provider: "anthropic".into(),
            }
            .into_value()
        }
        Err(e) => AiResponse::error("anthropic", format!("Failed to parse response: {}", e)),
    }
}

// ── Ollama (local) ────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn ask_ollama(model: &str, prompt: &str, system: Option<&str>) -> Value {
    let base_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
    let endpoint = format!("{}/api/generate", base_url);

    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }

    match ureq::post(&endpoint)
        .set("Content-Type", "application/json")
        .send_json(&body)
    {
        Ok(resp) => match resp.into_json::<serde_json::Value>() {
            Ok(json) => {
                let text = json["response"].as_str().unwrap_or("").to_string();
                let tokens = json["eval_count"].as_u64().unwrap_or(0);
                let confidence = adjust_confidence_for_hedging(0.85, &text);
                AiResponse {
                    text,
                    confidence,
                    tokens_used: tokens,
                    model: model.into(),
                    provider: "ollama".into(),
                }
                .into_value()
            }
            Err(e) => AiResponse::error("ollama", format!("Parse error: {}", e)),
        },
        Err(ureq::Error::Status(404, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            AiResponse::error(
                "ollama",
                format!(
                    "Model '{}' not found in Ollama.\nRun `ollama list` to see available models.\nOllama said: {}",
                    model,
                    truncate(&body, 300)
                ),
            )
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            AiResponse::error(
                "ollama",
                format!("Ollama returned HTTP {}: {}", code, truncate(&body, 300)),
            )
        }
        Err(e) => AiResponse::error(
            "ollama",
            format!(
                "Cannot connect to Ollama at {}. Is it running? Start with: ollama serve\nError: {}",
                base_url, e
            ),
        ),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn adjust_confidence_for_hedging(base: f64, text: &str) -> f64 {
    let lower = text.to_lowercase();
    let hedges = [
        "i'm not sure",
        "i think",
        "maybe",
        "possibly",
        "i believe",
        "not certain",
        "might be",
        "could be",
        "i'm unsure",
        "unclear",
    ];
    let hedge_count = hedges.iter().filter(|h| lower.contains(*h)).count();
    (base - hedge_count as f64 * 0.05).clamp(0.1, 1.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
