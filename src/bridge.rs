//! GX Package Interop Bridge
//! JS (Node.js) and Python bridges via subprocess JSON IPC.

use crate::value::Value;

// ── WASM stub — no subprocess support in browser ──────────────────────────────

#[cfg(target_arch = "wasm32")]
pub struct Bridge;

#[cfg(target_arch = "wasm32")]
impl Bridge {
    pub fn new_js() -> Result<Self, String> {
        Err("JS bridge not available in playground".into())
    }
    pub fn new_python() -> Result<Self, String> {
        Err("Python bridge not available in playground".into())
    }
    pub fn call(&mut self, _module: &str, _method: &str, _args: &[Value]) -> Result<Value, String> {
        Err("Bridge not available in playground".into())
    }
}

#[cfg(target_arch = "wasm32")]
pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => serde_json::json!(n),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        _ => serde_json::Value::Null,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        _ => Value::Null,
    }
}

// ── Native implementation ─────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

// ── Native JS/Python bridge implementation ────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
const JS_SHIM: &str = r#"
const readline = require('readline');
const rl = readline.createInterface({ input: process.stdin, terminal: false });

rl.on('line', (line) => {
  let req;
  try { req = JSON.parse(line); } catch(e) { respond({ ok: false, error: 'Invalid JSON: ' + e.message }); return; }

  if (req.type === 'exit') { process.exit(0); }
  if (req.type === 'call') {
    try {
      const mod = require(req.module);
      // Navigate nested method: "get", "post", "data.parse" etc.
      const parts = req.method.split('.');
      let target = mod;
      let parent = mod;
      for (let i = 0; i < parts.length - 1; i++) {
        parent = target;
        target = target[parts[i]];
      }
      const fn_name = parts[parts.length - 1];
      const fn_ref = target[fn_name] || target;
      let result;
      if (typeof fn_ref === 'function') {
        result = fn_ref.apply(parent, req.args || []);
      } else {
        result = fn_ref;
      }
      // Handle promises
      if (result && typeof result.then === 'function') {
        result.then(val => respond({ ok: true, result: serialize(val) }))
              .catch(err => respond({ ok: false, error: String(err) }));
      } else {
        respond({ ok: true, result: serialize(result) });
      }
    } catch(e) {
      respond({ ok: false, error: String(e) });
    }
  }
});

function serialize(v) {
  if (v === null || v === undefined) return null;
  if (typeof v === 'function') return '[Function]';
  if (typeof v === 'object' && v.data !== undefined) return v.data; // axios response
  try { JSON.stringify(v); return v; } catch(e) { return String(v); }
}

function respond(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
}
"#;

// ── Python Shim (embedded) ────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
const PY_SHIM: &str = r#"
import sys
import json
import importlib

def get_nested(obj, parts):
    for part in parts:
        obj = getattr(obj, part, None)
        if obj is None:
            return None
    return obj

def serialize(v):
    try:
        json.dumps(v)
        return v
    except (TypeError, ValueError):
        return str(v)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except json.JSONDecodeError as e:
        sys.stdout.write(json.dumps({"ok": False, "error": str(e)}) + "\n")
        sys.stdout.flush()
        continue

    if req.get("type") == "exit":
        sys.exit(0)

    if req.get("type") == "call":
        try:
            mod = importlib.import_module(req["module"])
            parts = req["method"].split(".")
            if len(parts) == 1:
                fn_ref = getattr(mod, parts[0])
            else:
                obj = get_nested(mod, parts[:-1])
                fn_ref = getattr(obj, parts[-1])

            args = req.get("args", [])
            if callable(fn_ref):
                result = fn_ref(*args)
            else:
                result = fn_ref

            sys.stdout.write(json.dumps({"ok": True, "result": serialize(result)}) + "\n")
            sys.stdout.flush()
        except Exception as e:
            sys.stdout.write(json.dumps({"ok": False, "error": str(e)}) + "\n")
            sys.stdout.flush()
"#;

// ── Bridge ────────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub struct Bridge {
    pub kind: BridgeKind,
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeKind {
    Js,
    Python,
}

#[cfg(not(target_arch = "wasm32"))]
impl Bridge {
    pub fn new_js() -> Result<Self, String> {
        // Check node is available
        if !command_exists("node") {
            return Err("Node.js not found. Install from https://nodejs.org".into());
        }
        let mut child = Command::new("node")
            .arg("--input-type=module")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start Node.js: {}", e))?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());

        // Send the shim code
        // Use CommonJS shim (avoid --input-type=module issues)
        writeln!(stdin, "{}", JS_SHIM).map_err(|e| format!("Shim write failed: {}", e))?;

        Ok(Bridge {
            kind: BridgeKind::Js,
            _child: child,
            stdin,
            stdout,
        })
    }

    pub fn new_python() -> Result<Self, String> {
        let python = find_python().ok_or("Python 3 not found. Install from https://python.org")?;
        let mut child = Command::new(&python)
            .arg("-c")
            .arg(PY_SHIM)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start Python: {}", e))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());

        Ok(Bridge {
            kind: BridgeKind::Python,
            _child: child,
            stdin,
            stdout,
        })
    }

    pub fn call(&mut self, module: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        let json_args: Vec<serde_json::Value> = args.iter().map(value_to_json).collect();
        let req = serde_json::json!({
            "type": "call",
            "module": module,
            "method": method,
            "args": json_args
        });

        let msg = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        writeln!(self.stdin, "{}", msg).map_err(|e| format!("Bridge write failed: {}", e))?;

        let mut response_line = String::new();
        self.stdout
            .read_line(&mut response_line)
            .map_err(|e| format!("Bridge read failed: {}", e))?;

        if response_line.is_empty() {
            return Err("Bridge process ended unexpectedly".into());
        }

        let resp: serde_json::Value = serde_json::from_str(response_line.trim())
            .map_err(|e| format!("Bridge JSON parse error: {}", e))?;

        if resp["ok"].as_bool() == Some(true) {
            Ok(json_to_value(&resp["result"]))
        } else {
            let err = resp["error"].as_str().unwrap_or("unknown error");
            Err(format!("{} bridge error: {}", self.kind_name(), err))
        }
    }

    fn kind_name(&self) -> &str {
        match self.kind {
            BridgeKind::Js => "JS",
            BridgeKind::Python => "Python",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, r#"{{"type":"exit"}}"#);
    }
}

// ── Conversion: Value ↔ JSON ──────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::json!(b),
        Value::Number(n) => serde_json::json!(n),
        Value::Str(s) => serde_json::json!(s),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Object(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                map.insert(k.clone(), value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_value).collect()),
        serde_json::Value::Object(m) => {
            let mut map = std::collections::HashMap::new();
            for (k, v) in m {
                map.insert(k.clone(), json_to_value(v));
            }
            Value::Object(map)
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn command_exists(cmd: &str) -> bool {
    Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn find_python() -> Option<String> {
    for candidate in &["python3", "python"] {
        if command_exists(candidate) {
            return Some((*candidate).into());
        }
    }
    None
}
