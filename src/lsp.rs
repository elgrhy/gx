//! A minimal, intentionally-scoped Language Server (`gx lsp`) — stdio,
//! JSON-RPC 2.0, hand-rolled framing (no new dependency: everything here
//! is built on `serde_json`, already a dependency for every other GX
//! subsystem's JSON handling).
//!
//! Implemented, and genuinely working end to end in any LSP-speaking
//! editor:
//! - `textDocument/didOpen` / `didChange` / `didClose` → re-parse on every
//!   edit and `textDocument/publishDiagnostics`, reusing the same
//!   column-aware error rendering the CLI uses (`diagnostics_render`) so
//!   editor squiggles land on the exact token, not just "somewhere on this
//!   line".
//! - `textDocument/hover` — builtin function signatures/descriptions for
//!   the identifier under the cursor (a static table; see `builtin_docs`).
//! - `textDocument/definition` — jumps to a `function`/`agent`/`helper`/
//!   `tool` declaration *within the same file* for the identifier under
//!   the cursor (multi-file/cross-import go-to-definition is not
//!   attempted here — see the module's tests for exactly what is/isn't
//!   covered).
//!
//! Deliberately NOT implemented in this pass — each would be a
//! substantial feature in its own right, and a half-implemented version
//! of any of them (a rename that misses references, a completion list
//! that's actually just keyword-matching) would be worse than being
//! honest that it isn't there yet: rename, find-references across files,
//! completion/autocomplete suggestion lists, semantic (token-type-aware)
//! highlighting, signature help (parameter-position hints while typing a
//! call), snippets.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};

// ── JSON-RPC framing ─────────────────────────────────────────────────────────

/// Read one `Content-Length`-framed JSON-RPC message. Returns `None` on
/// EOF (the client closed the pipe) or a malformed frame — both end the
/// server loop the same way a real editor disconnecting would.
pub fn read_message<R: BufRead>(reader: &mut R) -> Option<serde_json::Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).ok()?;
        if n == 0 {
            return None; // EOF before a full header block
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
        // Other headers (e.g. Content-Type) are accepted and ignored.
    }
    let len = content_length?;
    // A client-supplied length is bounded defensively — a malformed or
    // hostile length claiming gigabytes would otherwise make a single
    // frame allocate unboundedly before `read_exact` ever gets a chance
    // to fail.
    if len > 64 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

/// Write one `Content-Length`-framed JSON-RPC message.
pub fn write_message<W: Write>(writer: &mut W, value: &serde_json::Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn response_ok(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn response_error(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn notification(method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// Parse GX source using whichever syntax it's written in — the same
/// dispatch `main.rs`'s own `parse_file` uses, minus the file-reading
/// step (an LSP document's text always comes from the client, never a
/// path on disk).
fn parse_source(source: &str) -> Result<crate::ast::Program, String> {
    if crate::indent_parser::is_indent_syntax(source) {
        crate::indent_parser::parse(source)
    } else {
        let tokens = crate::lexer::Lexer::new(source).tokenize()?;
        crate::parser::Parser::new(tokens).parse()
    }
}

// ── Server ────────────────────────────────────────────────────────────────────

/// Run the LSP server, blocking, until the client disconnects or sends
/// `exit`. `gx lsp`'s entire implementation — `main.rs` just calls this.
pub fn run() {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut docs: HashMap<String, String> = HashMap::new();

    while let Some(msg) = read_message(&mut reader) {
        if handle_message(&msg, &mut docs, &mut writer).is_none() {
            break;
        }
    }
}

/// Handle one message, writing any response/notification to `writer`.
/// Returns `None` to signal the server should stop (an `exit`
/// notification) — everything else returns `Some(())` regardless of
/// whether the message produced a response, so `run`'s loop keeps going.
fn handle_message<W: Write>(
    msg: &serde_json::Value,
    docs: &mut HashMap<String, String>,
    writer: &mut W,
) -> Option<()> {
    let method = msg.get("method").and_then(|m| m.as_str())?;
    let id = msg.get("id").cloned();
    let params = msg
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match method {
        "initialize" => {
            if let Some(id) = id {
                let result = serde_json::json!({
                    "capabilities": {
                        "textDocumentSync": 1, // Full document sync
                        "hoverProvider": true,
                        "definitionProvider": true,
                    },
                    "serverInfo": { "name": "gx-lsp", "version": env!("CARGO_PKG_VERSION") },
                });
                let _ = write_message(writer, &response_ok(id, result));
            }
        }
        "initialized" | "$/cancelRequest" => {
            // No response required for either.
        }
        "textDocument/didOpen" => {
            if let Some((uri, text)) = doc_open_params(&params) {
                docs.insert(uri.clone(), text);
                publish_diagnostics(writer, docs, &uri);
            }
        }
        "textDocument/didChange" => {
            if let Some((uri, text)) = doc_change_params(&params) {
                docs.insert(uri.clone(), text);
                publish_diagnostics(writer, docs, &uri);
            }
        }
        "textDocument/didClose" => {
            if let Some(uri) = params
                .get("textDocument")
                .and_then(|t| t.get("uri"))
                .and_then(|u| u.as_str())
            {
                docs.remove(uri);
            }
        }
        "textDocument/hover" => {
            if let Some(id) = id {
                let result = hover_result(&params, docs);
                let _ = write_message(writer, &response_ok(id, result));
            }
        }
        "textDocument/definition" => {
            if let Some(id) = id {
                let result = definition_result(&params, docs);
                let _ = write_message(writer, &response_ok(id, result));
            }
        }
        "shutdown" => {
            if let Some(id) = id {
                let _ = write_message(writer, &response_ok(id, serde_json::Value::Null));
            }
        }
        "exit" => {
            return None;
        }
        _ => {
            // An unrecognized *request* (has an id) gets a proper
            // method-not-found response rather than being silently
            // dropped, so a client waiting on it doesn't hang forever.
            // An unrecognized *notification* (no id) is safe to ignore —
            // the protocol explicitly allows servers to ignore
            // notifications they don't understand.
            if let Some(id) = id {
                let _ = write_message(writer, &response_error(id, -32601, "method not found"));
            }
        }
    }
    Some(())
}

fn doc_open_params(params: &serde_json::Value) -> Option<(String, String)> {
    let td = params.get("textDocument")?;
    let uri = td.get("uri")?.as_str()?.to_string();
    let text = td.get("text")?.as_str()?.to_string();
    Some((uri, text))
}

fn doc_change_params(params: &serde_json::Value) -> Option<(String, String)> {
    let uri = params
        .get("textDocument")?
        .get("uri")?
        .as_str()?
        .to_string();
    // Full sync (textDocumentSync: 1) — the client always sends the
    // complete new text in the last contentChanges entry, never a diff.
    let text = params
        .get("contentChanges")?
        .as_array()?
        .last()?
        .get("text")?
        .as_str()?
        .to_string();
    Some((uri, text))
}

// ── Diagnostics ───────────────────────────────────────────────────────────────

fn publish_diagnostics<W: Write>(writer: &mut W, docs: &HashMap<String, String>, uri: &str) {
    let Some(source) = docs.get(uri) else {
        return;
    };
    let diagnostics = compute_diagnostics(source);
    let _ = write_message(
        writer,
        &notification(
            "textDocument/publishDiagnostics",
            serde_json::json!({ "uri": uri, "diagnostics": diagnostics }),
        ),
    );
}

/// Parse `source` and return LSP `Diagnostic[]` — empty when it parses
/// cleanly (which also *clears* any previously published diagnostics for
/// this document once the client applies an empty array, exactly as
/// intended). Reuses `diagnostics_render::parse_location` so a syntax
/// error lands on the same line/column the CLI's `gx run`/`gx check`
/// would report, converted from GX's 1-based line/col to LSP's 0-based
/// line/character.
fn compute_diagnostics(source: &str) -> Vec<serde_json::Value> {
    let Err(err) = parse_source(source) else {
        return Vec::new();
    };
    let Some(loc) = crate::diagnostics_render::parse_location(&err) else {
        // No recoverable location — still surface *something* rather
        // than silently showing no error at all, anchored at the top of
        // the document.
        return vec![lsp_diagnostic(0, 0, 0, 1, &err)];
    };
    let line0 = loc.line.saturating_sub(1);
    let col0 = loc.col.map(|c| c.saturating_sub(1)).unwrap_or(0);
    let end_col = col0 + 1;
    vec![lsp_diagnostic(line0, col0, line0, end_col, &loc.message)]
}

fn lsp_diagnostic(
    start_line: usize,
    start_char: usize,
    end_line: usize,
    end_char: usize,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "range": {
            "start": { "line": start_line, "character": start_char },
            "end": { "line": end_line, "character": end_char },
        },
        "severity": 1, // Error
        "source": "gx",
        "message": message,
    })
}

// ── Hover ─────────────────────────────────────────────────────────────────────

fn hover_result(params: &serde_json::Value, docs: &HashMap<String, String>) -> serde_json::Value {
    let Some((source, word)) = word_at_cursor(params, docs) else {
        return serde_json::Value::Null;
    };
    if let Some(doc) = builtin_docs::lookup(&word) {
        return serde_json::json!({
            "contents": { "kind": "markdown", "value": doc },
        });
    }
    // Not a builtin — check whether it's a user-defined function/agent/
    // tool in the same document, and show its signature if so (hover
    // works for user code too, not only builtins).
    if let Ok(program) = parse_source(source) {
        if let Some(sig) = user_definition_signature(&program, &word) {
            return serde_json::json!({ "contents": { "kind": "markdown", "value": sig } });
        }
    }
    serde_json::Value::Null
}

fn user_definition_signature(program: &crate::ast::Program, name: &str) -> Option<String> {
    if let Some(f) = program.functions.iter().find(|f| f.name == name) {
        return Some(format!(
            "```gx\nfunction {}({})\n```",
            f.name,
            f.params.join(", ")
        ));
    }
    if let Some(h) = program.helpers.iter().find(|h| h.name == name) {
        return Some(format!("```gx\nagent \"{}\"\n```", h.name));
    }
    if let Some(t) = program.tools.iter().find(|t| t.name == name) {
        let params: Vec<String> = t.params.iter().map(|p| p.name.clone()).collect();
        return Some(format!(
            "```gx\ntool \"{}\"({})\n```\n\n{}",
            t.name,
            params.join(", "),
            t.description
        ));
    }
    None
}

// ── Go to definition ─────────────────────────────────────────────────────────

fn definition_result(
    params: &serde_json::Value,
    docs: &HashMap<String, String>,
) -> serde_json::Value {
    let Some(uri) = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(|u| u.as_str())
    else {
        return serde_json::Value::Null;
    };
    let Some((source, word)) = word_at_cursor(params, docs) else {
        return serde_json::Value::Null;
    };
    let Ok(program) = parse_source(source) else {
        return serde_json::Value::Null;
    };

    let def_line = program
        .functions
        .iter()
        .find(|f| f.name == word)
        .map(|f| f.line)
        .or_else(|| {
            program
                .helpers
                .iter()
                .find(|h| h.name == word)
                .map(|h| h.line)
        })
        .or_else(|| {
            program
                .tools
                .iter()
                .find(|t| t.name == word)
                .map(|t| t.line)
        });

    match def_line {
        Some(line) => {
            let line0 = line.saturating_sub(1);
            serde_json::json!({
                "uri": uri,
                "range": {
                    "start": { "line": line0, "character": 0 },
                    "end": { "line": line0, "character": 0 },
                },
            })
        }
        None => serde_json::Value::Null,
    }
}

// ── Shared: word under the cursor ────────────────────────────────────────────

/// Resolve `params.textDocument.uri` + `params.position` to (that
/// document's current source text, the identifier the cursor is inside
/// or immediately after). `None` if the document isn't open, the
/// position is out of range, or there's no identifier there (e.g. the
/// cursor is on whitespace or punctuation).
fn word_at_cursor<'a>(
    params: &serde_json::Value,
    docs: &'a HashMap<String, String>,
) -> Option<(&'a str, String)> {
    let uri = params.get("textDocument")?.get("uri")?.as_str()?;
    let source = docs.get(uri)?.as_str();
    let position = params.get("position")?;
    let line_idx = position.get("line")?.as_u64()? as usize;
    let char_idx = position.get("character")?.as_u64()? as usize;

    let line = source.lines().nth(line_idx)?;
    let chars: Vec<char> = line.chars().collect();
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

    // char_idx can legitimately equal chars.len() (cursor at end of
    // line) — clamp rather than treat as out of range.
    let at = char_idx.min(chars.len());
    // If the cursor sits just past the end of an identifier (the common
    // case — most editors report the position *after* the character
    // under a text-hover), look one character back first.
    let anchor = if at < chars.len() && is_word_char(chars[at]) {
        at
    } else if at > 0 && is_word_char(chars[at - 1]) {
        at - 1
    } else {
        return None;
    };

    let start = (0..=anchor)
        .rev()
        .find(|&i| !is_word_char(chars[i]))
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = (anchor..chars.len())
        .find(|&i| !is_word_char(chars[i]))
        .unwrap_or(chars.len());
    if start >= end {
        return None;
    }
    Some((source, chars[start..end].iter().collect()))
}

// ── Builtin documentation table ──────────────────────────────────────────────

// `pub(crate)`, not private, so `gx repl`'s `:help <name>` command (in
// main.rs) can reuse the exact same table rather than maintaining a
// second copy that would inevitably drift from this one.
pub(crate) mod builtin_docs {
    /// A small, hand-maintained subset of the Built-in Functions reference
    /// (docs/language_reference.md) — the most commonly used/discovered
    /// builtins across the categories a developer is most likely to hover
    /// over while writing a script. Not exhaustive by design: growing this
    /// table is easy (one line each) and low-risk, so it's deliberately
    /// scoped to what's clearly useful today rather than mechanically
    /// transcribing the entire several-hundred-entry reference table up
    /// front.
    pub fn lookup(name: &str) -> Option<&'static str> {
        Some(match name {
            "log" => "```gx\nlog(v)\n```\nPrint value to stdout with newline.",
            "say" => "```gx\nsay x\n```\nPrint value (statement form).",
            "assert" => "```gx\nassert condition, message?\n```\nThrows `Signal::AssertFail` if `condition` is falsy.",
            "retry" => "```gx\nretry(fn, max?, opts?)\n```\nCalls `fn()` until it succeeds — a thrown error *or* a returned `{ ok: false, ... }` both count as a retryable failure. `opts`: `delay` (ms), `backoff` (`\"exponential\"`|`\"linear\"`|`\"fixed\"`).",
            "unwrap" => "```gx\nunwrap(result)\n```\nIf `result` is `{ ok: false, error, error_kind, ... }`, throws (catchable via `try/catch`). Anything else passes through unchanged.",
            "has_capability" => "```gx\nhas_capability(resource, name?)\n```\n`true`/`false` — would this resource/name currently be authorized? Never throws, never has a side effect.",
            "http_get" => "```gx\nhttp_get(url, opts?)\n```\nReturns `{ ok, status, body, data?, error?, error_kind? }` — never throws on a failed *request* (timeout, non-2xx, blocked).",
            "http_post" => "```gx\nhttp_post(url, body, opts?)\n```\nSame return shape as `http_get`.",
            "http_request" => "```gx\nhttp_request({ url, method, headers?, body?, timeout? })\n```\nUnified form of `http_get`/`http_post`/`http_put`/`http_delete`.",
            "db_query" => "```gx\ndb_query(path, sql, params?)\n```\nReturns an array of row objects. Throws on failure (catchable via `try/catch`) rather than returning `{ ok: false }`.",
            "db_exec" => "```gx\ndb_exec(path, sql, params?)\n```\nRuns a non-SELECT statement. Throws on failure.",
            "process_run" => "```gx\nprocess_run({ command, args?, timeout?, cwd?, env? })\n```\nReturns `{ ok, stdout, stderr, exit_code, error?, error_kind? }`. Requires `--allow-process`.",
            "task_spawn" => "```gx\ntask_spawn(fn, opts?)\n```\nRuns `fn` concurrently; returns a handle for `task_wait`/`task_cancel`.",
            "task_wait" => "```gx\ntask_wait(handle, timeout_ms?)\n```\nReturns `{ ok, value, error?, cancelled? }`.",
            "read_file" => "```gx\nread_file(path)\n```\nReturns file contents as a string. Throws if the path doesn't exist or is outside the sandbox.",
            "write_file" => "```gx\nwrite_file(path, content)\n```\nThrows on failure.",
            "json_stringify" => "```gx\njson_stringify(v)\n```\nAliases: `to_json`, `json`.",
            "json_parse" => "```gx\njson_parse(s)\n```\nAlias: `parse_json`.",
            "len" => "```gx\nlen(v)\n```\nLength of a string, array, or object (key count).",
            "type_of" => "```gx\ntype_of(v)\n```\nReturns `\"string\"`|`\"number\"`|`\"boolean\"`|`\"array\"`|`\"object\"`|`\"null\"`|`\"function\"`.",
            "span" => "```gx\nspan(\"name\") { ... }\n```\nManual diagnostics instrumentation — active under `--trace`. Also valid as `span(\"name\"):` in progressive syntax.",
            "db_transaction" => "```gx\ndb_transaction(path) { ... }\n```\nRuns the block atomically; commits on success, rolls back if it throws. Also valid as `db_transaction(path):` in progressive syntax.",
            "ask" => "```gx\nask <provider> { prompt: \"...\", model?: \"...\" }\n```\nCalls an AI provider (`openai`/`anthropic`/`ollama`). Every call is auto-logged to `memory.ai_trace`.",
            "embed" => "```gx\nembed \"text\"\n```\nReturns an `Array` of floats (OpenAI `text-embedding-3-small`).",
            "breakpoint" => "```gx\nbreakpoint()\n```\nPauses execution and opens an interactive debugger prompt — works in any execution context, no flag required. See also `gx debug`/`--break`.",
            "test" => "```gx\ntest(name, fn)\n```\nRegisters a named, isolated test case — run separately (its own fresh assertions) by `gx test` after the top-level script finishes.",
            "before_each" => "```gx\nbefore_each(fn)\n```\nRuns before every `test()` case in this file. Share state with the test body via `memory.*` (closures capture by value).",
            "after_each" => "```gx\nafter_each(fn)\n```\nRuns after every `test()` case in this file, even if the test failed — see `before_each`.",
            "set_random_seed" => "```gx\nset_random_seed(n)\n```\nMakes `random`/`random_int`/`random_choice`/`shuffle` fully deterministic for the rest of the run.",
            "test_temp_dir" => "```gx\ntest_temp_dir()\n```\nReturns a fresh, writable scratch directory — sandbox-respecting, unique on every call.",
            "assert_golden" => "```gx\nassert_golden(actual, path)\n```\nByte-for-byte comparison against a saved file. Missing file, or `GX_UPDATE_GOLDEN=1`: writes `actual` and passes.",
            "config_load" => "```gx\nconfig_load({ defaults?, file?, env_prefix?, overrides?, schema? })\n```\nLayered config: defaults < file < env overrides (type-coerced) < explicit overrides, with optional fail-fast schema validation.",
            "jsonl_parse" => "```gx\njsonl_parse(text)\n```\nOne independent JSON value per line (JSON Lines / NDJSON) — distinct from `json_parse`, which expects the whole text to be one value.",
            "jsonl_stringify" => "```gx\njsonl_stringify(arr)\n```\nOne compact JSON value per line.",
            "versioned_stringify" => "```gx\nversioned_stringify(value, version)\n```\nWraps `value` with a version tag. Pair with `versioned_parse` for fail-loud-on-drift persisted data.",
            "versioned_parse" => "```gx\nversioned_parse(text, expected_version?)\n```\nThrows if the tag doesn't match `expected_version`. Omit it to read the data unconditionally.",
            "data_import" => "```gx\ndata_import(path)\n```\nRead + parse, format detected from the extension (`.json`/`.yaml`/`.yml`/`.toml`/`.csv`/`.xml`/`.jsonl`).",
            "data_export" => "```gx\ndata_export(path, value, schema?)\n```\nStringify + write, format detected from the extension. Optional schema validation fails before writing anything.",
            "render_template" => "```gx\nrender_template(template, data)\n```\n`{{dotted.path}}` substitution against `data`. Load `template` via `read_file` — a GX string literal's own `\"{expr}\"` interpolation would mangle `{{...}}` first.",
            "task_emit" => "```gx\ntask_emit(value)\n```\nCalled from inside a running task to report incremental progress. Drained by the caller via `task_progress(handle)`.",
            "task_progress" => "```gx\ntask_progress(handle)\n```\nDrains every value `task_emit`'d since the last drain. Empty array (not an error) when there's nothing new.",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(value: &serde_json::Value) -> Vec<u8> {
        let body = serde_json::to_vec(value).unwrap();
        let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn read_message_round_trips_a_written_message() {
        let value = serde_json::json!({ "jsonrpc": "2.0", "method": "initialize", "id": 1 });
        let bytes = frame(&value);
        let mut reader = BufReader::new(Cursor::new(bytes));
        let read = read_message(&mut reader).unwrap();
        assert_eq!(read, value);
    }

    #[test]
    fn read_message_returns_none_on_eof() {
        let mut reader = BufReader::new(Cursor::new(Vec::new()));
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn read_message_returns_none_on_a_missing_content_length_header() {
        let mut reader = BufReader::new(Cursor::new(b"\r\n{}".to_vec()));
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn read_message_rejects_an_absurdly_large_content_length() {
        let mut reader = BufReader::new(Cursor::new(
            b"Content-Length: 999999999999\r\n\r\n".to_vec(),
        ));
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn read_message_handles_two_consecutive_frames() {
        let a = serde_json::json!({ "jsonrpc": "2.0", "method": "a" });
        let b = serde_json::json!({ "jsonrpc": "2.0", "method": "b" });
        let mut bytes = frame(&a);
        bytes.extend(frame(&b));
        let mut reader = BufReader::new(Cursor::new(bytes));
        assert_eq!(read_message(&mut reader).unwrap(), a);
        assert_eq!(read_message(&mut reader).unwrap(), b);
        assert!(read_message(&mut reader).is_none());
    }

    fn roundtrip(
        msg: serde_json::Value,
        docs: &mut HashMap<String, String>,
    ) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let _ = handle_message(&msg, docs, &mut out);
        // Parse every frame written into `out` back into JSON values.
        let mut reader = BufReader::new(Cursor::new(out));
        let mut results = Vec::new();
        while let Some(v) = read_message(&mut reader) {
            results.push(v);
        }
        results
    }

    #[test]
    fn initialize_responds_with_server_capabilities() {
        let mut docs = HashMap::new();
        let msg =
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let responses = roundtrip(msg, &mut docs);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(
            responses[0]["result"]["capabilities"]["hoverProvider"],
            true
        );
        assert_eq!(
            responses[0]["result"]["capabilities"]["definitionProvider"],
            true
        );
    }

    #[test]
    fn did_open_a_clean_document_publishes_zero_diagnostics() {
        let mut docs = HashMap::new();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///a.gx", "text": "function f() { return 1 }\n" } },
        });
        let notes = roundtrip(msg, &mut docs);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["method"], "textDocument/publishDiagnostics");
        assert_eq!(
            notes[0]["params"]["diagnostics"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn did_open_a_broken_document_publishes_a_located_diagnostic() {
        let mut docs = HashMap::new();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///a.gx", "text": "function f() { if x > }\n" } },
        });
        let notes = roundtrip(msg, &mut docs);
        let diags = notes[0]["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
        // Line 0 (0-based) — the LSP protocol's own convention, converted
        // from GX's 1-based "Line 1" the parser actually reported.
        assert_eq!(diags[0]["range"]["start"]["line"], 0);
        assert!(!diags[0]["message"].as_str().unwrap().is_empty());
    }

    #[test]
    fn did_change_replaces_the_document_and_republishes_diagnostics() {
        let mut docs = HashMap::new();
        docs.insert(
            "file:///a.gx".to_string(),
            "function f() { if x > }\n".to_string(),
        );
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "contentChanges": [{ "text": "function f() { return 1 }\n" }],
            },
        });
        let notes = roundtrip(msg, &mut docs);
        assert_eq!(
            notes[0]["params"]["diagnostics"].as_array().unwrap().len(),
            0
        );
        assert_eq!(docs["file:///a.gx"], "function f() { return 1 }\n");
    }

    #[test]
    fn did_close_removes_the_document() {
        let mut docs = HashMap::new();
        docs.insert("file:///a.gx".to_string(), "x = 1\n".to_string());
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///a.gx" } },
        });
        roundtrip(msg, &mut docs);
        assert!(!docs.contains_key("file:///a.gx"));
    }

    #[test]
    fn hover_on_a_known_builtin_returns_its_signature() {
        let mut docs = HashMap::new();
        docs.insert(
            "file:///a.gx".to_string(),
            "x = retry(fn() { return 1 }, 3)\n".to_string(),
        );
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "position": { "line": 0, "character": 6 }, // inside "retry"
            },
        });
        let responses = roundtrip(msg, &mut docs);
        let hover = &responses[0]["result"];
        assert!(hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("retry(fn"));
    }

    #[test]
    fn hover_on_whitespace_returns_null() {
        let mut docs = HashMap::new();
        docs.insert("file:///a.gx".to_string(), "x = 1\n".to_string());
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "position": { "line": 0, "character": 1 }, // the space after "x"
            },
        });
        let responses = roundtrip(msg, &mut docs);
        assert_eq!(responses[0]["result"], serde_json::Value::Null);
    }

    #[test]
    fn hover_on_a_user_defined_function_shows_its_signature() {
        let mut docs = HashMap::new();
        docs.insert(
            "file:///a.gx".to_string(),
            "function double(n) { return n * 2 }\nx = double(5)\n".to_string(),
        );
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "position": { "line": 1, "character": 5 }, // inside "double(5)"
            },
        });
        let responses = roundtrip(msg, &mut docs);
        let value = responses[0]["result"]["contents"]["value"]
            .as_str()
            .unwrap();
        assert!(value.contains("double(n)"));
    }

    #[test]
    fn definition_on_a_call_jumps_to_the_function_declaration() {
        let mut docs = HashMap::new();
        docs.insert(
            "file:///a.gx".to_string(),
            "function double(n) { return n * 2 }\nx = double(5)\n".to_string(),
        );
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "position": { "line": 1, "character": 5 },
            },
        });
        let responses = roundtrip(msg, &mut docs);
        let result = &responses[0]["result"];
        assert_eq!(result["uri"], "file:///a.gx");
        assert_eq!(result["range"]["start"]["line"], 0);
    }

    #[test]
    fn definition_on_an_unknown_identifier_returns_null() {
        let mut docs = HashMap::new();
        docs.insert(
            "file:///a.gx".to_string(),
            "x = does_not_exist(5)\n".to_string(),
        );
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": "file:///a.gx" },
                "position": { "line": 0, "character": 6 },
            },
        });
        let responses = roundtrip(msg, &mut docs);
        assert_eq!(responses[0]["result"], serde_json::Value::Null);
    }

    #[test]
    fn shutdown_responds_and_exit_stops_the_loop() {
        let mut docs = HashMap::new();
        let shutdown = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "shutdown" });
        let responses = roundtrip(shutdown, &mut docs);
        assert_eq!(responses[0]["result"], serde_json::Value::Null);

        let exit = serde_json::json!({ "jsonrpc": "2.0", "method": "exit" });
        let mut out = Vec::new();
        let should_continue = handle_message(&exit, &mut docs, &mut out);
        assert!(should_continue.is_none());
    }

    #[test]
    fn an_unknown_request_gets_a_method_not_found_error_not_silence() {
        let mut docs = HashMap::new();
        let msg = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "textDocument/totallyMadeUp" });
        let responses = roundtrip(msg, &mut docs);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["error"]["code"], -32601);
    }

    #[test]
    fn an_unknown_notification_is_silently_ignored() {
        let mut docs = HashMap::new();
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": "some/futureNotification" });
        let responses = roundtrip(msg, &mut docs);
        assert!(responses.is_empty());
    }
}
