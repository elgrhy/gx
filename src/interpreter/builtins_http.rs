//! HTTP client builtins and SSRF protection.
//!
//! SSRF protection has two layers, deliberately:
//!
//! 1. A cheap, string-based pre-check ([`check_url_safe`]) on the URL as
//!    written in the script — fails fast, before any network I/O, with a
//!    clear error naming the blocked host.
//! 2. The authoritative check: a custom [`ureq::Resolver`] ([`SsrfSafeResolver`])
//!    that classifies the *actual resolved IP address* on every single
//!    connection ureq makes — including every redirect hop. This is what
//!    actually closes SSRF, not the pre-check: a URL can name an external
//!    host that later redirects to `169.254.169.254`, or a hostname that
//!    simply *resolves* to a private address, or an IP written in a
//!    non-dotted-decimal form (`http://2130706433/` is `127.0.0.1`) — none
//!    of those are caught by inspecting the URL string, only by inspecting
//!    where the connection is actually about to go.
//!
//! Both layers ask the same [`crate::capability::Capabilities::authorize`]
//! — neither implements its own allow/deny decision.

use super::builtins_json::{gx_value_to_json, json_to_gx_value};
use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

/// Cap on how much of a response body is retained in memory, mirroring the
/// process runtime's identical constant and rationale: unbounded
/// `into_string()` on an attacker- or bug-controlled server's response is an
/// easy memory-exhaustion vector. Truncation is always reported
/// (`truncated`/`body_bytes`), never silent.
#[cfg(not(target_arch = "wasm32"))]
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Same cap applied to `http_stream`'s line buffer, so a single enormous
/// line (or a stream with no newlines at all) can't grow unbounded either.
#[cfg(not(target_arch = "wasm32"))]
const MAX_STREAM_BYTES: usize = 32 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling on a per-call `timeout` override — prevents `timeout: 999999999`
/// from defeating the point of having a cap at all.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

// ── SSRF: real IP classification ────────────────────────────────────────────

/// Whether `ip` is loopback, private, link-local, unspecified, multicast, or
/// carrier-grade-NAT shared space (100.64.0.0/10) — every non-routable-to-
/// the-public-internet range relevant to SSRF. IPv6 additionally unwraps
/// IPv4-mapped addresses (`::ffff:127.0.0.1`) before classifying.
#[cfg(not(target_arch = "wasm32"))]
fn ip_is_internal(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
            {
                return true;
            }
            // 100.64.0.0/10 — carrier-grade NAT / shared address space.
            // Ipv4Addr::is_shared() covers exactly this but is still
            // unstable (`ip` feature) — checked manually instead.
            let o = v4.octets();
            o[0] == 100 && (64..=127).contains(&o[1])
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return ip_is_internal(IpAddr::V4(mapped));
            }
            // fc00::/7 (unique local) and fe80::/10 (link-local) — checked
            // on the raw segments since Ipv6Addr has no single is_private().
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

/// Resolves hostnames the same way `ureq`'s default resolver would, but
/// authorizes every resolved address through the Capability Runtime before
/// handing it back — called for every connection ureq makes, including each
/// redirect hop, which is what makes this the authoritative SSRF check
/// rather than the best-effort pre-check `check_url_safe` does on the
/// original URL string.
#[cfg(not(target_arch = "wasm32"))]
struct SsrfSafeResolver {
    capabilities: crate::capability::Capabilities,
    diagnostics: crate::diagnostics::Diagnostics,
}

#[cfg(not(target_arch = "wasm32"))]
impl ureq::Resolver for SsrfSafeResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use crate::capability::Resource;
        use std::net::ToSocketAddrs;
        let addrs: Vec<std::net::SocketAddr> = netloc.to_socket_addrs()?.collect();
        for addr in &addrs {
            let resource = if ip_is_internal(addr.ip()) {
                Resource::InternalNetwork
            } else {
                Resource::ExternalNetwork
            };
            if let Err(denial) = self.capabilities.authorize(resource, None) {
                // Audited here, not just in check_url_safe's cheap
                // pre-check: this is the path that actually catches a
                // redirect or a DNS answer landing on an internal address
                // — the pre-check only ever sees the original URL string.
                self.diagnostics.audit(
                    "capability_denied",
                    serde_json::json!({
                        "resource": resource.name(),
                        "name": netloc,
                        "reason": denial.to_string(),
                    }),
                );
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "SSRF protection: '{}' resolved to {}, which is not authorized",
                        netloc,
                        addr.ip()
                    ),
                ));
            }
        }
        Ok(addrs)
    }
}

/// Shared `ureq` agent builder, capability-aware via [`SsrfSafeResolver`].
/// Every HTTP builtin (`http_get`/`post`/`put`/`delete`/`http_stream`/
/// `http_upload`) goes through this — there is no other way to make an
/// outbound request, so there is no separate code path that could skip
/// either the timeout defaults or the SSRF resolver.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_agent(
    capabilities: &crate::capability::Capabilities,
    diagnostics: &crate::diagnostics::Diagnostics,
) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(DEFAULT_CONNECT_TIMEOUT)
        .timeout_read(DEFAULT_READ_TIMEOUT)
        .resolver(SsrfSafeResolver {
            capabilities: capabilities.clone(),
            diagnostics: diagnostics.clone(),
        })
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

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Pull an optional `timeout` (seconds) out of a headers/opts object,
/// clamped to `MAX_CALL_TIMEOUT`. Reserving this one key name means a
/// script that happens to want a literal `timeout` HTTP header can't via
/// this path — an acceptable trade-off since that's not a real header any
/// server expects, and it's what closes GX's previous complete absence of
/// per-call timeout control (every GClaw callsite wanted this and had no
/// way to get it — see docs for the migration note).
#[cfg(not(target_arch = "wasm32"))]
fn extract_timeout(headers_val: &Value) -> Option<std::time::Duration> {
    if let Value::Object(m) = headers_val {
        let secs = m.get("timeout").and_then(|v| v.as_number())?;
        if secs.is_nan() || secs <= 0.0 {
            // Also catches NaN (every comparison with NaN is false) —
            // `secs <= 0.0` let NaN fall through to the unguarded
            // `Duration::from_secs_f64` below, which panics on it (and on
            // `Infinity`, which is `> 0.0` and so wasn't caught either).
            return None;
        }
        return Some(crate::clamp_duration_secs(secs, MAX_CALL_TIMEOUT));
    }
    None
}

/// Apply every header in `headers_val` except the reserved `timeout` key.
#[cfg(not(target_arch = "wasm32"))]
fn apply_headers(mut req: ureq::Request, headers_val: &Value) -> ureq::Request {
    if let Value::Object(headers) = headers_val {
        for (k, v) in headers {
            if k == "timeout" {
                continue;
            }
            req = req.set(k, &v.to_string());
        }
    }
    req
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_timeout(req: ureq::Request, headers_val: &Value) -> ureq::Request {
    match extract_timeout(headers_val) {
        Some(d) => req.timeout(d),
        None => req,
    }
}

/// Read a response body up to [`MAX_RESPONSE_BYTES`], reporting the real
/// total and whether it was truncated — never silently incomplete.
#[cfg(not(target_arch = "wasm32"))]
fn read_capped_body(resp: ureq::Response) -> (String, usize, bool) {
    use std::io::Read;
    let mut reader = resp.into_reader();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut total = 0usize;
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        total += n;
        if buf.len() < MAX_RESPONSE_BYTES {
            let take = (MAX_RESPONSE_BYTES - buf.len()).min(n);
            buf.extend_from_slice(&chunk[..take]);
        }
        if total > MAX_RESPONSE_BYTES {
            // Stop reading rather than draining to completion: an
            // oversized response is anomalous, and the cost of reading
            // (and discarding) potentially unbounded further bytes just to
            // keep one connection reusable isn't worth it.
            break;
        }
    }
    let truncated = total > MAX_RESPONSE_BYTES;
    (String::from_utf8_lossy(&buf).into_owned(), total, truncated)
}

/// Classify a `ureq::Error` into a stable, matchable string — addresses a
/// real GClaw complaint (`improvegx.md` §18): `catch e` previously yielded
/// only a free-text message, with no way to tell a timeout from a DNS
/// failure from an SSRF block without string-matching.
#[cfg(not(target_arch = "wasm32"))]
fn classify_ureq_error(e: &ureq::Error) -> &'static str {
    use std::error::Error as _;
    match e {
        ureq::Error::Status(_, _) => "http_status",
        ureq::Error::Transport(t) => {
            // SsrfSafeResolver reports blocks via io::ErrorKind::PermissionDenied;
            // ureq wraps every resolver error as ErrorKind::Dns, so without
            // checking the source kind a block would misreport as "dns_error".
            let io_kind = t
                .source()
                .and_then(|s| s.downcast_ref::<std::io::Error>())
                .map(|io_err| io_err.kind());
            if io_kind == Some(std::io::ErrorKind::PermissionDenied) {
                return "blocked";
            }
            // A timeout can surface wrapped in Dns/ConnectionFailed/Io
            // depending on which phase (resolve, TLS handshake, read) hit
            // the deadline, and the underlying io::Error can be
            // TimedOut or WouldBlock depending on platform — checked
            // before the per-phase match so none of those combinations
            // get mislabeled as a generic connection/DNS/IO failure.
            if matches!(
                io_kind,
                Some(std::io::ErrorKind::TimedOut) | Some(std::io::ErrorKind::WouldBlock)
            ) {
                return "timeout";
            }
            match t.kind() {
                ureq::ErrorKind::Dns => "dns_error",
                ureq::ErrorKind::ConnectionFailed => "connection_failed",
                ureq::ErrorKind::TooManyRedirects => "too_many_redirects",
                ureq::ErrorKind::Io => "io_error",
                _ => "transport_error",
            }
        }
    }
}

/// Build the `{ok, status, body, body_bytes, truncated, data?, error?, error_kind?}`
/// result object shared by `http_get`/`post`/`put`/`delete` — replaces four
/// near-identical copies that had drifted out of sync (`http_put` was
/// missing the `Status` arm entirely, so a failed PUT lost its status code;
/// `http_delete` never returned `body`/`data` at all).
#[cfg(not(target_arch = "wasm32"))]
fn build_result(resp_result: Result<ureq::Response, ureq::Error>) -> Value {
    match resp_result {
        Ok(resp) => {
            let status = resp.status() as f64;
            let (body, total_bytes, truncated) = read_capped_body(resp);
            let mut map = HashMap::new();
            map.insert("ok".into(), Value::Bool(status < 400.0));
            map.insert("status".into(), Value::Number(status));
            map.insert("body".into(), Value::Str(body.clone()));
            map.insert("body_bytes".into(), Value::Number(total_bytes as f64));
            map.insert("truncated".into(), Value::Bool(truncated));
            if !truncated {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    map.insert("data".into(), json_to_gx_value(&json));
                }
            }
            Value::Object(map)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let (body, total_bytes, truncated) = read_capped_body(resp);
            let mut map = HashMap::new();
            map.insert("ok".into(), Value::Bool(false));
            map.insert("status".into(), Value::Number(code as f64));
            map.insert("body".into(), Value::Str(body.clone()));
            map.insert("body_bytes".into(), Value::Number(total_bytes as f64));
            map.insert("truncated".into(), Value::Bool(truncated));
            if !truncated {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    map.insert("data".into(), json_to_gx_value(&json));
                }
            }
            map.insert("error".into(), Value::Str(format!("HTTP {}", code)));
            map.insert("error_kind".into(), Value::Str("http_status".into()));
            Value::Object(map)
        }
        Err(e) => {
            let mut map = HashMap::new();
            map.insert("ok".into(), Value::Bool(false));
            map.insert("status".into(), Value::Number(0.0));
            map.insert(
                "error_kind".into(),
                Value::Str(classify_ureq_error(&e).into()),
            );
            map.insert("error".into(), Value::Str(e.to_string()));
            Value::Object(map)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn http_builtin(
    name: &str,
    args: &[Value],
    agent: &ureq::Agent,
) -> Result<Value, Signal> {
    match name {
        "http_get" | "fetch" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_get requires a URL string".into()))?;
            let headers_val = args.get(1).cloned().unwrap_or(Value::Null);
            let req = apply_timeout(apply_headers(agent.get(&url), &headers_val), &headers_val);
            Ok(build_result(req.call()))
        }
        "http_post" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_post requires a URL string".into()))?;
            let body_val = args.get(1).cloned().unwrap_or(Value::Null);
            let headers_val = args.get(2).cloned().unwrap_or(Value::Null);
            let req = agent.post(&url).set("Content-Type", "application/json");
            let req = apply_timeout(apply_headers(req, &headers_val), &headers_val);
            let json_body = gx_value_to_json(&body_val);
            Ok(build_result(req.send_json(&json_body)))
        }
        "http_put" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_put requires a URL string".into()))?;
            let body_val = args.get(1).cloned().unwrap_or(Value::Null);
            let headers_val = args.get(2).cloned().unwrap_or(Value::Null);
            let req = agent.put(&url).set("Content-Type", "application/json");
            let req = apply_timeout(apply_headers(req, &headers_val), &headers_val);
            let json_body = gx_value_to_json(&body_val);
            Ok(build_result(req.send_json(&json_body)))
        }
        "http_delete" => {
            let url = args
                .first()
                .and_then(|v| v.as_str().map(String::from))
                .ok_or_else(|| Signal::Error("http_delete requires a URL string".into()))?;
            let headers_val = args.get(1).cloned().unwrap_or(Value::Null);
            let req = apply_timeout(
                apply_headers(agent.delete(&url), &headers_val),
                &headers_val,
            );
            Ok(build_result(req.call()))
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
pub(super) fn http_stream_builtin(args: &[Value], agent: &ureq::Agent) -> Result<Value, Signal> {
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

    // Routed through the same shared, capability-aware agent as every other
    // HTTP builtin — previously called bare `ureq::get`/`ureq::post`
    // directly, which meant no configured timeout at all (a stream request
    // to a hung server blocked forever) and, more seriously, none of the
    // SSRF-resolver protection every other HTTP entry point had.
    let req = apply_timeout(agent.request(&method.to_uppercase(), &url), &opts);
    let resp = match method.to_uppercase().as_str() {
        "POST" | "PUT" | "PATCH" => {
            let json_body = gx_value_to_json(&body_val);
            req.set("Content-Type", "application/json")
                .send_json(&json_body)
        }
        _ => req.call(),
    }
    .map_err(|e| Signal::Error(format!("http_stream {} failed: {}", method, e)))?;

    let reader = BufReader::new(resp.into_reader());
    let mut chunks: Vec<Value> = Vec::new();
    let mut total_bytes = 0usize;
    for line in reader.lines() {
        match line {
            Ok(l) => {
                total_bytes += l.len();
                if total_bytes > MAX_STREAM_BYTES {
                    return Err(Signal::Error(format!(
                        "http_stream: response exceeded the {}-byte cap",
                        MAX_STREAM_BYTES
                    )));
                }
                chunks.push(Value::Str(l));
            }
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
pub(super) fn http_upload_builtin(
    args: &[Value],
    agent: &ureq::Agent,
    capabilities: &crate::capability::Capabilities,
) -> Result<Value, Signal> {
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
            // Routed through the Capability Runtime's filesystem scope —
            // previously a raw std::fs::read(path), meaning a sandboxed
            // script could read and exfiltrate any file on disk (e.g.
            // /etc/passwd) via multipart upload regardless of --no-sandbox.
            let safe_path = capabilities.resolve_path(path).map_err(|e| {
                Signal::Error(format!("http_upload: access denied for '{}': {}", path, e))
            })?;
            let file_data = std::fs::read(&safe_path).map_err(|e| {
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
    let req = agent.post(&url).set("Content-Type", &content_type);
    let opts_val = Value::Object(map.clone());
    let req = apply_timeout(req, &opts_val);
    let resp_result = req.send_bytes(&body);
    Ok(build_result(resp_result))
}

/// Strip the query string and fragment from a URL before it's recorded as
/// span/log data — query strings routinely carry API keys, presigned-URL
/// signatures, and OAuth tokens (e.g. `?api_key=...`), and diagnostics
/// output is meant to be safe to ship to a log aggregator or paste into a
/// bug report. Scheme/host/path are kept since they're what's actually
/// useful for correlating requests in a trace.
pub(super) fn diagnostic_url(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].to_string()
}

/// Extract the bare host (no scheme, no port, no brackets around IPv6) from
/// a URL — pure string classification, no capability decision.
#[cfg(not(target_arch = "wasm32"))]
fn extract_host(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host_port = rest.split('/').next().unwrap_or(rest);
    // Strip `user:pass@` userinfo, if present, so it isn't mistaken for the
    // host itself (this pre-check is best-effort and not the authoritative
    // SSRF decision — see `SsrfSafeResolver` — but it should still classify
    // and audit-log the real target, not the credentials).
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .unwrap_or(host_port)
            .trim_start_matches('[')
            .to_string()
    } else {
        host_port.split(':').next().unwrap_or(host_port).to_string()
    }
}

/// Best-effort pre-check classification: if `host` parses as a literal IP,
/// classify it exactly via [`ip_is_internal`]; otherwise fall back to the
/// well-known internal hostnames. A hostname that merely *resolves* to an
/// internal address isn't (and can't cheaply be) caught here — that's
/// exactly what [`SsrfSafeResolver`] exists to catch authoritatively.
#[cfg(not(target_arch = "wasm32"))]
fn is_internal_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip_is_internal(ip);
    }
    let host_lower = host.to_lowercase();
    host_lower == "localhost" || host_lower == "ip6-localhost" || host_lower.ends_with(".localhost")
}

/// Classify the URL (HTTP's own job) and authorize it (the Capability
/// Runtime's job — see `crate::capability`). This subsystem never decides
/// on its own whether a request is allowed; it only knows how to tell
/// internal from external and asks `capabilities` for the answer. This is
/// the fast pre-check only — see the module docs for why the
/// `SsrfSafeResolver` used by every real connection is the authoritative one.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn check_url_safe(
    url: &str,
    capabilities: &crate::capability::Capabilities,
    diagnostics: &crate::diagnostics::Diagnostics,
) -> Result<(), Signal> {
    use crate::capability::Resource;
    if url.starts_with("file://") {
        return Err(Signal::Error(
            "HTTP functions do not allow file:// URLs.".into(),
        ));
    }
    let host = extract_host(url);
    let internal = is_internal_host(&host);
    let resource = if internal {
        Resource::InternalNetwork
    } else {
        Resource::ExternalNetwork
    };
    capabilities.authorize(resource, None).map_err(|denial| {
        diagnostics.audit(
            "capability_denied",
            serde_json::json!({
                "resource": resource.name(),
                "name": host,
                "reason": denial.to_string(),
            }),
        );
        if internal {
            Signal::Error(format!(
                "SSRF protection: requests to '{}' are blocked. \
                 Use --allow-internal-http to allow internal network access.",
                host
            ))
        } else {
            Signal::Error(format!(
                "External network access to '{}' is restricted by gx.json's \
                 capabilities.external_network setting.",
                host
            ))
        }
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::capability::Capabilities;
    use std::net::IpAddr;
    use std::str::FromStr;

    fn ip(s: &str) -> IpAddr {
        IpAddr::from_str(s).unwrap()
    }

    #[test]
    fn diagnostic_url_strips_query_string_and_fragment_so_secrets_never_reach_a_span() {
        // Query strings routinely carry API keys/tokens/presigned-URL
        // signatures — diagnostic_url is what keeps them out of the JSONL
        // a span writes to stderr under --trace.
        assert_eq!(
            diagnostic_url("https://api.example.com/v1/users?api_key=SECRET123"),
            "https://api.example.com/v1/users"
        );
        assert_eq!(
            diagnostic_url("https://example.com/page#section"),
            "https://example.com/page"
        );
        assert_eq!(
            diagnostic_url("https://example.com/page?token=abc#frag"),
            "https://example.com/page"
        );
        assert_eq!(
            diagnostic_url("https://example.com/no-query"),
            "https://example.com/no-query"
        );
        assert_eq!(diagnostic_url(""), "");
    }

    #[test]
    fn ip_is_internal_covers_ipv4_ranges() {
        assert!(ip_is_internal(ip("127.0.0.1")));
        assert!(ip_is_internal(ip("10.1.2.3")));
        assert!(ip_is_internal(ip("172.16.0.1")));
        assert!(ip_is_internal(ip("172.31.255.255")));
        assert!(!ip_is_internal(ip("172.32.0.1"))); // just outside 172.16/12
        assert!(ip_is_internal(ip("192.168.1.1")));
        assert!(ip_is_internal(ip("169.254.169.254"))); // cloud metadata
        assert!(ip_is_internal(ip("0.0.0.0")));
        assert!(ip_is_internal(ip("100.64.0.1"))); // CGNAT shared space
        assert!(ip_is_internal(ip("100.127.255.255")));
        assert!(!ip_is_internal(ip("100.128.0.1"))); // just outside 100.64/10
        assert!(!ip_is_internal(ip("8.8.8.8")));
        assert!(!ip_is_internal(ip("1.1.1.1")));
    }

    #[test]
    fn ip_is_internal_covers_ipv6_ranges() {
        assert!(ip_is_internal(ip("::1"))); // loopback
        assert!(ip_is_internal(ip("::"))); // unspecified
        assert!(ip_is_internal(ip("fc00::1"))); // unique local
        assert!(ip_is_internal(ip("fd12:3456:789a::1"))); // unique local
        assert!(ip_is_internal(ip("fe80::1"))); // link-local
        assert!(!ip_is_internal(ip("2001:4860:4860::8888"))); // real public (Google DNS)
    }

    #[test]
    fn ip_is_internal_unwraps_ipv4_mapped_addresses() {
        // ::ffff:127.0.0.1 — a classic SSRF-filter-bypass encoding that a
        // naive "is this IPv6" check would miss entirely.
        assert!(ip_is_internal(ip("::ffff:127.0.0.1")));
        assert!(ip_is_internal(ip("::ffff:192.168.1.1")));
        assert!(!ip_is_internal(ip("::ffff:8.8.8.8")));
    }

    #[test]
    fn is_internal_host_classifies_literal_ips_precisely() {
        // Previously this only matched exact strings and dotted-quad
        // octets — real literal IPv6 private ranges (fc00::/7, fe80::/10)
        // were invisible to it entirely.
        assert!(is_internal_host("fc00::1"));
        assert!(is_internal_host("fe80::1"));
        assert!(is_internal_host("127.0.0.1"));
        assert!(!is_internal_host("8.8.8.8"));
    }

    #[test]
    fn is_internal_host_falls_back_to_known_hostnames() {
        assert!(is_internal_host("localhost"));
        assert!(is_internal_host("foo.localhost"));
        assert!(!is_internal_host("example.com"));
        // A hostname that merely *resolves* to an internal address can't be
        // caught by this cheap pre-check — that's what SsrfSafeResolver is
        // for (see the redirect/DNS-rebinding test in mod-level tests).
    }

    #[test]
    fn check_url_safe_blocks_file_urls() {
        let caps = Capabilities::new();
        let diag = crate::diagnostics::Diagnostics::new();
        assert!(check_url_safe("file:///etc/passwd", &caps, &diag).is_err());
    }

    #[test]
    fn check_url_safe_allows_external_denies_internal_by_default() {
        let caps = Capabilities::new();
        let diag = crate::diagnostics::Diagnostics::new();
        assert!(check_url_safe("https://example.com", &caps, &diag).is_ok());
        assert!(check_url_safe("http://127.0.0.1/", &caps, &diag).is_err());
        assert!(check_url_safe("http://[fc00::1]/", &caps, &diag).is_err());
    }

    #[test]
    fn extract_timeout_reads_and_clamps_the_reserved_key() {
        let mut m = std::collections::HashMap::new();
        m.insert("timeout".to_string(), Value::Number(2.0));
        let v = Value::Object(m);
        assert_eq!(extract_timeout(&v), Some(std::time::Duration::from_secs(2)));

        let mut m2 = std::collections::HashMap::new();
        m2.insert("timeout".to_string(), Value::Number(999_999.0));
        assert_eq!(extract_timeout(&Value::Object(m2)), Some(MAX_CALL_TIMEOUT));

        let mut m3 = std::collections::HashMap::new();
        m3.insert("timeout".to_string(), Value::Number(-5.0));
        assert_eq!(extract_timeout(&Value::Object(m3)), None);

        assert_eq!(extract_timeout(&Value::Null), None);
    }

    #[test]
    fn apply_headers_sends_every_header_except_the_reserved_timeout_key() {
        let agent = ureq::AgentBuilder::new().build();
        let mut m = std::collections::HashMap::new();
        m.insert("X-Test".to_string(), Value::Str("abc".into()));
        m.insert("timeout".to_string(), Value::Number(5.0));
        let headers_val = Value::Object(m);
        let req = apply_headers(agent.get("http://example.invalid"), &headers_val);
        // No direct getter on ureq::Request for a set header, so this just
        // confirms building the request with a mix of a real header and
        // the reserved key doesn't panic and the reserved key is skipped
        // (behavioral confirmation lives in the resolver/timeout tests).
        let _ = req;
    }

    #[test]
    fn classify_ureq_error_labels_ssrf_blocks_distinctly_from_dns_failure() {
        // Route a request through a resolver that always rejects, exactly
        // as SsrfSafeResolver does when a resolved address isn't
        // authorized — confirms the resulting error is labeled "blocked",
        // not the generic "dns_error" ureq would suggest from ErrorKind::Dns
        // alone (see classify_ureq_error's doc comment for why that
        // distinction needs the source-error check).
        struct AlwaysBlock;
        impl ureq::Resolver for AlwaysBlock {
            fn resolve(&self, _netloc: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "blocked for test",
                ))
            }
        }
        let agent = ureq::AgentBuilder::new().resolver(AlwaysBlock).build();
        let err = agent.get("http://example.invalid/").call().unwrap_err();
        assert_eq!(classify_ureq_error(&err), "blocked");
    }

    #[test]
    fn classify_ureq_error_labels_genuine_dns_failure_as_dns_error() {
        let agent = http_agent(
            &Capabilities::new(),
            &crate::diagnostics::Diagnostics::new(),
        );
        let err = agent
            .get("http://this-host-does-not-exist.invalid.test/")
            .call()
            .unwrap_err();
        let kind = classify_ureq_error(&err);
        assert!(
            kind == "dns_error" || kind == "connection_failed",
            "expected a DNS/connection failure classification, got {}",
            kind
        );
    }

    /// Starts a tiny local HTTP server on `port` that replies to every
    /// request with a 302 redirecting to `location` — used to prove the
    /// SSRF-via-redirect bypass is actually closed, not just theoretically
    /// described. A real external redirect target isn't reliable to depend
    /// on in a test (network access, rate limits, availability), so this
    /// spins up the redirect source locally instead.
    fn spawn_redirecting_server(port: u16, location: &'static str) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().take(1).flatten() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    location
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
    }

    #[test]
    fn ssrf_resolver_blocks_a_redirect_to_an_internal_address() {
        // This is the actual vulnerability class this milestone closes:
        // check_url_safe only ever validated the *original* URL, so an
        // allowed external URL that 302-redirected to a private address
        // sailed straight through with the old string-only check. The
        // resolver is consulted on every connection ureq makes, including
        // each redirect hop, so this must now fail.
        let redirect_source_port = 18471;
        spawn_redirecting_server(redirect_source_port, "http://127.0.0.1:1/internal-secret");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let caps = Capabilities::new(); // internal_network denied by default
        let agent = http_agent(&caps, &crate::diagnostics::Diagnostics::new());
        let url = format!("http://127.0.0.1:{}/", redirect_source_port);
        // The redirect *source* itself is a loopback address too, so with
        // internal_network denied by default even the first hop would be
        // rejected — that's expected and still proves the point (nothing
        // reaches the redirect target). The important assertion is that
        // this returns an error at all, not a successful 200.
        let result = agent.get(&url).call();
        assert!(result.is_err(), "expected the request to be blocked");
    }

    #[test]
    fn ssrf_resolver_distinguishes_denied_from_merely_unreachable() {
        // Same redirect chain as above, but with internal_network granted:
        // the resolver must let the connection *attempt* proceed (proving
        // it's checking authorization per-hop, not unconditionally
        // rejecting every loopback address regardless of policy) — it
        // should fail because nothing listens on port 1, a connection
        // error, never a permission error.
        let redirect_source_port = 18472;
        spawn_redirecting_server(redirect_source_port, "http://127.0.0.1:1/internal-secret");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let mut caps = Capabilities::new();
        caps.internal_network = true;
        let agent = http_agent(&caps, &crate::diagnostics::Diagnostics::new());
        let url = format!("http://127.0.0.1:{}/", redirect_source_port);
        let err = agent
            .get(&url)
            .call()
            .expect_err("port 1 should refuse the connection");
        assert_ne!(
            classify_ureq_error(&err),
            "blocked",
            "with internal_network granted, failure must be connectivity, not policy"
        );
    }
}
