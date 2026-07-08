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
            self.capabilities
                .authorize(resource, Some(module))
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
                bridge.call(module, method, args).map_err(Signal::Error)
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
                bridge.call(module, method, args).map_err(Signal::Error)
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
                bridge.call(module, method, args).map_err(Signal::Error)
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
                bridge.call(module, method, args).map_err(Signal::Error)
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
        use super::builtins_json::json_to_gx_value;
        self.capabilities
            .authorize(Resource::HttpServer, None)
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
        for mut request in server.incoming_requests() {
            let method = request.method().to_string().to_uppercase();
            let url = request.url().to_string();
            let (path, query) = if let Some(q) = url.find('?') {
                (url[..q].to_string(), url[q + 1..].to_string())
            } else {
                (url.clone(), String::new())
            };
            let mut body_str = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut body_str);
            let mut req_map = HashMap::new();
            req_map.insert("method".to_string(), Value::Str(method.clone()));
            req_map.insert("path".to_string(), Value::Str(path.clone()));
            req_map.insert("body".to_string(), Value::Str(body_str.clone()));
            req_map.insert("query".to_string(), Value::Str(query.clone()));
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                req_map.insert("json".to_string(), json_to_gx_value(&json));
            }
            let matched = routes
                .iter()
                .find(|r| (r.method == method || r.method == "ANY") && r.path == path);
            let (ct, body, status) = if let Some(route) = matched {
                let mut route_env = env.clone();
                route_env.set("request", Value::Object(req_map));
                match self.run_stmts(&route.body.clone(), &mut route_env) {
                    Ok(_) => ("text/plain; charset=utf-8".into(), "OK".into(), 200u16),
                    Err(Signal::Respond(ct, b, s)) => (ct, b, s),
                    Err(Signal::Error(e)) => (
                        "text/plain; charset=utf-8".into(),
                        format!("500 Internal Error: {}", e),
                        500,
                    ),
                    Err(_) => ("text/plain; charset=utf-8".into(), "OK".into(), 200),
                }
            } else {
                (
                    "text/plain; charset=utf-8".into(),
                    format!("404 Not Found: {} {}", method, path),
                    404,
                )
            };
            let ct_header = tiny_http::Header::from_bytes(b"Content-Type".as_ref(), ct.as_bytes())
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
        Ok(Value::Null)
    }
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
}
