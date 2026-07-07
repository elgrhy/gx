//! GX Package Interop Bridge
//! JS (Node.js), TypeScript, Python, Go, and generic binary bridges via JSON IPC.
//! Protocol: newline-delimited JSON over stdin/stdout.
//! Request:  {"type":"call","module":"m","method":"fn","args":[...]}
//! Response: {"ok":true,"result":...} or {"ok":false,"error":"..."}

use crate::value::Value;

// ── WASM stub — no subprocess support in browser ──────────────────────────────

#[cfg(target_arch = "wasm32")]
pub struct Bridge;

#[cfg(target_arch = "wasm32")]
impl Bridge {
    pub fn new_js() -> Result<Self, String> {
        Err("JS bridge not available in playground".into())
    }
    pub fn new_typescript() -> Result<Self, String> {
        Err("TypeScript bridge not available in playground".into())
    }
    pub fn new_python() -> Result<Self, String> {
        Err("Python bridge not available in playground".into())
    }
    pub fn new_binary(_path: &str) -> Result<Self, String> {
        Err("Binary bridge not available in playground".into())
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
    /// Present only for the TypeScript bridge (see `new_typescript`) — a
    /// temp file holding the shim source, cleaned up on `Drop`.
    shim_path: Option<std::path::PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeKind {
    Js,
    TypeScript,
    Python,
    /// Generic binary that speaks the JSON IPC protocol over stdin/stdout.
    Binary(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl Bridge {
    /// Take stdin/stdout from a just-spawned child and assemble the
    /// `Bridge` — the one piece of bookkeeping every constructor below
    /// needs, regardless of what was spawned or how.
    fn finish(
        mut child: Child,
        kind: BridgeKind,
        shim_path: Option<std::path::PathBuf>,
        process_label: &str,
    ) -> Result<Self, String> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("Failed to get {} stdin pipe", process_label))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| format!("Failed to get {} stdout pipe", process_label))?,
        );
        Ok(Bridge {
            kind,
            _child: child,
            stdin,
            stdout,
            shim_path,
        })
    }

    pub fn new_js() -> Result<Self, String> {
        // Check node is available
        if !command_exists("node") {
            return Err("Node.js not found. Install from https://nodejs.org".into());
        }
        // The shim is passed via `-e` (a command-line argument, exactly like
        // the Python bridge's `-c JS_SHIM` below) — NOT written to stdin.
        // JS_SHIM is CommonJS (`require('readline')`), so it must run in
        // Node's default CommonJS mode, not `--input-type=module` (which
        // rejects `require`). Just as important: passing it via `-e` leaves
        // stdin completely free for the shim's own `readline` interface to
        // consume the ongoing stream of JSON-IPC request lines — writing the
        // shim to stdin instead (the previous, never-exercised approach)
        // meant Node was simultaneously trying to read its own script AND
        // the shim's request protocol from the same pipe, and in
        // `--input-type=module` mode never even started executing since ES
        // modules aren't evaluated until stdin reaches EOF, which the bridge
        // deliberately never sends (the pipe stays open for the process's
        // whole lifetime) — the process would simply hang forever.
        let child = Command::new("node")
            .arg("-e")
            .arg(JS_SHIM)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start Node.js: {}", e))?;

        Self::finish(child, BridgeKind::Js, None, "Node.js")
    }

    /// TypeScript bridge — tries `tsx` first (fast, zero-config), then `ts-node`.
    /// Falls back to plain `node` if neither is installed (for .js files).
    pub fn new_typescript() -> Result<Self, String> {
        let runner = if command_exists("tsx") {
            "tsx"
        } else if command_exists("ts-node") {
            "ts-node"
        } else if command_exists("node") {
            "node"
        } else {
            return Err("TypeScript bridge: neither tsx, ts-node, nor node found.\n\
                 Install tsx with: npm install -g tsx"
                .into());
        };

        // The shim is written to a temp file and passed as a plain file
        // argument — not stdin, and not a per-tool `-e`/`--eval` flag.
        // `node -e` is what the JS bridge above uses and has verified
        // works, but `tsx`/`ts-node`'s exact inline-eval flag support isn't
        // something to rely on uniformly across all three possible
        // runners. Running a file path is the one invocation every
        // JS/TS runner supports identically, and — just as important —
        // it leaves stdin completely free for the shim's own
        // request/response protocol from the very first byte, avoiding the
        // exact hang class documented on `new_js` above (this bridge
        // previously wrote the shim to stdin *and* passed
        // `--input-type=module`, which would have hung forever the moment
        // the plain-`node` fallback path actually ran, for the same reason
        // `new_js` did before it was fixed).
        let shim_path = write_shim_to_temp_file("ts", JS_SHIM)?;
        let child = Command::new(runner)
            .arg(&shim_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                let _ = std::fs::remove_file(&shim_path);
                format!("Failed to start TypeScript runner ({}): {}", runner, e)
            })?;

        Self::finish(
            child,
            BridgeKind::TypeScript,
            Some(shim_path),
            "TypeScript runner",
        )
    }

    pub fn new_python() -> Result<Self, String> {
        let python = find_python().ok_or("Python 3 not found. Install from https://python.org")?;
        let child = Command::new(&python)
            .arg("-c")
            .arg(PY_SHIM)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start Python: {}", e))?;

        Self::finish(child, BridgeKind::Python, None, "Python")
    }

    /// Generic binary bridge — calls any executable that reads JSON lines from stdin
    /// and writes JSON lines to stdout using the same IPC protocol.
    /// Use for Go, Rust, Java, .NET, or any compiled service.
    pub fn new_binary(path: &str) -> Result<Self, String> {
        if !std::path::Path::new(path).exists() {
            return Err(format!(
                "Binary bridge: executable '{}' not found. Build it first.",
                path
            ));
        }
        let child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                // Same classification the native process runtime uses
                // (`builtins_process::classify_spawn_error`) — the exists()
                // check above catches a missing file, but not "exists but
                // isn't executable", which shows up here as a permission
                // error and deserves a more specific message than the OS's.
                use crate::interpreter::classify_spawn_error;
                use crate::interpreter::SpawnErrorKind;
                match classify_spawn_error(&e) {
                    SpawnErrorKind::PermissionDenied => format!(
                        "Binary bridge: '{}' exists but isn't executable ({}). Try: chmod +x {}",
                        path, e, path
                    ),
                    _ => format!("Failed to start binary '{}': {}", path, e),
                }
            })?;

        Self::finish(child, BridgeKind::Binary(path.to_string()), None, "binary")
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
        match &self.kind {
            BridgeKind::Js => "JS",
            BridgeKind::TypeScript => "TypeScript",
            BridgeKind::Python => "Python",
            BridgeKind::Binary(path) => path.as_str(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, r#"{{"type":"exit"}}"#);
        if let Some(path) = &self.shim_path {
            let _ = std::fs::remove_file(path);
        }
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
        Value::Closure(params, _, _) => serde_json::json!(format!("<fn({})>", params.join(", "))),
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

/// Monotonic counter making every temp shim filename unique within this
/// process — see the note on `write_shim_to_temp_file` for why this can't
/// just be the PID.
#[cfg(not(target_arch = "wasm32"))]
static SHIM_FILE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write `source` to a uniquely-named temp file and return its path.
///
/// The filename must be unique *per bridge instance*, not just per OS
/// process: `Interpreter::new_typescript()` is a lazy singleton within one
/// `Interpreter`, but `parallel {}` runs multiple independent `Interpreter`s
/// concurrently in the same process (see `eval_parallel_map`), and each one
/// can create its own TypeScript bridge at the same time. Using only the PID
/// here previously meant two concurrently-created bridges could collide on
/// the same filename — one's `Drop` deleting the file out from under the
/// other's still-starting `node`/`tsx`/`ts-node` process, which then failed
/// immediately with "Bridge process ended unexpectedly". A per-instance
/// counter closes that race.
#[cfg(not(target_arch = "wasm32"))]
fn write_shim_to_temp_file(label: &str, source: &str) -> Result<std::path::PathBuf, String> {
    let n = SHIM_FILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "gx_bridge_shim_{}_{}_{}.js",
        label,
        std::process::id(),
        n
    ));
    std::fs::write(&path, source)
        .map_err(|e| format!("Failed to write bridge shim to {}: {}", path.display(), e))?;
    Ok(path)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` on a background thread and fail *with its real panic message*
    /// if it doesn't finish within `timeout` — a hung bridge is exactly the
    /// regression class this guards against (both `new_js` and
    /// `new_typescript` have, at different points, actually hung forever;
    /// see their doc comments). Using `is_finished()` + `join()` rather than
    /// a channel means an assertion failure inside `f` is reported as that
    /// failure, not misreported as a timeout.
    fn with_timeout<F: FnOnce() + Send + 'static>(timeout: std::time::Duration, f: F) {
        let handle = std::thread::spawn(f);
        let start = std::time::Instant::now();
        while !handle.is_finished() {
            if start.elapsed() > timeout {
                panic!(
                    "operation did not complete within {:?} — likely hung",
                    timeout
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if let Err(e) = handle.join() {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn js_bridge_call_completes_without_hanging() {
        if !command_exists("node") {
            eprintln!("skipping js_bridge_call_completes_without_hanging: node not installed");
            return;
        }
        with_timeout(std::time::Duration::from_secs(10), || {
            let mut bridge = Bridge::new_js().expect("failed to start JS bridge");
            let result = bridge
                .call(
                    "path",
                    "join",
                    &[Value::Str("a".into()), Value::Str("b".into())],
                )
                .expect("bridge call failed");
            assert_eq!(result, Value::Str("a/b".to_string()));
        });
    }

    #[test]
    fn typescript_bridge_call_completes_without_hanging() {
        // In this environment this exercises the plain-`node` fallback path
        // (neither tsx nor ts-node installed) — the exact path that used to
        // hang forever before this milestone's fix.
        if !command_exists("node") {
            eprintln!(
                "skipping typescript_bridge_call_completes_without_hanging: node not installed"
            );
            return;
        }
        with_timeout(std::time::Duration::from_secs(10), || {
            let mut bridge = Bridge::new_typescript().expect("failed to start TS bridge");
            let result = bridge
                .call("path", "basename", &[Value::Str("/a/b/c.txt".into())])
                .expect("bridge call failed");
            assert_eq!(result, Value::Str("c.txt".to_string()));
        });
    }

    #[test]
    fn typescript_bridge_cleans_up_shim_temp_file_on_drop() {
        if !command_exists("node") {
            eprintln!(
                "skipping typescript_bridge_cleans_up_shim_temp_file_on_drop: node not installed"
            );
            return;
        }
        let shim_path = {
            let bridge = Bridge::new_typescript().expect("failed to start TS bridge");
            bridge
                .shim_path
                .clone()
                .expect("expected a shim temp file for the TypeScript bridge")
        };
        // `bridge` was dropped at the end of the block above.
        assert!(
            !shim_path.exists(),
            "shim temp file {:?} was not cleaned up on Drop",
            shim_path
        );
    }

    #[test]
    fn python_bridge_call_completes_without_hanging() {
        if find_python().is_none() {
            eprintln!(
                "skipping python_bridge_call_completes_without_hanging: python not installed"
            );
            return;
        }
        with_timeout(std::time::Duration::from_secs(10), || {
            let mut bridge = Bridge::new_python().expect("failed to start Python bridge");
            let result = bridge
                .call(
                    "os.path",
                    "join",
                    &[Value::Str("a".into()), Value::Str("b".into())],
                )
                .expect("bridge call failed");
            assert_eq!(result, Value::Str("a/b".to_string()));
        });
    }
}
