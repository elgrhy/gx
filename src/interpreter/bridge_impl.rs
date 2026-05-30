//! Bridge calls (JS/Python) and the HTTP `serve` block implementation.

use super::{Env, IResult, Interpreter, Signal};
use crate::ast::{Expr, RouteDecl};
use crate::bridge::Bridge;
use crate::value::Value;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
impl Interpreter {
    pub fn call_js(&mut self, module: &str, method: &str, args: &[Value]) -> Result<Value, Signal> {
        use crate::bridge::{json_to_value, value_to_json};
        use std::process::Command;

        let json_args = serde_json::to_string(&args.iter().map(value_to_json).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".into());

        let script = format!(
            r#"try {{
  const mod = require('{}');
  const parts = '{}' .split('.');
  let fn_ref = mod;
  for (const p of parts) {{ fn_ref = fn_ref[p]; }}
  const args = {};
  const result = typeof fn_ref === 'function' ? fn_ref(...args) : fn_ref;
  if (result && typeof result.then === 'function') {{
    result.then(r => console.log(JSON.stringify({{ok:true,result:r && r.data !== undefined ? r.data : r}})))
          .catch(e => console.log(JSON.stringify({{ok:false,error:String(e)}})));
  }} else {{
    console.log(JSON.stringify({{ok:true,result:result}}));
  }}
}} catch(e) {{ console.log(JSON.stringify({{ok:false,error:String(e)}})); }}"#,
            module, method, json_args
        );

        let output = Command::new("node")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| Signal::Error(format!("Failed to run node: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().last().unwrap_or("").trim();

        if last_line.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Signal::Error(format!("JS error: {}", stderr.trim())));
        }

        match serde_json::from_str::<serde_json::Value>(last_line) {
            Ok(json) => {
                if json["ok"].as_bool() == Some(true) {
                    Ok(json_to_value(&json["result"]))
                } else {
                    Err(Signal::Error(format!(
                        "JS error: {}",
                        json["error"].as_str().unwrap_or("unknown")
                    )))
                }
            }
            Err(_) => Ok(Value::Str(last_line.to_string())),
        }
    }

    pub fn bridge_call(
        &mut self,
        namespace: &str,
        module: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Signal> {
        // Enforce module allowlist when a gx.json manifest declares dependencies.
        match namespace {
            "js" => {
                if let Some(ref allowed) = self.allowed_js_modules.clone() {
                    if !allowed.iter().any(|m| m == module) {
                        return Err(Signal::Error(format!(
                            "JS module '{}' is not listed in gx.json dependencies. \
                             Add it with: gx install js.{}",
                            module, module
                        )));
                    }
                }
            }
            "py" => {
                if let Some(ref allowed) = self.allowed_py_modules.clone() {
                    if !allowed.iter().any(|m| m == module) {
                        return Err(Signal::Error(format!(
                            "Python module '{}' is not listed in gx.json dependencies. \
                             Add it with: gx install py.{}",
                            module, module
                        )));
                    }
                }
            }
            _ => {}
        }
        match namespace {
            "js" => self.call_js(module, method, args),
            "ts" => {
                // TypeScript bridge — lazy-init, shared with JS bridge slot
                if self.js_bridge.is_none() {
                    match Bridge::new_typescript() {
                        Ok(b) => self.js_bridge = Some(b),
                        Err(e) => return Err(Signal::Error(e)),
                    }
                }
                let bridge = self
                    .js_bridge
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
