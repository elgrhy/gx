# GX Language

> Brain-first programming language for building transparent, auditable AI assistants.

[![npm version](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![Crates.io](https://img.shields.io/crates/v/gxlang)](https://crates.io/crates/gxlang)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://github.com/elgrhy/gx)

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust. No cloud lock-in.

## Install

```bash
npm install -g gxlang
gx --version   # gx 0.5.0
```

Downloads the correct native binary for your platform (macOS arm64/x64, Linux x64/arm64, Windows x64). No Rust required.

## Quick Start

```bash
gx init my-agent
cd my-agent
gx run main.gx
```

## What's New in v0.5.0

| Feature | Example |
|---|---|
| **Inline eval** | `gx -e 'say sha256("hello")'` |
| **SHA-256** | `sha256("text")` → 64-char hex |
| **UUID v4** | `uuid()` → `"f47ac10b-..."` |
| **Path helpers** | `dirname`, `basename`, `path_join` |
| **Glob** | `glob("src/**/*.gx")` |
| **URL parsing** | `url_parse(url).host` |
| **Group by** | `group_by(rows, "dept")` |
| **Truncate** | `truncate("hello world", 8)` → `"hello w…"` |
| **Token count** | `token_count(text)`, `tokens_used()` |
| **Inline write** | `write("Loading...")` — no trailing newline |
| **Stdlib namespace** | `use std.crypto`, `use std.fs`, `use std.net` |

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
