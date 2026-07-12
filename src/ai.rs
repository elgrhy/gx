//! GX AI Primitives — ask, embed, infer
//! Connectors: openai, anthropic, ollama (local)

use crate::value::Value;
use std::collections::HashMap;

/// Default read timeout for a cloud AI provider call — deliberately much
/// longer than `builtins_http`'s own `DEFAULT_READ_TIMEOUT` (30s): a real
/// completion, especially with a larger `max_tokens` or a slower model, can
/// easily take more than 30 seconds, and a timeout that fires mid-generation
/// wastes the tokens already billed for no result. Overridable per call via
/// `timeout` (seconds) in `ask`'s params, same convention `http_get` uses.
#[cfg(not(target_arch = "wasm32"))]
const AI_DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Replace every occurrence of `key` in `s` with `[REDACTED]` so API keys
/// never appear in error messages or logs.
#[cfg(not(target_arch = "wasm32"))]
fn redact(s: &str, key: &str) -> String {
    if key.is_empty() {
        return s.to_string();
    }
    s.replace(key, "[REDACTED]")
}

/// Classify an HTTP status code from an AI provider into a stable,
/// provider-independent `error_kind` — this is the "rate-limit hook"/
/// "retry hook" this runtime provides: not a built-in retry engine (GX
/// already has a general-purpose `retry()` builtin for that), just enough
/// structured signal for a script's own retry closure to tell "the request
/// itself was malformed, retrying won't help" apart from "this is
/// transient, back off and try again" without string-matching an error
/// message.
#[cfg(not(target_arch = "wasm32"))]
fn classify_status(code: u16) -> &'static str {
    match code {
        429 => "rate_limited",
        401 | 403 => "auth_error",
        400 | 404 | 422 => "invalid_request",
        408 => "timeout",
        500..=599 => "server_error",
        _ => "http_error",
    }
}

/// The `Retry-After` header, when present and in the simple
/// seconds-as-an-integer form both OpenAI and Anthropic actually send
/// (RFC 7231 also allows an HTTP-date form; deliberately not parsed here —
/// no date-parsing dependency is worth adding for a form neither provider
/// uses today, and a missing value here just means a script's retry hook
/// falls back to its own backoff policy instead of the provider's
/// suggested one).
#[cfg(not(target_arch = "wasm32"))]
fn retry_after_ms(resp: &ureq::Response) -> Option<u64> {
    resp.header("Retry-After")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| secs * 1000)
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
        map.insert("error_kind".into(), Value::Null);
        map.insert("retry_after_ms".into(), Value::Null);
        Value::Object(map)
    }

    pub fn error(provider: &str, msg: String) -> Value {
        Self::error_with_kind(provider, msg, "unknown", None)
    }

    /// Same shape as `error`, plus a stable `error_kind` (see
    /// `classify_status`) and, when the provider sent one, the
    /// `Retry-After` header in milliseconds — the two fields a script's own
    /// retry/rate-limit handling actually needs.
    pub fn error_with_kind(
        provider: &str,
        msg: String,
        kind: &str,
        retry_after_ms: Option<u64>,
    ) -> Value {
        let mut map = HashMap::new();
        map.insert("text".into(), Value::Str(String::new()));
        map.insert("error".into(), Value::Str(msg));
        map.insert("confidence".into(), Value::Number(0.0));
        map.insert("tokens_used".into(), Value::Number(0.0));
        map.insert("provider".into(), Value::Str(provider.to_string()));
        map.insert("ok".into(), Value::Bool(false));
        map.insert("error_kind".into(), Value::Str(kind.to_string()));
        map.insert(
            "retry_after_ms".into(),
            retry_after_ms
                .map(|ms| Value::Number(ms as f64))
                .unwrap_or(Value::Null),
        );
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
pub fn ask_ai(
    provider: &str,
    model: Option<&str>,
    params: &HashMap<String, Value>,
    agent: &ureq::Agent,
) -> Value {
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

    // A provider-neutral message list (see `Message`/the AI Context Runtime)
    // — when present, this is the full conversation and takes precedence
    // over the flat prompt/system single-turn form. Each provider's
    // connector is responsible for translating it into that provider's own
    // wire format (e.g. Anthropic's `system` is a top-level field, not a
    // message with role "system") — that translation stays inside this
    // module so it never leaks into the interpreter or GX scripts.
    let messages_val = params.get("messages").cloned();

    // Structured-output constraint — OpenAI-only pass-through (see
    // `ask_openai_with_tools`'s doc for why Anthropic isn't emulated here).
    let response_format_val = params.get("response_format").cloned();

    let timeout = params
        .get("timeout")
        .and_then(|v| v.as_number())
        // `secs.clamp(1.0, 300.0)` alone doesn't rescue NaN (`f64::clamp`
        // returns its input unchanged when it's NaN, since every NaN
        // comparison is false) — `crate::clamp_duration_secs` does.
        .map(|secs| crate::clamp_duration_secs(secs.max(1.0), std::time::Duration::from_secs(300)))
        .unwrap_or(AI_DEFAULT_READ_TIMEOUT);

    match provider {
        "openai" => {
            if stream {
                ask_openai_streaming(
                    model.unwrap_or("gpt-4o-mini"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                    agent,
                    timeout,
                )
            } else {
                ask_openai_with_tools(
                    model.unwrap_or("gpt-4o-mini"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                    tools_val.as_ref(),
                    messages_val.as_ref(),
                    response_format_val.as_ref(),
                    agent,
                    timeout,
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
                    agent,
                    timeout,
                )
            } else {
                ask_anthropic_with_tools(
                    model.unwrap_or("claude-sonnet-4-6"),
                    &prompt,
                    system.as_deref(),
                    max_tokens,
                    temperature,
                    tools_val.as_ref(),
                    messages_val.as_ref(),
                    agent,
                    timeout,
                )
            }
        }
        "ollama" => ask_ollama(
            model.unwrap_or("llama3"),
            &prompt,
            system.as_deref(),
            messages_val.as_ref(),
            agent,
            timeout,
        ),
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
pub fn embed_text(text: &str, agent: &ureq::Agent) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("openai", "OPENAI_API_KEY not set".into()),
    };

    let body = serde_json::json!({
        "model": "text-embedding-3-small",
        "input": text
    });

    match agent
        .post("https://api.openai.com/v1/embeddings")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .timeout(AI_DEFAULT_READ_TIMEOUT)
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
        Err(ureq::Error::Status(code, resp)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&resp);
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error_with_kind(
                "openai",
                format!("HTTP {}: {}", code, truncate(&body, 200)),
                kind,
                retry_ms,
            )
        }
        Err(e) => AiResponse::error(
            "openai",
            redact(&format!("Request failed: {}", e), &api_key),
        ),
    }
}

/// Classify input into one of the provided classes.
#[cfg(not(target_arch = "wasm32"))]
pub fn infer_classifier(
    input: &str,
    classes: &[String],
    provider: &str,
    agent: &ureq::Agent,
) -> Value {
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

    let response = ask_ai(provider, None, &params, agent);

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

// ── Provider-neutral message list → OpenAI wire format ───────────────────────

/// Translate the AI Context Runtime's provider-neutral message list into
/// OpenAI's `messages` array shape. OpenAI's format is close enough to the
/// neutral one (system is just another message role, tool results use
/// `tool_call_id`) that this is a fairly direct mapping — the real
/// provider-specific work is in the Anthropic translation below.
#[cfg(not(target_arch = "wasm32"))]
fn gx_messages_to_openai_json(messages: &[Value]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter_map(|m| {
            let Value::Object(m) = m else { return None };
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if role == "assistant" {
                if let Some(Value::Array(calls)) = m.get("tool_calls") {
                    if !calls.is_empty() {
                        let tool_calls: Vec<serde_json::Value> = calls
                            .iter()
                            .filter_map(|c| {
                                let Value::Object(c) = c else { return None };
                                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let args = c
                                    .get("arguments")
                                    .map(crate::interpreter::gx_value_to_json)
                                    .unwrap_or(serde_json::json!({}));
                                Some(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": args.to_string(),
                                    }
                                }))
                            })
                            .collect();
                        return Some(serde_json::json!({
                            "role": "assistant",
                            "content": if content.is_empty() { serde_json::Value::Null } else { serde_json::json!(content) },
                            "tool_calls": tool_calls,
                        }));
                    }
                }
            }
            if role == "tool" {
                let tool_call_id = m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                return Some(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                }));
            }
            Some(serde_json::json!({"role": role, "content": content}))
        })
        .collect()
}

// ── OpenAI with tool use ──────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn ask_openai_with_tools(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    tools: Option<&Value>,
    messages_val: Option<&Value>,
    response_format: Option<&Value>,
    agent: &ureq::Agent,
    timeout: std::time::Duration,
) -> Value {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("openai", "OPENAI_API_KEY environment variable not set.\nGet your key at https://platform.openai.com".into()),
    };

    let messages = match messages_val {
        Some(Value::Array(items)) => {
            // The system prompt always travels via the flat `system`
            // param, never embedded in the message list itself — same
            // single code path whether the caller is a one-shot `ask` or
            // the AI Context Runtime's `context_ask`, and it's what lets
            // this same `system` value be handed to Anthropic's connector
            // unchanged (its `system` is a top-level field, not a message).
            let mut messages = Vec::new();
            if let Some(sys) = system {
                messages.push(serde_json::json!({"role": "system", "content": sys}));
            }
            messages.extend(gx_messages_to_openai_json(items));
            messages
        }
        _ => {
            let mut messages = Vec::new();
            if let Some(sys) = system {
                messages.push(serde_json::json!({"role": "system", "content": sys}));
            }
            messages.push(serde_json::json!({"role": "user", "content": prompt}));
            messages
        }
    };

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

    // Structured-output constraint — a direct pass-through to a param
    // OpenAI's API already accepts natively (`{"type": "json_object"}` or
    // `{"type": "json_schema", "json_schema": {...}}`), so a script gets
    // schema-constrained generation without GX inventing its own schema
    // language. No native equivalent exists on Anthropic's API today (Claude
    // achieves structured output via forced tool-choice instead, a
    // materially different request shape) — deliberately not emulated here
    // rather than build a shim that would need to fake a different
    // provider's protocol under one shared param name.
    if let Some(rf) = response_format {
        body["response_format"] = crate::interpreter::gx_value_to_json(rf);
    }

    match agent
        .post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .timeout(timeout)
        .send_json(&body)
    {
        Ok(resp) => parse_openai_response_with_tools(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&resp);
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error_with_kind(
                "openai",
                format!("HTTP {}: {}", code, truncate(&body, 200)),
                kind,
                retry_ms,
            )
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
    agent: &ureq::Agent,
    timeout: std::time::Duration,
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

    let resp = match agent
        .post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .timeout(timeout)
        .send_json(&body)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&r);
            let b = redact(&r.into_string().unwrap_or_default(), &api_key);
            return AiResponse::error_with_kind(
                "openai",
                format!("HTTP {}: {}", code, truncate(&b, 200)),
                kind,
                retry_ms,
            );
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

/// Translate the provider-neutral message list into Anthropic's `messages`
/// shape. Anthropic has no "system" role in the array at all (handled
/// separately via the flat `system` param, same as the single-turn path —
/// a "system"-role entry here would be a caller bug, so it's just skipped
/// rather than mis-sent) and no "tool" role: a tool result is a `user`
/// message whose content is a `tool_result` block, and an assistant's
/// pending tool calls are `tool_use` content blocks rather than a separate
/// field the way OpenAI represents them.
#[cfg(not(target_arch = "wasm32"))]
fn gx_messages_to_anthropic_json(messages: &[Value]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter_map(|m| {
            let Value::Object(m) = m else { return None };
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match role {
                "system" => None,
                "tool" => {
                    let tool_use_id =
                        m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                    Some(serde_json::json!({
                        "role": "user",
                        "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": content}]
                    }))
                }
                "assistant" => {
                    if let Some(Value::Array(calls)) = m.get("tool_calls") {
                        if !calls.is_empty() {
                            let mut blocks = Vec::new();
                            if !content.is_empty() {
                                blocks.push(serde_json::json!({"type": "text", "text": content}));
                            }
                            for c in calls {
                                let Value::Object(c) = c else { continue };
                                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let input = c
                                    .get("arguments")
                                    .map(crate::interpreter::gx_value_to_json)
                                    .unwrap_or(serde_json::json!({}));
                                blocks.push(
                                    serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input}),
                                );
                            }
                            return Some(serde_json::json!({"role": "assistant", "content": blocks}));
                        }
                    }
                    Some(serde_json::json!({"role": "assistant", "content": content}))
                }
                _ => Some(serde_json::json!({"role": "user", "content": content})),
            }
        })
        .collect()
}

// ── Anthropic with tool use ───────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn ask_anthropic_with_tools(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    tools: Option<&Value>,
    messages_val: Option<&Value>,
    agent: &ureq::Agent,
    timeout: std::time::Duration,
) -> Value {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("anthropic", "ANTHROPIC_API_KEY environment variable not set.\nGet your key at https://console.anthropic.com".into()),
    };

    let messages = match messages_val {
        Some(Value::Array(items)) => gx_messages_to_anthropic_json(items),
        _ => vec![serde_json::json!({"role": "user", "content": prompt})],
    };

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": messages
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

    match agent
        .post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .timeout(timeout)
        .send_json(&body)
    {
        Ok(resp) => parse_anthropic_response_with_tools(resp, model),
        Err(ureq::Error::Status(code, resp)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&resp);
            let body = redact(&resp.into_string().unwrap_or_default(), &api_key);
            AiResponse::error_with_kind(
                "anthropic",
                format!("HTTP {}: {}", code, truncate(&body, 200)),
                kind,
                retry_ms,
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
    agent: &ureq::Agent,
    timeout: std::time::Duration,
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

    let resp = match agent
        .post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .timeout(timeout)
        .send_json(&body)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&r);
            let b = redact(&r.into_string().unwrap_or_default(), &api_key);
            return AiResponse::error_with_kind(
                "anthropic",
                format!("HTTP {}: {}", code, truncate(&b, 200)),
                kind,
                retry_ms,
            );
        }
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

/// Translate the AI Context Runtime's provider-neutral message list into
/// Ollama's `/api/chat` shape (`{"role": ..., "content": ...}`, no tool-call
/// wire format of its own to speak of) — same idea as
/// `gx_messages_to_openai_json`, deliberately simpler since Ollama's chat
/// endpoint doesn't support tool calls.
#[cfg(not(target_arch = "wasm32"))]
fn gx_messages_to_ollama_json(messages: &[Value]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter_map(|m| {
            let Value::Object(m) = m else { return None };
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            Some(serde_json::json!({"role": role, "content": content}))
        })
        .collect()
}

/// Ollama connector. Routed through the same shared, capability-checked,
/// pooled `ureq::Agent` and `timeout` every other provider uses — before
/// this fix, `ask_ollama` called the bare `ureq::post` free function
/// directly, silently dropping both the `timeout` param `ask`'s callers
/// already computed and set (`ask_ai`'s own `timeout` local, honored by
/// every other provider) and the connection pooling/SSRF-resolver wiring
/// the shared agent carries. A slow local model had no way to be bounded,
/// forcing scripts back to hand-rolled `http_post` calls just to get a
/// timeout — which re-triggers the SSRF gate `ask ollama` is otherwise
/// specifically exempt from.
///
/// When `messages_val` carries a non-empty message list (the AI Context
/// Runtime's `context_ask`, or a direct multi-turn `ask ollama { messages:
/// [...] }`), this hits `/api/chat` instead of the single-shot
/// `/api/generate` — Ollama's chat endpoint is already message-array based,
/// so the wire format only needed a thin translation, not a new protocol.
#[cfg(not(target_arch = "wasm32"))]
fn ask_ollama(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    messages_val: Option<&Value>,
    agent: &ureq::Agent,
    timeout: std::time::Duration,
) -> Value {
    let base_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());

    let use_chat = matches!(messages_val, Some(Value::Array(items)) if !items.is_empty());

    let (endpoint, body) = if use_chat {
        let Some(Value::Array(items)) = messages_val else {
            unreachable!()
        };
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(serde_json::json!({"role": "system", "content": sys}));
        }
        messages.extend(gx_messages_to_ollama_json(items));
        (
            format!("{}/api/chat", base_url),
            serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false
            }),
        )
    } else {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false
        });
        if let Some(sys) = system {
            body["system"] = serde_json::json!(sys);
        }
        (format!("{}/api/generate", base_url), body)
    };

    match agent
        .post(&endpoint)
        .set("Content-Type", "application/json")
        .timeout(timeout)
        .send_json(&body)
    {
        Ok(resp) => match resp.into_json::<serde_json::Value>() {
            Ok(json) => {
                let text = if use_chat {
                    json["message"]["content"].as_str().unwrap_or("").to_string()
                } else {
                    json["response"].as_str().unwrap_or("").to_string()
                };
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
            AiResponse::error_with_kind(
                "ollama",
                format!(
                    "Model '{}' not found in Ollama.\nRun `ollama list` to see available models.\nOllama said: {}",
                    model,
                    truncate(&body, 300)
                ),
                "invalid_request",
                None,
            )
        }
        Err(ureq::Error::Status(code, resp)) => {
            let kind = classify_status(code);
            let retry_ms = retry_after_ms(&resp);
            let body = resp.into_string().unwrap_or_default();
            AiResponse::error_with_kind(
                "ollama",
                format!("Ollama returned HTTP {}: {}", code, truncate(&body, 300)),
                kind,
                retry_ms,
            )
        }
        Err(e) => AiResponse::error_with_kind(
            "ollama",
            format!(
                "Cannot connect to Ollama at {}. Is it running? Start with: ollama serve\nError: {}",
                base_url, e
            ),
            "network_error",
            None,
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
        // `&s[..max]` alone panics whenever `max` lands inside a multi-byte
        // UTF-8 character instead of on a char boundary — reachable from
        // any AI-provider-compatible endpoint (including a self-hosted
        // Ollama server, or a custom OpenAI-compatible base URL) whose
        // error response body has a multi-byte character near this offset.
        // Walk back to the nearest valid boundary instead of assuming `max`
        // already is one.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Object(m)
    }

    #[test]
    fn truncate_does_not_panic_when_max_lands_inside_a_multi_byte_char() {
        // 'é' is 2 bytes (0xC3 0xA9); "ab" + 'é' puts a char boundary
        // violation exactly at byte offset 3 if sliced naively.
        let s = "ab\u{e9}cdef";
        let out = truncate(s, 3);
        assert!(out.starts_with("ab"));
        assert!(out.ends_with("..."));
    }

    #[test]
    fn truncate_leaves_short_strings_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn classify_status_covers_the_documented_kinds() {
        assert_eq!(classify_status(429), "rate_limited");
        assert_eq!(classify_status(401), "auth_error");
        assert_eq!(classify_status(403), "auth_error");
        assert_eq!(classify_status(400), "invalid_request");
        assert_eq!(classify_status(404), "invalid_request");
        assert_eq!(classify_status(422), "invalid_request");
        assert_eq!(classify_status(408), "timeout");
        assert_eq!(classify_status(500), "server_error");
        assert_eq!(classify_status(503), "server_error");
        assert_eq!(classify_status(599), "server_error");
        assert_eq!(classify_status(999), "http_error");
    }

    #[test]
    fn error_with_kind_carries_retry_after_ms_through() {
        let result = AiResponse::error_with_kind(
            "openai",
            "rate limited".to_string(),
            "rate_limited",
            Some(2500),
        );
        let Value::Object(m) = result else {
            panic!("expected object");
        };
        assert_eq!(m.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(
            m.get("error_kind"),
            Some(&Value::Str("rate_limited".to_string()))
        );
        assert_eq!(m.get("retry_after_ms"), Some(&Value::Number(2500.0)));
    }

    #[test]
    fn error_without_kind_still_has_a_stable_shape() {
        let result = AiResponse::error("openai", "boom".to_string());
        let Value::Object(m) = result else {
            panic!("expected object");
        };
        assert_eq!(
            m.get("error_kind"),
            Some(&Value::Str("unknown".to_string()))
        );
        assert_eq!(m.get("retry_after_ms"), Some(&Value::Null));
    }

    #[test]
    fn success_response_also_carries_the_error_kind_shape_as_null() {
        // Every AI response — success or failure — has the same fields, so
        // a script can check `result.error_kind` unconditionally without a
        // "field doesn't exist" surprise on the happy path.
        let result = AiResponse {
            text: "hi".into(),
            confidence: 0.9,
            tokens_used: 10,
            model: "gpt-4o-mini".into(),
            provider: "openai".into(),
        }
        .into_value();
        let Value::Object(m) = result else {
            panic!("expected object");
        };
        assert_eq!(m.get("error_kind"), Some(&Value::Null));
        assert_eq!(m.get("retry_after_ms"), Some(&Value::Null));
    }

    #[test]
    fn gx_messages_to_openai_json_translates_plain_turns() {
        let messages = vec![
            obj(&[
                ("role", Value::Str("user".into())),
                ("content", Value::Str("hi".into())),
            ]),
            obj(&[
                ("role", Value::Str("assistant".into())),
                ("content", Value::Str("hello".into())),
            ]),
        ];
        let json = gx_messages_to_openai_json(&messages);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"], "hi");
        assert_eq!(json[1]["role"], "assistant");
        assert_eq!(json[1]["content"], "hello");
    }

    #[test]
    fn gx_messages_to_openai_json_translates_tool_result_messages() {
        let messages = vec![obj(&[
            ("role", Value::Str("tool".into())),
            ("content", Value::Str("42 degrees".into())),
            ("tool_call_id", Value::Str("call_1".into())),
        ])];
        let json = gx_messages_to_openai_json(&messages);
        assert_eq!(json[0]["role"], "tool");
        assert_eq!(json[0]["tool_call_id"], "call_1");
        assert_eq!(json[0]["content"], "42 degrees");
    }

    #[test]
    fn gx_messages_to_openai_json_translates_assistant_tool_calls() {
        let mut call = HashMap::new();
        call.insert("id".to_string(), Value::Str("call_1".into()));
        call.insert("name".to_string(), Value::Str("get_weather".into()));
        let mut args = HashMap::new();
        args.insert("city".to_string(), Value::Str("Paris".into()));
        call.insert("arguments".to_string(), Value::Object(args));

        let messages = vec![obj(&[
            ("role", Value::Str("assistant".into())),
            ("content", Value::Str("".into())),
            ("tool_calls", Value::Array(vec![Value::Object(call)])),
        ])];
        let json = gx_messages_to_openai_json(&messages);
        assert_eq!(json[0]["role"], "assistant");
        assert_eq!(json[0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(json[0]["tool_calls"][0]["type"], "function");
        assert_eq!(json[0]["tool_calls"][0]["function"]["name"], "get_weather");
        // Arguments must be a JSON *string* (OpenAI's wire format), not a nested object.
        assert!(json[0]["tool_calls"][0]["function"]["arguments"].is_string());
        assert!(json[0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap()
            .contains("Paris"));
    }

    #[test]
    fn gx_messages_to_anthropic_json_extracts_no_system_role() {
        // Anthropic has no "system" role in its messages array at all —
        // system travels via the flat `system` param instead (see
        // ask_anthropic_with_tools). A stray system-role entry (shouldn't
        // happen — context_add_message rejects it — but defense in depth)
        // must be dropped, not mis-sent as a user turn.
        let messages = vec![
            obj(&[
                ("role", Value::Str("system".into())),
                ("content", Value::Str("be terse".into())),
            ]),
            obj(&[
                ("role", Value::Str("user".into())),
                ("content", Value::Str("hi".into())),
            ]),
        ];
        let json = gx_messages_to_anthropic_json(&messages);
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["role"], "user");
    }

    #[test]
    fn gx_messages_to_anthropic_json_translates_tool_result_as_a_user_message() {
        let messages = vec![obj(&[
            ("role", Value::Str("tool".into())),
            ("content", Value::Str("42 degrees".into())),
            ("tool_call_id", Value::Str("call_1".into())),
        ])];
        let json = gx_messages_to_anthropic_json(&messages);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"][0]["type"], "tool_result");
        assert_eq!(json[0]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(json[0]["content"][0]["content"], "42 degrees");
    }

    #[test]
    fn gx_messages_to_anthropic_json_translates_assistant_tool_calls_as_content_blocks() {
        let mut call = HashMap::new();
        call.insert("id".to_string(), Value::Str("call_1".into()));
        call.insert("name".to_string(), Value::Str("get_weather".into()));
        call.insert("arguments".to_string(), Value::Object(HashMap::new()));

        let messages = vec![obj(&[
            ("role", Value::Str("assistant".into())),
            ("content", Value::Str("Let me check.".into())),
            ("tool_calls", Value::Array(vec![Value::Object(call)])),
        ])];
        let json = gx_messages_to_anthropic_json(&messages);
        assert_eq!(json[0]["role"], "assistant");
        assert_eq!(json[0]["content"][0]["type"], "text");
        assert_eq!(json[0]["content"][0]["text"], "Let me check.");
        assert_eq!(json[0]["content"][1]["type"], "tool_use");
        assert_eq!(json[0]["content"][1]["id"], "call_1");
        assert_eq!(json[0]["content"][1]["name"], "get_weather");
    }
}
