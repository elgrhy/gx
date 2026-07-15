# GX Language

> Brain-first programming language for building transparent, auditable AI assistants.

[![npm version](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![Crates.io](https://img.shields.io/crates/v/gxlang)](https://crates.io/crates/gxlang)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://github.com/elgrhy/gx)

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust. No cloud lock-in.

GX stays brain-first, but it's a general-purpose language for production
applications, not just prototyping agents: a capability-sandboxed runtime,
native Process/HTTP/Database/Crypto/Task/AI Context runtimes, a package
manager with lockfiles, a testing framework, a debugger, a REPL, and more —
see the [full documentation](https://github.com/elgrhy/gx#readme) for the
complete picture.

## Install

```bash
npm install -g gxlang
gx --version   # gx 0.7.0
```

Downloads the correct native binary for your platform (macOS arm64/x64, Linux x64/arm64, Windows x64). No Rust required.

## Quick Start

```bash
gx init my-agent
cd my-agent
gx run main.gx
```

## What's New in v0.7.0

Findings from building a large GX application (~6,000 lines, 34 files, 17
agents) end-to-end on v0.6.1. Backward compatible. Full detail in
[CHANGELOG.md](https://github.com/elgrhy/gx/blob/main/CHANGELOG.md).

- Top-level constants and named functions are now real, visible values from
  anywhere — a top-level `NAME = value` is visible inside every function,
  and a bare reference to a named function works like `fn(){}` instead of
  silently evaluating to `null`
- A brace-syntax file using `agent` as a variable name (or text shaped like
  `"On error:"`) no longer silently misroutes to the indentation parser
  and produces an empty program
- String interpolation with a single-quoted string argument now evaluates
  (`"{arr.join(', ')}"`) — single-quoted string literals are supported
- `&&`/`||` now work, as documented; `argv()`/`script_args()` give scripts
  real command-line arguments; `--project-sandbox` supports multi-directory
  projects; `date_add_iso()` is the safe alternative to `date_add`
- `gx check` warns on a discarded bare-identifier statement and actually
  checks every file argument given, not just the first
- `gx fmt` fixes a real non-idempotency bug and uses denser, more
  conventional call-site spacing

## What's New in v0.6.1

Production-hardening patch based on real-world feedback migrating a large GX
application (~55 files, 23 agents, 8 bridge integrations) onto v0.6.0.
Backward compatible. Full detail in
[CHANGELOG.md](https://github.com/elgrhy/gx/blob/main/CHANGELOG.md).

- `spawn agent` now requires a callable `brain{}` — a clear error instead of
  a silent `null` when the target only exposes async `when message` handlers
- Fire-and-forget `spawn "event" to "agent"` now fails clearly on an
  undeclared target instead of silently queuing an undeliverable message
- `http_post`/`http_put` no longer double-encode a pre-stringified body
- `gx check` runs real whole-project static diagnostics, not just a parse check
- `ask ollama` honors `timeout` via a pooled connection; `context_ask`
  supports Ollama

## What's New in v0.6.0

| Runtime | Example |
|---|---|
| **Capability Runtime** | Filesystem/process/shell/network/database/AI-provider access sandboxed by default — `gx run --allow-process --allow-shell` |
| **Process** | `process_run({ command: "git", args: ["status"] })` — no shell, no injection surface |
| **HTTP** | `serve on port 3000 { route GET "/health" { respond json { ok: true } } }`, SSRF-defended client |
| **Database** | `db_transaction(path) { db_exec(db, "...", [...]) }` — pooled connections, real transactions |
| **Task** | `task_spawn(fn() { ... }, { timeout: 30s })`, cancellable, bounded concurrency |
| **Testing / Debugger / REPL** | `gx test`, `breakpoint()` + `gx debug`, `gx repl` with persistent state |
| **Package Runtime** | `gx install`, lockfile-pinned dependencies (`gx.lock`) |

Full details, including this release's security and reliability hardening,
in the [CHANGELOG](https://github.com/elgrhy/gx/blob/main/CHANGELOG.md) and
[full README](https://github.com/elgrhy/gx#readme).

## v0.4.0 Features

| Feature | Example |
|---|---|
| **Regex** | `regex_find(text, "\\$([0-9.]+)")` |
| **Date/Time** | `date_diff(date_parse("2024-01-01"), date_now(), "days")` |
| **CSV/YAML/TOML** | `csv_parse(read_file("data.csv"))` |
| **.env loading** | `load_env(".env")` · `get_env("KEY", "default")` |
| **TypeScript bridge** | `use ts.mylib` → calls ts-node/tsx automatically |
| **Go/Binary bridge** | `use binary "./my_go_service"` |
| **AI tool use** | `ask openai { prompt: "...", tools: [my_tool] }` |
| **Streaming AI** | `ask openai { prompt: "...", stream: true }` |
| **Persistent memory** | `persist_memory()` / `load_memory()` → SQLite |
| **Vector store** | `vector_store_search(store, embed(query), 5)` |
| **Schema validation** | `schema_validate(data, { name: "string" })` |
| **Await block** | `await { a: http_get(url1), b: http_get(url2) } into results` |
| **Retry backoff** | `retry(fn() { risky_call() }, 5, { backoff: "exponential" })` |
| **Observability** | `trace_log("event", { data: value })` → JSONL to stderr |

## Real Agent Example

```gx
tool "lookup_customer" {
  description: "Look up customer by ID"
  execute(customer_id) {
    return http_get("https://api.example.com/customers/{customer_id}").data
  }
}

agent "support_bot" {
  remember { session_count = 0 }

  when started {
    load_env(".env")
    load_memory()
    memory.session_count += 1

    spec = { query: "string", customer_id: "number" }
    check = schema_validate(input, spec)
    if !check.ok {
      say "Invalid request: " + check.errors[0]
      return
    }

    response = ask openai {
      prompt:  input.query,
      tools:   [lookup_customer],
      model:   "gpt-4o",
      stream:  true
    }

    // Token tracking
    say "Tokens used this session: {tokens_used()}"
    persist_memory()
    say response.text
  }
}
```

## Inline Scripting

```bash
# Quick one-liners — no file needed
gx -e 'say sha256("hello world")'
gx -e 'say uuid()'
gx -e 'say url_parse("https://example.com:8080/api?q=1").port'
gx -e 'say token_count("how many tokens is this?")'
```

## Language Interop

GX can call into any language via subprocess bridges:

```gx
use js.axios          // Node.js
use ts.analytics      // TypeScript (auto-detects tsx or ts-node)
use py.pandas         // Python
use binary "./svc"    // Any compiled binary (Go, Rust, Java, .NET)
use go "./service"    // Go binary with JSON protocol
```

## All Built-ins

`ask` · `embed` · `sha256` · `uuid` · `token_count` · `tokens_used` · `http_get/post/put/delete` · `read_file` · `write_file` · `dirname` · `basename` · `path_join` · `glob` · `json_stringify/parse` · `csv_parse/stringify` · `yaml_parse/stringify` · `toml_parse/stringify` · `regex_test/find/find_all/replace/split/captures` · `date_now/parse/format/diff/add/parts` · `vector_store_new/add/search` · `cosine_similarity` · `schema_validate` · `persist_memory` · `load_memory` · `load_env` · `get_env` · `retry` · `trace_log` · `await {}` · `db_query/exec` · `base64_encode/decode` · `readline` · `write` · `truncate` · `url_parse` · `group_by` · `shell` · and more

## Security Model

GX is secure by default:

| Operation | Default | Flag to enable |
|---|---|---|
| Shell execution | Blocked | `--allow-shell` |
| Internal HTTP (SSRF) | Blocked | `--allow-internal-http` |
| File access | Sandboxed to script dir | `--no-sandbox` |

## Full Documentation

[github.com/elgrhy/gx](https://github.com/elgrhy/gx)

## License

MIT — © 2026 Ahmed Elgarhy / DEVJSX LIMITED (London, UK)
