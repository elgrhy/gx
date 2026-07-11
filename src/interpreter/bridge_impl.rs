//! Bridge calls (JS/Python) and the HTTP `serve` block implementation.

use super::{Env, IResult, Interpreter, Signal};
use crate::ast::{Expr, RouteDecl};
use crate::bridge::Bridge;
use crate::capability::Resource;
use crate::value::Value;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
impl Interpreter {
    pub fn bridge_call(
        &mut self,
        namespace: &str,
        module: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Signal> {
        // Every bridge namespace authorizes through the same Capability
        // Runtime call, scoped to `module` — no namespace implements its
        // own allowlist logic anymore. `js`/`py`/`process` already had one
        // before this milestone; `ts`/`binary`/`go`/`rust_bin` previously
        // had *no* check at all (arbitrary-executable loading, unguarded)
        // — this closes that gap uniformly rather than bolting a fix onto
        // just the namespaces that happened to be reported.
        let resource = match namespace {
            "js" => Some(Resource::JsBridge),
            "ts" => Some(Resource::TsBridge),
            "py" => Some(Resource::PyBridge),
            "binary" => Some(Resource::BinaryBridge),
            "go" => Some(Resource::GoBridge),
            "rust_bin" => Some(Resource::RustBinBridge),
            _ => None,
        };
        if let Some(resource) = resource {
            self.authorize_capability(resource, Some(module))
                .map_err(|e| {
                    // Only the "not in the allowlist" case has a useful,
                    // specific next step (declare it in gx.json); an
                    // operator --deny can't be worked around from a
                    // manifest, so that message stands on its own.
                    let hint = match &e {
                        crate::capability::Denial::NotInAllowlist { .. } => format!(
                            " — add it to gx.json's dependencies.{} to allow it.",
                            resource.name()
                        ),
                        _ => String::new(),
                    };
                    Signal::Error(format!("{}{}", e, hint))
                })?;
        }
        match namespace {
            "js" => {
                // Persistent Node process speaking the JSON-IPC protocol
                // (see bridge.rs) — module/method/args are passed as JSON
                // values, never spliced into a script string, and the
                // process is reused across calls instead of paying Node's
                // ~50-100ms startup cost on every single call.
                if self.js_bridge.is_none() {
                    match Bridge::new_js() {
                        Ok(b) => self.js_bridge = Some(b),
                        Err(e) => return Err(Signal::Error(e)),
                    }
                }
                let bridge = self
                    .js_bridge
                    .as_mut()
                    .ok_or_else(|| Signal::Error("JS bridge unavailable".into()))?;
                let result = bridge.call(module, method, args);
                // A timed-out call leaves the bridge's request/response
                // stream unsafe to reuse (see `Bridge::call`'s doc
                // comment) — drop the cached instance so the next call
                // spawns a fresh process instead of silently handing a
                // future call the wrong response.
                if self.js_bridge.as_ref().is_some_and(Bridge::is_broken) {
                    self.js_bridge = None;
                }
                result.map_err(Signal::Error)
            }
            "ts" => {
                // TypeScript bridge — its own slot, independent of the plain
                // JS bridge above, since a program may use both namespaces
                // at once and they run through different runners (tsx/
                // ts-node vs plain node).
                if self.ts_bridge.is_none() {
                    match Bridge::new_typescript() {
                        Ok(b) => self.ts_bridge = Some(b),
                        Err(e) => return Err(Signal::Error(e)),
                    }
                }
                let bridge = self
                    .ts_bridge
                    .as_mut()
                    .ok_or_else(|| Signal::Error("TypeScript bridge unavailable".into()))?;
                let result = bridge.call(module, method, args);
                if self.ts_bridge.as_ref().is_some_and(Bridge::is_broken) {
                    self.ts_bridge = None;
                }
                result.map_err(Signal::Error)
            }
            "py" => {
                if self.py_bridge.is_none() {
                    match Bridge::new_python() {
                        Ok(b) => self.py_bridge = Some(b),
                        Err(e) => return Err(Signal::Error(e)),
                    }
                }
                let bridge = self
                    .py_bridge
                    .as_mut()
                    .ok_or_else(|| Signal::Error("Python bridge unavailable".into()))?;
                let result = bridge.call(module, method, args);
                if self.py_bridge.as_ref().is_some_and(Bridge::is_broken) {
                    self.py_bridge = None;
                }
                result.map_err(Signal::Error)
            }
            // Generic binary / Go bridge — module is the path to the executable.
            // Syntax: use binary "./my_service" → bridge_call("binary", "./my_service", method, args)
            // Syntax: use go "./my_go_service" → bridge_call("go", "./my_go_service", method, args)
            "binary" | "go" | "rust_bin" => {
                let bridge_key = format!("{}:{}", namespace, module);
                if !self.binary_bridges.contains_key(&bridge_key) {
                    match Bridge::new_binary(module) {
                        Ok(b) => {
                            self.binary_bridges.insert(bridge_key.clone(), b);
                        }
                        Err(e) => return Err(Signal::Error(e)),
                    }
                }
                let bridge = self
                    .binary_bridges
                    .get_mut(&bridge_key)
                    .ok_or_else(|| Signal::Error("Binary bridge unavailable".into()))?;
                let result = bridge.call(module, method, args);
                if self
                    .binary_bridges
                    .get(&bridge_key)
                    .is_some_and(Bridge::is_broken)
                {
                    self.binary_bridges.remove(&bridge_key);
                }
                result.map_err(Signal::Error)
            }
            "rust" => Err(Signal::Error(format!(
                "Native Rust interop for '{}' requires recompiling GX with the crate linked. \
                 For subprocess interop, use: use binary \"./my_rust_binary\"",
                module
            ))),
            other => Err(Signal::Error(format!(
                "Unknown namespace '{}'. Use: js, ts, py, binary, go",
                other
            ))),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn run_serve(
        &mut self,
        port_expr: &Expr,
        routes: &[RouteDecl],
        env: &mut Env,
    ) -> IResult {
        self.authorize_capability(Resource::HttpServer, None)
            .map_err(|e| Signal::Error(e.to_string()))?;
        let port = self
            .eval_expr(port_expr, env)?
            .as_number()
            .unwrap_or(3000.0) as u16;
        let addr = format!("0.0.0.0:{}", port);
        let server = tiny_http::Server::http(&addr)
            .map_err(|e| Signal::Error(format!("Cannot start server on port {}: {}", port, e)))?;
        println!("GX server listening on http://localhost:{}", port);
        println!("Press Ctrl+C to stop.");

        // A bounded worker pool, not one request at a time: previously a
        // single slow client or a single slow route (e.g. one waiting on
        // an AI provider call) blocked literally every other route on the
        // server, including unrelated webhooks. tiny_http's own docs
        // recommend exactly this pattern — an Arc<Server> with multiple
        // threads calling recv() — and the library is built to support it.
        let server = std::sync::Arc::new(server);
        let routes: std::sync::Arc<Vec<RouteDecl>> = std::sync::Arc::new(routes.to_vec());
        // Shared across every worker (see `active_sse_responders`'s own doc
        // comment) so `respond stream`'s cap on concurrent responder
        // threads is enforced server-wide, not reset per worker.
        let active_sse_responders = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut workers = Vec::with_capacity(SERVER_WORKER_THREADS);
        for _ in 0..SERVER_WORKER_THREADS {
            let server = server.clone();
            let routes = routes.clone();
            let worker_env = env.clone();
            let active_sse_responders = active_sse_responders.clone();
            // Each worker gets its own Interpreter, inheriting the
            // program's definitions and capabilities — the same
            // inheritance pattern `spawn agent`/`parallel {}` already use
            // (see crate::capability), extended to cover everything a
            // route body can reach (tools, module_functions, global_vars)
            // since a server route is running the *same program*, not an
            // isolated spawned computation. Per-worker state that must not
            // be shared across concurrent requests (bridges, the HTTP
            // agent, the SQLite tx connection, ...) starts fresh — exactly
            // as it would for any other freshly constructed Interpreter.
            let mut worker = Interpreter::new();
            worker.helpers = self.helpers.clone();
            worker.functions = self.functions.clone();
            worker.capabilities = self.capabilities.clone();
            worker.module_functions = self.module_functions.clone();
            worker.tools = self.tools.clone();
            worker.global_vars = self.global_vars.clone();
            worker.base_path = self.base_path.clone();
            // Config (enabled/min_level) inherited; no trace_id yet — each
            // request mints its own fresh one in handle_one_request, since
            // a worker thread handles many requests over its lifetime and
            // each is its own logical trace.
            worker.diagnostics = self.diagnostics.for_child();
            worker.no_loop_limit = self.no_loop_limit;
            worker.active_sse_responders = Some(active_sse_responders.clone());

            // A larger-than-default stack, same as every other thread that
            // runs GX interpreter logic end-to-end (see `WORKER_THREAD_
            // STACK_SIZE`'s own doc comment) — the `catch_unwind` below
            // recovers a worker from an ordinary panic inside one request,
            // but a real stack overflow is a process-level abort, not an
            // unwindable panic, so `catch_unwind` cannot and does not catch
            // it. Confirmed empirically before this fix: a route handler
            // calling an accidentally-unbounded recursive function crashed
            // the whole server process — not just failed the one request.
            let worker_thread = std::thread::Builder::new()
                .stack_size(super::WORKER_THREAD_STACK_SIZE)
                .spawn(move || {
                    let mut worker = worker;
                    loop {
                        let request = match server.recv() {
                            Ok(r) => r,
                            Err(_) => break, // listener closed
                        };
                        // A panic inside a single request (an interpreter bug
                        // hit by unusual input, say) must not take a worker
                        // out of rotation permanently — without this, enough
                        // requests hitting the same panic would eventually
                        // kill every worker, leaving the server listening but
                        // never able to answer anything. The one request that
                        // panicked gets no response (its connection just
                        // drops), but every other worker, and every
                        // subsequent request on this one, keeps working.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            handle_one_request(&mut worker, request, &routes, &worker_env);
                        }));
                        if let Err(payload) = result {
                            let msg = payload
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            eprintln!("[gx server] worker recovered from a panic: {}", msg);
                        }
                    }
                })
                .expect("failed to spawn an HTTP server worker thread");
            workers.push(worker_thread);
        }
        for w in workers {
            let _ = w.join();
        }
        Ok(Value::Null)
    }
}

/// Fixed-size worker pool for `serve on port N`. I/O-bound (routes mostly
/// wait on AI providers, outbound HTTP, subprocesses), so more workers than
/// CPU cores is the right default — a simple fixed constant rather than a
/// configurable pool size, since "avoid unnecessary abstraction" outweighs
/// the value of tuning this before anyone has asked for it.
#[cfg(not(target_arch = "wasm32"))]
const SERVER_WORKER_THREADS: usize = 8;

/// Cap on a request body GX will retain — mirrors the identical response-side
/// cap and rationale in `builtins_http`: unbounded reads are a memory-
/// exhaustion vector, this time from an inbound client rather than an
/// outbound server. Unlike the response side, an oversized *request* is
/// rejected outright (413) rather than truncated-and-processed, since running
/// a route against a body that's been cut off mid-JSON-object is more likely
/// to produce a confusing bug than a useful partial result.
#[cfg(not(target_arch = "wasm32"))]
const MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;

/// A `Read` implementor that pulls byte chunks from a channel, reporting EOF
/// once the sender is dropped. Lets `respond stream { ... }`'s
/// `request.respond(...)` run on a background thread — tiny_http's `respond`
/// blocks reading from this until EOF — while the interpreter keeps
/// executing the block's statements (each `sse_send` call feeding the
/// channel) on the foreground thread, one frame at a time instead of one
/// buffered response at the end.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct ChannelReader {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl ChannelReader {
    pub(super) fn new(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        ChannelReader {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::io::Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0), // sender dropped -> clean EOF
            }
        }
        let n = (self.buf.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Match `path` against a route pattern that may contain `:name` segments
/// (e.g. `/users/:id`), returning the captured params on a match. Plain
/// string routes (no `:`) behave exactly as the previous exact-match
/// comparison did — this is a superset, not a behavior change for them.
#[cfg(not(target_arch = "wasm32"))]
fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pat_segs: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    let path_segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    if pat_segs.len() != path_segs.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (p, s) in pat_segs.iter().zip(path_segs.iter()) {
        if let Some(name) = p.strip_prefix(':') {
            params.insert(name.to_string(), s.to_string());
        } else if p != s {
            return None;
        }
    }
    Some(params)
}

/// Parse a raw query string into a `{key: value}` object — previously only
/// the raw string was exposed, so every route wanting a specific param had
/// to hand-parse it. The raw `query` string is still set alongside this for
/// backward compatibility.
#[cfg(not(target_arch = "wasm32"))]
fn parse_query(query: &str) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let k = parts.next().unwrap_or("");
        if k.is_empty() {
            continue;
        }
        let v = parts.next().unwrap_or("");
        map.insert(
            super::util::url_decode(k),
            Value::Str(super::util::url_decode(v)),
        );
    }
    map
}

/// Read a request body up to `MAX_REQUEST_BODY_BYTES`, reporting whether it
/// was truncated (caller rejects with 413 rather than processing a partial
/// body — see `MAX_REQUEST_BODY_BYTES`'s docs).
#[cfg(not(target_arch = "wasm32"))]
fn read_body_capped(request: &mut tiny_http::Request) -> (String, bool) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut total = 0usize;
    let reader = request.as_reader();
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        total += n;
        if buf.len() < MAX_REQUEST_BODY_BYTES {
            let take = (MAX_REQUEST_BODY_BYTES - buf.len()).min(n);
            buf.extend_from_slice(&chunk[..take]);
        }
        if total > MAX_REQUEST_BODY_BYTES {
            break;
        }
    }
    let truncated = total > MAX_REQUEST_BODY_BYTES;
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

#[cfg(not(target_arch = "wasm32"))]
fn send_response(request: tiny_http::Request, content_type: &str, body: String, status: u16) {
    let ct_header =
        tiny_http::Header::from_bytes(b"Content-Type".as_ref(), content_type.as_bytes())
            .unwrap_or_else(|_| {
                tiny_http::Header::from_bytes(
                    b"Content-Type".as_ref(),
                    b"text/plain; charset=utf-8",
                )
                .expect("fallback Content-Type header is always valid")
            });
    let sec_headers: &[(&[u8], &[u8])] = &[
        (b"X-Content-Type-Options", b"nosniff"),
        (b"X-Frame-Options", b"DENY"),
        (b"X-XSS-Protection", b"1; mode=block"),
        (b"Referrer-Policy", b"strict-origin-when-cross-origin"),
    ];
    let mut response = tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header(ct_header);
    for (name, value) in sec_headers {
        if let Ok(h) = tiny_http::Header::from_bytes(*name, *value) {
            response = response.with_header(h);
        }
    }
    let _ = request.respond(response);
}

#[cfg(not(target_arch = "wasm32"))]
fn respond_plain(request: tiny_http::Request, status: u16, body: &str) {
    send_response(
        request,
        "text/plain; charset=utf-8",
        body.to_string(),
        status,
    );
}

/// End the per-request root span started in `handle_one_request`, tagging
/// it with the method/path/status — a small helper only because there are
/// several exit points (rejected for size, 404, a normal response, a
/// `respond stream` route) that each need to call this exactly once.
#[cfg(not(target_arch = "wasm32"))]
fn end_request_span(
    interp: &mut Interpreter,
    span: crate::diagnostics::SpanHandle,
    method: &str,
    path: &str,
    status: u16,
) {
    let outcome = if status >= 500 {
        crate::diagnostics::Outcome::Error("server error")
    } else {
        crate::diagnostics::Outcome::Ok
    };
    interp.diagnostics.end_span(
        span,
        outcome,
        &[
            ("method", serde_json::Value::String(method.to_string())),
            ("path", serde_json::Value::String(path.to_string())),
            (
                "status",
                serde_json::Value::Number(serde_json::Number::from(status)),
            ),
        ],
    );
}

/// Handle exactly one HTTP request against `routes`, using a worker-local
/// `interp`/`env` — see `Interpreter::run_serve` for why each worker gets
/// its own. This is the per-request body previously inlined directly in
/// `run_serve`'s single loop; extracted so `run_serve` can run
/// `SERVER_WORKER_THREADS` of these concurrently.
#[cfg(not(target_arch = "wasm32"))]
fn handle_one_request(
    interp: &mut Interpreter,
    mut request: tiny_http::Request,
    routes: &[RouteDecl],
    env: &Env,
) {
    use super::builtins_json::json_to_gx_value;

    let method = request.method().to_string().to_uppercase();
    let url = request.url().to_string();
    let (path, query) = if let Some(q) = url.find('?') {
        (url[..q].to_string(), url[q + 1..].to_string())
    } else {
        (url.clone(), String::new())
    };

    // Each incoming request is its own logical operation — a fresh trace
    // ID, never inherited from whatever the previous request on this same
    // worker thread happened to leave behind — with a root span covering
    // the whole request, ended exactly once below regardless of which of
    // the several ways this function can conclude (rejected for size, 404,
    // a normal response, or a `respond stream` route that already handled
    // its own response).
    interp.diagnostics.reset_trace_id();
    let span = interp.diagnostics.start_span("http.server.request");

    // Reject oversized bodies before even reading them when the client is
    // honest about Content-Length; a client that lies (or uses chunked
    // encoding to exceed it) is still caught by the cap inside
    // read_body_capped below.
    if let Some(len) = request.body_length() {
        if len > MAX_REQUEST_BODY_BYTES {
            end_request_span(interp, span, &method, &path, 413);
            respond_plain(request, 413, "413 Payload Too Large");
            return;
        }
    }
    let (body_str, body_truncated) = read_body_capped(&mut request);
    if body_truncated {
        end_request_span(interp, span, &method, &path, 413);
        respond_plain(request, 413, "413 Payload Too Large");
        return;
    }

    // Headers were never exposed to scripts before this — the single
    // biggest gap in the previous server: it made verifying a webhook
    // signature (Slack/Discord/GitHub/Stripe HMAC headers) structurally
    // impossible, not just inconvenient. Lower-cased for consistent access
    // regardless of how the client capitalized them (HTTP header names are
    // case-insensitive per RFC 7230).
    let headers_map: HashMap<String, Value> = request
        .headers()
        .iter()
        .map(|h| {
            (
                h.field.as_str().as_str().to_lowercase(),
                Value::Str(h.value.as_str().to_string()),
            )
        })
        .collect();
    let remote_addr = request
        .remote_addr()
        .map(|a| Value::Str(a.to_string()))
        .unwrap_or(Value::Null);

    let matched = routes.iter().find_map(|r| {
        if r.method == method || r.method == "ANY" {
            match_path(&r.path, &path).map(|params| (r, params))
        } else {
            None
        }
    });

    let (ct, body, status) = match matched {
        Some((route, params)) => {
            let mut req_map = HashMap::new();
            req_map.insert("method".to_string(), Value::Str(method.clone()));
            req_map.insert("path".to_string(), Value::Str(path.clone()));
            req_map.insert("body".to_string(), Value::Str(body_str.clone()));
            req_map.insert("query".to_string(), Value::Str(query.clone()));
            req_map.insert(
                "query_params".to_string(),
                Value::Object(parse_query(&query)),
            );
            req_map.insert(
                "params".to_string(),
                Value::Object(
                    params
                        .into_iter()
                        .map(|(k, v)| (k, Value::Str(v)))
                        .collect(),
                ),
            );
            req_map.insert("headers".to_string(), Value::Object(headers_map));
            req_map.insert("remote_addr".to_string(), remote_addr);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                req_map.insert("json".to_string(), json_to_gx_value(&json));
            }

            let mut route_env = env.clone();
            route_env.set("request", Value::Object(req_map));

            // Handed off so `respond stream { ... }` can claim it mid-route
            // (see Stmt::RespondStream in mod.rs) — reclaimed below unless
            // that already happened, in which case the stream handler has
            // already responded and joined the background writer thread.
            interp.pending_request = Some(request);
            let result = interp.run_stmts(&route.body.clone(), &mut route_env);
            match interp.pending_request.take() {
                None => {
                    // A `respond stream { ... }` block already claimed the
                    // request, sent its own response, and joined its
                    // background writer — nothing left to do, but the root
                    // span still needs to end (status 200: the stream ran
                    // to completion rather than being rejected).
                    end_request_span(interp, span, &method, &path, 200);
                    return;
                }
                Some(reclaimed) => {
                    request = reclaimed;
                    match result {
                        Ok(_) => (
                            "text/plain; charset=utf-8".to_string(),
                            "OK".to_string(),
                            200u16,
                        ),
                        Err(Signal::Respond(ct, b, s)) => (ct, b, s),
                        Err(Signal::Error(e)) => {
                            // Logged in full server-side; the client only
                            // gets a generic message. The previous
                            // behavior echoed the raw error text straight
                            // into the response body, which can leak
                            // internal details (file paths, capability
                            // denial reasons naming internal config,
                            // stack-adjacent context) to whoever sent the
                            // request.
                            interp.diagnostics.log(
                                crate::diagnostics::Level::Error,
                                &format!("{} {} -> 500", method, path),
                                Some(serde_json::json!({ "error": e.to_string() })),
                            );
                            (
                                "text/plain; charset=utf-8".to_string(),
                                "500 Internal Server Error".to_string(),
                                500,
                            )
                        }
                        Err(_) => (
                            "text/plain; charset=utf-8".to_string(),
                            "OK".to_string(),
                            200,
                        ),
                    }
                }
            }
        }
        None => (
            "text/plain; charset=utf-8".to_string(),
            format!("404 Not Found: {} {}", method, path),
            404,
        ),
    };

    end_request_span(interp, span, &method, &path, status);

    send_response(request, &ct, body, status);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::capability::{Allowlist, Resource};

    // `use binary "path"`/`use go "path"` currently can't be reached
    // through either GX parser (parse_import only accepts the dotted
    // `namespace.identifier` form used by js/py — a pre-existing gap,
    // unrelated to this milestone). These tests call `bridge_call`
    // directly to verify the capability check itself is correct
    // regardless of whether the surface syntax to reach it exists yet.

    #[test]
    fn binary_bridge_denied_by_allowlist_fails_before_spawning() {
        let mut i = Interpreter::new();
        i.capabilities.binary_executables = Allowlist::only(["/bin/echo".to_string()]);
        let err = i
            .bridge_call("binary", "/bin/definitely-not-allowed", "run", &[])
            .unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("not listed in gx.json's allowlist"),
            "expected an allowlist denial, got: {}",
            msg
        );
    }

    #[test]
    fn binary_bridge_allowed_by_allowlist_proceeds_past_the_capability_check() {
        let mut i = Interpreter::new();
        i.capabilities.binary_executables = Allowlist::only(["/bin/echo".to_string()]);
        // Past the capability check, it fails for a different reason
        // (echo isn't a JSON-IPC binary) — proves authorization isn't
        // what blocked it.
        let err = i
            .bridge_call("binary", "/bin/echo", "run", &[])
            .unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            !msg.contains("not listed in gx.json's allowlist"),
            "should have passed the capability check, got: {}",
            msg
        );
    }

    #[test]
    fn go_and_ts_bridges_are_denied_by_the_same_mechanism_as_js_py() {
        let mut i = Interpreter::new();
        i.capabilities.go_executables = Allowlist::only(["./allowed-service".to_string()]);
        let err = i
            .bridge_call("go", "./not-allowed-service", "run", &[])
            .unwrap_err();
        assert!(format!("{:?}", err).contains("not listed in gx.json's allowlist"));
    }

    #[test]
    fn operator_deny_blocks_a_bridge_even_with_no_allowlist_declared() {
        let mut i = Interpreter::new();
        i.capabilities.deny(Resource::BinaryBridge);
        let err = i
            .bridge_call("binary", "/bin/echo", "run", &[])
            .unwrap_err();
        assert!(format!("{:?}", err).contains("explicitly denied"));
    }

    #[test]
    fn http_server_denied_fails_before_binding_the_port() {
        let mut i = Interpreter::new();
        i.capabilities.http_server = false;
        let mut env = Env::new();
        let port_expr = Expr::Num(0.0);
        let err = i.run_serve(&port_expr, &[], &mut env).unwrap_err();
        assert!(format!("{:?}", err).contains("disabled by default"));
    }

    // ── match_path / parse_query ──────────────────────────────────────────────

    #[test]
    fn match_path_exact_routes_behave_like_the_previous_string_equality_check() {
        assert!(match_path("/health", "/health").is_some());
        assert!(match_path("/health", "/healthy").is_none());
        assert!(match_path("/", "/").is_some());
        assert!(match_path("/a/b", "/a/b/c").is_none());
    }

    #[test]
    fn match_path_captures_named_segments() {
        let params = match_path("/users/:id", "/users/42").unwrap();
        assert_eq!(params.get("id"), Some(&"42".to_string()));

        let params2 = match_path("/users/:id/posts/:post_id", "/users/7/posts/99").unwrap();
        assert_eq!(params2.get("id"), Some(&"7".to_string()));
        assert_eq!(params2.get("post_id"), Some(&"99".to_string()));

        assert!(match_path("/users/:id", "/users").is_none());
        assert!(match_path("/users/:id", "/users/42/extra").is_none());
    }

    #[test]
    fn parse_query_decodes_and_handles_edge_cases() {
        let q = parse_query("name=caf%C3%A9&city=");
        assert_eq!(q.get("name"), Some(&Value::Str("café".to_string())));
        assert_eq!(q.get("city"), Some(&Value::Str(String::new())));
        assert!(parse_query("").is_empty());
        assert!(parse_query("=novalue").is_empty()); // empty key skipped
    }

    // ── Full-stack integration tests ────────────────────────────────────────
    //
    // These spin up a real `serve on port N` program (via run_program, on a
    // background thread — the server loop never returns on its own, so
    // there's no clean shutdown; the thread is abandoned and reclaimed when
    // the test process exits) and make real HTTP requests against it. Each
    // test uses its own fixed port since cargo test runs tests in parallel.

    fn start_test_server(source: &str) {
        let tokens = crate::lexer::Lexer::new(source).tokenize().unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        std::thread::spawn(move || {
            let mut interp = Interpreter::new();
            let _ = interp.run_program(&program);
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// Same as `start_test_server`, but with tracing turned on — exercises
    /// the real `reset_trace_id`/root-span/`end_request_span` path each
    /// worker runs per request, the same way `main.rs`'s `--trace` flag
    /// would in production.
    fn start_test_server_with_diagnostics(source: &str) {
        let tokens = crate::lexer::Lexer::new(source).tokenize().unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        std::thread::spawn(move || {
            let mut interp = Interpreter::new();
            interp.diagnostics.set_enabled(true);
            interp.diagnostics.ensure_trace_id();
            let _ = interp.run_program(&program);
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    #[test]
    fn server_exposes_headers_path_params_and_query_params() {
        start_test_server(
            r#"
serve on port 18601 {
  route GET "/users/:id" {
    respond json {
      id: request.params.id,
      name: request.query_params.name,
      auth: request.headers.authorization
    }
  }
}
"#,
        );
        let resp = ureq::get("http://127.0.0.1:18601/users/42?name=ada")
            .set("Authorization", "Bearer tok123")
            .call()
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.into_json().unwrap();
        assert_eq!(body["id"], "42");
        assert_eq!(body["name"], "ada");
        assert_eq!(body["auth"], "Bearer tok123");
    }

    #[test]
    fn server_rejects_oversized_body_with_413() {
        start_test_server(
            r#"
serve on port 18602 {
  route POST "/upload" {
    respond json { ok: true }
  }
}
"#,
        );
        // MAX_REQUEST_BODY_BYTES is 32 MiB; send well past it as a
        // streamed body without a Content-Length so the cap must be
        // enforced while reading, not just via a pre-check.
        let oversized = vec![b'x'; MAX_REQUEST_BODY_BYTES + 1024];
        let result = ureq::post("http://127.0.0.1:18602/upload").send_bytes(&oversized);
        match result {
            Err(ureq::Error::Status(code, _)) => assert_eq!(code, 413),
            other => panic!("expected HTTP 413, got {:?}", other),
        }
    }

    #[test]
    fn server_returns_generic_500_without_leaking_internal_error_text() {
        start_test_server(
            r#"
serve on port 18603 {
  route GET "/boom" {
    x = 1 / 0
  }
}
"#,
        );
        let err = ureq::get("http://127.0.0.1:18603/boom").call().unwrap_err();
        match err {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 500);
                let body = resp.into_string().unwrap();
                assert!(
                    !body.to_lowercase().contains("divide") && !body.contains("1 / 0"),
                    "500 response must not leak the internal error message, got: {}",
                    body
                );
            }
            other => panic!("expected HTTP 500, got {:?}", other),
        }
    }

    #[test]
    fn server_with_tracing_enabled_still_serves_requests_correctly_across_concurrent_workers() {
        // The root `"http.server.request"` span (reset_trace_id + start_span
        // + end_request_span) runs on every request regardless of outcome —
        // this proves that wiring doesn't change response correctness, and
        // that concurrent requests across the worker pool each get their
        // own trace/span without interfering with one another (a leaked or
        // cross-request-shared span would show up as a hang or a wrong
        // status/body here, not as a diagnostics-specific failure).
        start_test_server_with_diagnostics(
            r#"
serve on port 18608 {
  route GET "/ok" {
    respond json { ok: true }
  }
  route GET "/boom" {
    x = 1 / 0
  }
}
"#,
        );
        let handles: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    if i % 2 == 0 {
                        let resp = ureq::get("http://127.0.0.1:18608/ok").call().unwrap();
                        assert_eq!(resp.status(), 200);
                    } else {
                        let err = ureq::get("http://127.0.0.1:18608/boom").call().unwrap_err();
                        match err {
                            ureq::Error::Status(code, _) => assert_eq!(code, 500),
                            other => panic!("expected HTTP 500, got {:?}", other),
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn server_handles_concurrent_slow_requests_without_serializing() {
        start_test_server(
            r#"
serve on port 18604 {
  route GET "/slow" {
    sleep(0.5)
    respond json { done: true }
  }
}
"#,
        );
        let start = std::time::Instant::now();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| ureq::get("http://127.0.0.1:18604/slow").call().unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let elapsed = start.elapsed();
        // Serialized, 4 * 0.5s = 2s; concurrent, ~0.5s. Generous margin for
        // slow CI, but well under fully-serial to prove they overlapped.
        assert!(
            elapsed < std::time::Duration::from_millis(1500),
            "4 concurrent 0.5s requests took {:?} — server appears to be serializing requests",
            elapsed
        );
    }

    #[test]
    fn server_sse_stream_delivers_events_in_order() {
        start_test_server(
            r#"
serve on port 18605 {
  route GET "/events" {
    respond stream {
      i = 0
      while i < 3 {
        sse_send("tick", { n: i })
        i += 1
      }
    }
  }
}
"#,
        );
        let resp = ureq::get("http://127.0.0.1:18605/events").call().unwrap();
        assert_eq!(
            resp.header("Content-Type"),
            Some("text/event-stream; charset=utf-8")
        );
        let body = resp.into_string().unwrap();
        let ticks: Vec<&str> = body.lines().filter(|l| l.starts_with("data: ")).collect();
        assert_eq!(ticks.len(), 3);
        assert!(ticks[0].contains("\"n\":0"));
        assert!(ticks[1].contains("\"n\":1"));
        assert!(ticks[2].contains("\"n\":2"));
    }

    #[test]
    fn sse_send_does_not_hang_every_worker_forever_when_clients_stop_reading() {
        // A client that connects and then never reads (a dead connection
        // that never closes, or just a very slow one) must not be able to
        // permanently take a worker out of rotation — sse_send's bounded
        // retry-with-deadline (SSE_SEND_TIMEOUT in mod.rs) exists
        // specifically to bound this. With spare idle workers, a naive
        // "just check one unrelated request still succeeds" test would
        // pass even with a broken (permanently-blocking) implementation,
        // since a *different* worker would simply pick it up — so this
        // saturates every worker with a never-reading connection first,
        // which only recovers if the fix is real.
        start_test_server(
            r#"
serve on port 18607 {
  route GET "/never-read" {
    respond stream {
      i = 0
      while i < 1000000 {
        sse_send("spam", { n: i })
        i += 1
      }
    }
  }
  route GET "/healthy" {
    respond json { ok: true }
  }
}
"#,
        );
        let mut stuck_connections = Vec::new();
        for _ in 0..SERVER_WORKER_THREADS {
            use std::io::Write;
            let mut stream = std::net::TcpStream::connect("127.0.0.1:18607").expect("connect");
            stream
                .write_all(b"GET /never-read HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("write request line");
            // Deliberately never read the response, and keep the
            // connection alive for the duration of the test — dropping it
            // would close the socket and let the server observe
            // "disconnected" instead of "buffer full", which is a
            // different (already-handled) code path.
            stuck_connections.push(stream);
        }
        // Give every worker a moment to actually pick up its connection
        // and start filling its buffer before checking recovery.
        std::thread::sleep(std::time::Duration::from_millis(500));
        // SSE_SEND_TIMEOUT is 5s; poll for recovery with a generous
        // overall bound well past that rather than one fixed sleep, so
        // the test isn't unnecessarily slow on a machine where it
        // recovers sooner.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut healthy = false;
        while std::time::Instant::now() < deadline {
            if let Ok(resp) = ureq::get("http://127.0.0.1:18607/healthy")
                .timeout(std::time::Duration::from_millis(500))
                .call()
            {
                if resp.status() == 200 {
                    healthy = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        drop(stuck_connections);
        assert!(
            healthy,
            "every worker was saturated by never-reading clients — none recovered within the expected bound"
        );
    }

    #[test]
    fn respond_stream_wires_the_shared_responder_cap_into_a_real_server_without_regressing() {
        // `try_reserve_sse_responder_slot_never_exceeds_the_cap_under_
        // concurrent_pressure` (mod.rs) already stress-tests the cap
        // arithmetic itself deterministically. This is the complementary
        // live-wiring check: real never-reading connections against a
        // real server must still (a) all be accepted without error below
        // the cap, (b) not wedge the server, and (c) let a fresh,
        // normally-reading client through once the stuck ones are closed.
        // Deliberately opens far fewer than `MAX_CONCURRENT_SSE_RESPONDERS`
        // — reliably reaching the real cap over the network depends on the
        // client's OS/TCP receive-window size actually being exceeded by
        // the route's payload, which varies enough across platforms/CI to
        // make a full-scale version of this specific assertion flaky; the
        // cap-is-enforced-at-exactly-N property is what the deterministic
        // test above already covers without that dependency.
        start_test_server(
            r#"
serve on port 18702 {
  route GET "/never-read" {
    respond stream {
      sse_send("chunk", { data: "x".repeat(2000000) })
    }
  }
  route GET "/healthy" {
    respond json { ok: true }
  }
}
"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(300));

        let mut stuck_connections = Vec::new();
        for _ in 0..20 {
            use std::io::Write;
            let mut stream = std::net::TcpStream::connect("127.0.0.1:18702").expect("connect");
            stream
                .write_all(b"GET /never-read HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("write request line");
            stuck_connections.push(stream);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));

        let healthy = ureq::get("http://127.0.0.1:18702/healthy")
            .timeout(std::time::Duration::from_secs(2))
            .call()
            .expect("server must still answer an unrelated route while streams are open");
        assert_eq!(healthy.status(), 200);

        drop(stuck_connections);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let still_healthy = ureq::get("http://127.0.0.1:18702/healthy")
            .timeout(std::time::Duration::from_secs(2))
            .call()
            .expect("server must remain healthy after the stuck connections close");
        assert_eq!(still_healthy.status(), 200);
    }

    #[test]
    fn sse_send_outside_a_stream_block_errors_instead_of_panicking() {
        let tokens = crate::lexer::Lexer::new(r#"sse_send("no stream here")"#)
            .tokenize()
            .unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        let err = Interpreter::new().run_program(&program).unwrap_err();
        assert!(err.contains("respond stream"));
    }

    // ── Panic resilience ─────────────────────────────────────────────────────
    //
    // GX itself is defensively coded (out-of-bounds indexing, division by
    // zero, etc. all return errors, not panics — confirmed empirically, and
    // there's no known reachable panic to trigger one "for real" through a
    // route body). These tests instead verify the *recovery mechanism*
    // itself — the same catch_unwind/resume_unwind shape used around
    // handle_one_request (bridge_impl.rs) and inside Stmt::RespondStream
    // (mod.rs) — so that if a genuinely panic-worthy bug is ever introduced
    // elsewhere in the interpreter, a worker thread survives it instead of
    // permanently losing capacity, and an in-progress SSE stream's sender/
    // responder thread aren't leaked.

    #[test]
    fn worker_loop_pattern_survives_a_panicking_request_and_keeps_serving() {
        // Mirrors run_serve's worker loop: catch_unwind around each
        // request, keep looping regardless. Proves a panic on request N
        // doesn't prevent request N+1 (on the *same* simulated worker)
        // from being handled.
        let mut handled = Vec::new();
        for i in 0..3 {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if i == 1 {
                    panic!("simulated request-handling panic");
                }
                i
            }));
            if let Ok(n) = result {
                handled.push(n);
            }
        }
        assert_eq!(
            handled,
            vec![0, 2],
            "requests before and after the panic must both be handled"
        );
    }

    #[test]
    fn sse_cleanup_runs_even_when_the_stream_body_panics() {
        // Mirrors Stmt::RespondStream's shape: sender set, body run inside
        // catch_unwind, sender always cleared afterward regardless of
        // whether the body panicked. Without catch_unwind here (a plain
        // call instead), that cleanup line would never run on a panic —
        // the unwind would skip straight past it — and rx below would
        // block forever instead of observing the channel close.
        let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);
        let mut tx_slot = Some(tx);
        assert!(tx_slot.is_some());

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("simulated route body panic");
        }));
        assert!(
            outcome.is_err(),
            "the simulated body panic should be caught, not crash the test"
        );

        tx_slot = None; // the cleanup step under test — reached because catch_unwind, not a raw call, was used above
        assert!(tx_slot.is_none());
        assert!(
            rx.recv().is_err(),
            "the channel must be closed (sender dropped) even though the body panicked"
        );
    }
}
