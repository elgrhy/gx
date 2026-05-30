# GX Language

> Brain-first programming language for building transparent, auditable AI assistants.

[![npm version](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![Crates.io](https://img.shields.io/crates/v/gx)](https://crates.io/crates/gx)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://github.com/elgrhy/gx)

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust. No cloud lock-in.

## Install

```bash
npm install -g gxlang
gx --version   # gx 0.4.0
```

Downloads the correct native binary for your platform (macOS arm64/x64, Linux x64/arm64, Windows x64). No Rust required.

## Quick Start

```bash
gx init my-agent
cd my-agent
gx run main.gx
```

## What's New in v0.4.0

The biggest GX release — now genuinely competitive with Python for agent-building.

| Feature | Example |
|---------|---------|
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
import "utils/data.gx" as data

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

    // Validate incoming request
    spec = { query: "string", customer_id: "number" }
    check = schema_validate(input, spec)
    if !check.ok {
      say "Invalid request: " + check.errors[0]
      return
    }

    // Ask AI with tool use
    response = ask openai {
      prompt:  input.query,
      tools:   [lookup_customer],
      model:   "gpt-4o",
      stream:  true
    }

    persist_memory()
    say response.text
  }
}
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

`ask` · `embed` · `http_get/post/put/delete` · `read_file` · `write_file` · `json_stringify/parse` · `csv_parse/stringify` · `yaml_parse/stringify` · `toml_parse/stringify` · `regex_test/find/find_all/replace/split/captures` · `date_now/parse/format/diff/add/parts` · `vector_store_new/add/search` · `cosine_similarity` · `schema_validate` · `persist_memory` · `load_memory` · `load_env` · `get_env` · `retry` · `trace_log` · `await {}` · `db_query/exec` · `base64_encode/decode` · `readline` · `shell` · and more

## v0.3.0 Features Still in v0.4.0

- Module system: `import "file.gx" as alias`
- `!` (not) operator: `if !result.ok { ... }`
- Range slicing: `text[0..5]`, `arr[1..4]`
- Integer JSON: `json_stringify({n: 50})` → `"n":50` (not `50.0`)
- Standalone `slice()` / `merge()`
- `output` usable as a plain variable name
- `readline()` for stdin
- `{{ }}` brace escaping in strings

## Full Documentation

[github.com/elgrhy/gx](https://github.com/elgrhy/gx)

## License

MIT — © 2025 Ahmed Elgarhy / DEVJSX LIMITED (London, UK)
