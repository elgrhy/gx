/// GX AI Primitives — ask, embed, infer
/// Connectors: openai, anthropic, ollama (local)

use std::collections::HashMap;
use crate::value::Value;

// ── Public API ────────────────────────────────────────────────────────────────

pub struct AiResponse {
    pub text: String,
    pub confidence: f64,
    pub tokens_used: u64,
    pub model: String,
    pub provider: String,
}

impl AiResponse {
    pub fn to_value(self) -> Value {
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

/// Call an AI model. Returns a Value::Object with text, confidence, tokens_used, model, provider, ok.
pub fn ask_ai(provider: &str, model: Option<&str>, params: &HashMap<String, Value>) -> Value {
    let prompt = params.get("prompt")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let system = params.get("system")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    let max_tokens = params.get("max_tokens")
        .and_then(|v| v.as_number())
        .unwrap_or(1024.0) as u32;

    let temperature = params.get("temperature")
        .and_then(|v| v.as_number())
        .unwrap_or(0.7);

    match provider {
        "openai"    => ask_openai(model.unwrap_or("gpt-4o-mini"), &prompt, system.as_deref(), max_tokens, temperature),
        "anthropic" => ask_anthropic(model.unwrap_or("claude-haiku-4-5-20251001"), &prompt, system.as_deref(), max_tokens, temperature),
        "ollama"    => ask_ollama(model.unwrap_or("llama3"), &prompt, system.as_deref()),
        other       => AiResponse::error(other, format!("Unknown AI provider '{}'. Use: openai, anthropic, ollama", other)),
    }
}

/// Embed text — returns an array of floats (vector embedding).
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
        Ok(resp) => {
            match resp.into_json::<serde_json::Value>() {
                Ok(json) => {
                    if let Some(embedding) = json["data"][0]["embedding"].as_array() {
                        let floats: Vec<Value> = embedding.iter()
                            .filter_map(|v| v.as_f64().map(Value::Number))
                            .collect();
                        Value::Array(floats)
                    } else {
                        AiResponse::error("openai", "No embedding in response".into())
                    }
                }
                Err(e) => AiResponse::error("openai", format!("Parse error: {}", e)),
            }
        }
        Err(e) => AiResponse::error("openai", format!("Request failed: {}", e)),
    }
}

/// Classify input into one of the provided classes.
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
            // Try to parse JSON from response
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

    // Fallback: return the raw response
    response
}

// ── OpenAI ────────────────────────────────────────────────────────────────────

fn ask_openai(model: &str, prompt: &str, system: Option<&str>, max_tokens: u32, temperature: f64) -> Value {
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
            let body = resp.into_string().unwrap_or_default();
            AiResponse::error("openai", format!("HTTP {}: {}", code, truncate(&body, 200)))
        }
        Err(e) => AiResponse::error("openai", format!("Request failed: {}", e)),
    }
}

fn parse_openai_response(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let text = json["choices"][0]["message"]["content"]
                .as_str().unwrap_or("").to_string();
            let tokens = json["usage"]["total_tokens"].as_u64().unwrap_or(0);
            // Estimate confidence from finish_reason
            let confidence = match json["choices"][0]["finish_reason"].as_str() {
                Some("stop") => 0.9,
                Some("length") => 0.7, // truncated — less confident
                _ => 0.8,
            };
            // Reduce confidence if response contains hedging language
            let confidence = adjust_confidence_for_hedging(confidence, &text);
            AiResponse { text, confidence, tokens_used: tokens, model: model.into(), provider: "openai".into() }.to_value()
        }
        Err(e) => AiResponse::error("openai", format!("Failed to parse response: {}", e)),
    }
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

fn ask_anthropic(model: &str, prompt: &str, system: Option<&str>, max_tokens: u32, _temperature: f64) -> Value {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => return AiResponse::error("anthropic", "ANTHROPIC_API_KEY environment variable not set.\nGet your key at https://console.anthropic.com".into()),
    };

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
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
            let body = resp.into_string().unwrap_or_default();
            AiResponse::error("anthropic", format!("HTTP {}: {}", code, truncate(&body, 200)))
        }
        Err(e) => AiResponse::error("anthropic", format!("Request failed: {}", e)),
    }
}

fn parse_anthropic_response(resp: ureq::Response, model: &str) -> Value {
    match resp.into_json::<serde_json::Value>() {
        Ok(json) => {
            let text = json["content"][0]["text"]
                .as_str().unwrap_or("").to_string();
            let tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0)
                       + json["usage"]["output_tokens"].as_u64().unwrap_or(0);
            let confidence = adjust_confidence_for_hedging(0.9, &text);
            AiResponse { text, confidence, tokens_used: tokens, model: model.into(), provider: "anthropic".into() }.to_value()
        }
        Err(e) => AiResponse::error("anthropic", format!("Failed to parse response: {}", e)),
    }
}

// ── Ollama (local) ────────────────────────────────────────────────────────────

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
        Ok(resp) => {
            match resp.into_json::<serde_json::Value>() {
                Ok(json) => {
                    let text = json["response"].as_str().unwrap_or("").to_string();
                    let tokens = json["eval_count"].as_u64().unwrap_or(0);
                    let confidence = adjust_confidence_for_hedging(0.85, &text);
                    AiResponse { text, confidence, tokens_used: tokens, model: model.into(), provider: "ollama".into() }.to_value()
                }
                Err(e) => AiResponse::error("ollama", format!("Parse error: {}", e)),
            }
        }
        Err(e) => AiResponse::error("ollama", format!(
            "Cannot connect to Ollama at {}. Is it running? Start with: ollama serve\nError: {}", base_url, e
        )),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn adjust_confidence_for_hedging(base: f64, text: &str) -> f64 {
    let lower = text.to_lowercase();
    let hedges = ["i'm not sure", "i think", "maybe", "possibly", "i believe",
                  "not certain", "might be", "could be", "i'm unsure", "unclear"];
    let hedge_count = hedges.iter().filter(|h| lower.contains(*h)).count();
    (base - hedge_count as f64 * 0.05).max(0.1).min(1.0)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max]) }
}
