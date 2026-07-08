# GX Language

**Brain-first programming language for building transparent, auditable AI assistants.**

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully debuggable. Built in Rust. Runs anywhere. No cloud lock-in.

[![Crates.io](https://img.shields.io/crates/v/gxlang)](https://crates.io/crates/gxlang)
[![npm](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![CI](https://github.com/elgrhy/gx/actions/workflows/ci.yml/badge.svg)](https://github.com/elgrhy/gx/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Install

```bash
# macOS / Linux — one-line installer
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm (any platform with Node.js 16+)
npm install -g gxlang

# Cargo
cargo install gxlang

# From source
git clone https://github.com/elgrhy/gx.git && cd gx && cargo build --release
```

```bash
gx --version   # gx 0.5.1
```

---

## Quick Start

```bash
gx init my-agent
cd my-agent
gx run main.gx
```

```gx
agent "hello" {
  when started {
    name = "World"
    say "Hello, {name}! GX v0.5.1 is running."
  }
}
```

---

## What's New (Unreleased)

### Capability Runtime — one place that authorizes everything dangerous

Every GX subsystem that touches something outside the interpreter's own
memory — filesystem, process execution, shell, HTTP client/server, SQLite,
AI providers, environment variables, and every bridge (`js`/`ts`/`py`/
`binary`/`go`/`rust_bin`) — now authorizes through a single
`Capabilities` value instead of each subsystem implementing its own check.
This closed several real gaps: AI provider calls, `env`/`get_env`/`set_env`,
`serve on port ...`, SQLite access, and the `ts`/`binary`/`go`/`rust_bin`
bridges had **no gate at all** before this — `use binary "/any/path"` would
spawn an arbitrary executable regardless of `--allow-process`. None of that
is deny-by-default now (existing scripts keep working unchanged), but all of
it is now restrictable via `gx.json`'s new `capabilities` section.

```json
{
  "dependencies": { "process": ["git"], "ai": ["anthropic"] },
  "capabilities": { "http_server": false, "env_deny": ["AWS_SECRET_ACCESS_KEY"] }
}
```

Also fixed: spawned agents (`spawn agent ... timeout N`, `parallel { ... }`)
used to run on a fresh, all-denied `Interpreter` regardless of what the
parent script had been granted — a multi-agent program couldn't use
`process_run` from inside a spawned agent even with `--allow-process` at the
top level. Capabilities now inherit correctly. A new `--deny <resource>`
CLI flag lets whoever invokes `gx` force-deny a resource regardless of what
the script or its manifest grants, and `gx build` now accepts the same
`--allow-*`/`--deny` flags and bakes them into the generated launcher, since
a distributed binary's end user has no way to pass them itself. See
[Capability Runtime](docs/language_reference.md#capability-runtime) for the
full model and migration notes.

### Native Process Runtime — the recommended replacement for `shell()`

`process_run`/`process_spawn` (+ `process_wait`/`process_kill`/`process_exists`/
`process_status`/`process_read`) run external programs via a real argument
array — `Command::new(command).args(args)` — never through a shell. No
quoting dialect, no shell-injection surface, consistent behavior across
Linux/macOS/Windows. Every spawned process is owned by the runtime for its
full lifetime: reaped the instant it exits (no zombies), killed automatically
if the program exits first (no orphans), with an optional `timeout` and true
concurrent execution.

```gx
result = process_run({ command: "git", args: ["status", "--short"], cwd: repo_dir })
if result.ok { say result.stdout } else { say "failed: {result.error_kind}" }
```

Gated by its own `--allow-process` flag, independent of `--allow-shell` — an
app can allow structured process execution while fully disabling shell
string execution. `gx.json`'s `dependencies.process: [...]` restricts which
executables may run, the same allowlist already used for JS/Python bridge
modules. `shell()` remains fully supported for cases that genuinely need
shell syntax (pipes, redirects, globs) — see
[Process Runtime](docs/language_reference.md#process-runtime-recommended)
for the full migration guide.

As part of this work, the JS and TypeScript bridges (`use js.X`/`use ts.X`)
were fixed: both now run through a persistent, JSON-IPC process the same way
the Python bridge already did, instead of a fragile per-call invocation that
— in both cases — would hang forever the moment it was actually exercised.
No GX-facing API changed.

Output is never silently lost, either: every result carries `truncated`/
`stdout_bytes`/`stderr_bytes` so a process that produces more than the 32 MiB
retention cap is detectable, not silently incomplete. And `process_status`
now reports `cwd`, `exit_reason`, and `stdin_bytes` alongside `pid`/timing —
enough to log a process's full lifecycle without exposing `env`/`stdin`
content.

### Crypto primitives

`hmac_sha256`/`hmac_sha512`, `secure_compare` (constant-time), `secure_random`,
`ed25519_generate_keypair`/`ed25519_sign`/`ed25519_verify`, and HS256-only
`jwt_sign`/`jwt_verify` — all cryptographic math delegated to audited crates
(`hmac`, `ed25519-dalek`, `jsonwebtoken`, `subtle`, `getrandom`), with
production input-size limits and a 32-byte minimum JWT secret. See the
Crypto section in [docs/language_reference.md](docs/language_reference.md)
for HMAC/JWT/Ed25519 webhook-verification examples (Slack, Discord, generic
HMAC).

---

## What's New in v0.5.1

Production-readiness patch based on real-world feedback from the GClaw agentic system.

- **`db_transaction(path) { body }`** — native SQLite transactions with automatic COMMIT/ROLLBACK; `db` variable exposed inside the block for parameterised queries
- **`db_exec` / `db_query` array params** — `db_exec(db, sql, [p1, p2])` now works alongside the existing spread form
- **`sleep(n)` takes seconds** — `sleep(5)` = 5 s; use `sleep(0.5)` or `sleep(500ms)` for sub-second delays
- **Duration suffix literals** — `500ms` → 0.5 s, `5s` → 5 s; works in any expression including function arguments
- **Regex quantifier fix** — `{1,6}` patterns in string literals are no longer incorrectly interpolated; only complete, valid expressions inside `{…}` are substituted

> **Upgrading from v0.5.0:** `sleep()` now takes **seconds** instead of milliseconds. Replace `sleep(5000)` with `sleep(5)`.

---

## What's New in v0.5.0

Developer experience improvements, a stdlib namespace, and crypto/filesystem builtins.

### Inline Eval — No File Needed

```bash
gx -e 'say "Hello from GX"'
gx -e 'say sha256("abc")'
gx -e 'say uuid()'
```

Run any GX snippet directly from the terminal without creating a file.

### Crypto

```gx
hash = sha256("hello world")          // 64-char hex SHA-256
id   = uuid()                         // "f47ac10b-58cc-4372-a567-0e02b2c3d479"
```

### File System Helpers

```gx
dir  = dirname("/home/user/report.txt")   // "/home/user"
file = basename("/home/user/report.txt")  // "report.txt"
path = path_join("data", "2024", "q1.csv") // "data/2024/q1.csv"
hits = glob("reports/*.txt")             // ["reports/jan.txt", "reports/feb.txt"]
```

### Token Awareness

```gx
n    = token_count("some text")    // heuristic: ~4 chars per token
used = tokens_used()               // cumulative tokens across all ask calls this run
```

### URL Parsing

```gx
u = url_parse("https://api.example.com:8080/v1/search?q=gx#top")
u.scheme   // "https"
u.host     // "api.example.com"
u.port     // "8080"
u.path     // "/v1/search"
u.query    // "q=gx"
u.fragment // "top"
```

### Data Helpers

```gx
// Group an array of objects by a field
rows = [
  { name: "Alice", dept: "eng" },
  { name: "Bob",   dept: "eng" },
  { name: "Carol", dept: "hr" }
]
by_dept = group_by(rows, "dept")
// { "eng": [{name:"Alice",...},{name:"Bob",...}], "hr": [{name:"Carol",...}] }

// Truncate with optional custom ellipsis
truncate("hello world", 8)           // "hello w…"
truncate("hello world", 8, "...")    // "hello..."
```

### Inline Output (No Newline)

```gx
write("Loading")
for n in range(1, 4) {
  write(".")
}
say ""   // newline at the end
// prints: Loading...
```

### Optional Stdlib Namespace

```gx
use std.crypto
use std.fs
use std.net
use std.collections
```

These are optional imports — all functions are already available globally. Use them for clarity in larger programs.

---

## What's New in v0.4.0

### Regex — Full Pattern Matching

```gx
price  = regex_find("Total: $42.50", "\\$([0-9.]+)")       // "42.50"
scores = regex_find_all("85, 92, 100", "\\d+")              // ["85","92","100"]
valid  = regex_test(email, "@")
clean  = regex_replace("hello   world", "\\s+", " ")
parts  = regex_split("a::b::c", ":+")                       // ["a","b","c"]
caps   = regex_captures("2024-01-15", "(\\d{{4}})-(\\d{{2}})-(\\d{{2}})")
// caps[1]="2024", caps[2]="01", caps[3]="15"
```

### Date / Time

```gx
now   = date_now()                          // "2026-06-01T09:00:00Z"
ts    = date_parse("2024-01-15")            // Unix timestamp
fmt   = date_format(ts, "%B %d, %Y")        // "January 15, 2024"
diff  = date_diff(ts, date_now(), "days")
next  = date_add(ts, 7, "days")
parts = date_parts(ts)  // { year, month, day, hour, minute, second, weekday }
```

### CSV, YAML, TOML

```gx
rows     = csv_parse(read_file("data.csv"))
config   = yaml_parse(read_file("config.yml"))
manifest = toml_parse(read_file("Cargo.toml"))
```

### .env File Loading

```gx
load_env(".env")                              // sandboxed — stays within script dir
key = get_env("OPENAI_API_KEY", "")
```

### Language Bridges — TypeScript, Go, Any Binary

```gx
use ts.analytics
result = ts.analytics.process(events)

use binary "./my_rust_service"
output = binary.transform(payload)

use py.pandas
df = py.pandas.read_csv("data.csv")
```

### AI Tool Use — Function Calling

```gx
tool "search_web" {
  description: "Search the web for current information"
  params: { query: { type: "string", required: true } }
  execute(query) {
    result = http_get("https://api.search.example.com?q={query}")
    return result.data
  }
}

agent "researcher" {
  when started {
    response = ask openai {
      prompt: "What is the current Bitcoin price?",
      tools:  [search_web],
      model:  "gpt-4o"
    }
    say response.text
  }
}
```

### Streaming AI

```gx
result = ask openai {
  prompt: "Write a 500-word essay on AI transparency",
  stream: true
}
```

### Persistent Memory — Survives Restarts

```gx
agent "counter" {
  remember { count = 0 }
  when started {
    load_memory()
    memory.count += 1
    log("Run #{memory.count}")
    persist_memory()
  }
}
```

### Vector Store — Semantic Search

```gx
store = vector_store_new("docs")
vector_store_add(store, "doc1", embed("The cat sat on the mat"), "cat story")
hits  = vector_store_search(store, embed("feline pets"), 3)
log(hits[0].label)   // "cat story"
```

### Await Block — Concurrent I/O

```gx
await {
  weather: http_get("https://api.weather.com/london"),
  news:    http_get("https://api.news.com/top"),
  stocks:  http_get("https://api.stocks.com/AAPL")
} into data
```

### Retry with Backoff

```gx
result = retry(fn() {
  return ask openai { prompt: "Classify this text" }
}, 5, { delay: 1000, backoff: "exponential" })
```

### Observability — Structured JSONL Tracing

```gx
trace_log("pipeline.start", { query: memory.query })
result = ask anthropic { prompt: memory.query }
trace_log("ai.done", { tokens: result.tokens_used })
```

---

## Core Language

### Agent Structure

```gx
agent "my_agent" {
  goal: "Do something useful"
  retry: 3
  timeout: 30s
  on_error: escalate

  remember {
    count = 0
    items = []
  }

  when started {
    memory.count += 1
    say "Run #{memory.count}"
  }

  when memory.count > 100 {
    memory.count = 0
    log("Reset counter")
  }

  when cron "0 9 * * 1-5" {
    log("Good morning, it's a weekday")
  }
}
```

### Functions — Three Scopes

```gx
// 1. File-root — available to all agents
function format_price(amount) {
  return "$" + to_string(round(amount * 100) / 100)
}

agent "shop" {
  // 2. Agent-level — available across all when blocks
  function validate_order(order) {
    return schema_validate(order, { item: "string", qty: "number" })
  }

  when started {
    // 3. Inline — scoped to this block
    function greet(name) { return "Hello, {name}!" }
    log(greet("Alice"))
  }
}
```

### Module System

```gx
import "utils/math.gx"    as math
import "utils/strings.gx" as str

result = math.add(10, 32)          // 42
label  = str.truncate(result, 5)   // "42..."
```

### Range Slicing

```gx
s = "hello world"
s[0..5]    // "hello"
s[6..11]   // "world"

arr = [1, 2, 3, 4, 5]
arr[1..4]  // [2, 3, 4]
```

### String Interpolation

```gx
name = "GX"
"Hello from {name}!"          // "Hello from GX!"
"Literal brace: {{name}}"     // "Literal brace: {name}"
"Result: {1 + 2 * 3}"         // "Result: 7"
```

### Control Flow

```gx
// Range for loop
for n in range(1, 11) { log(n) }

// While with break/continue
while running {
  line = readline()
  if line == null { break }
  if line.starts_with("#") { continue }
  process(line)
}

// Try/catch
try {
  result = http_post(url, payload)
} catch NetworkError e {
  log("Network: " + e)
} catch e {
  log("Other: " + e)
}

// Await — concurrent branches
await { a: expr1, b: expr2 } into results

// Retry with backoff
result = retry(fn() { risky_call() }, 3, { backoff: "exponential" })
```

---

## Built-in Reference

| Category | Builtins |
|---|---|
| **AI** | `ask openai/anthropic/ollama` (streaming + tool use), `embed`, `infer classifier` |
| **Token** | `token_count(str)`, `tokens_used()` |
| **HTTP** | `http_get`, `http_post`, `http_put`, `http_delete`, `http_stream`, `http_upload` |
| **File I/O** | `read_file`, `write_file`, `append_file`, `delete_file`, `file_exists`, `list_dir`, `make_dir` |
| **Path** | `dirname`, `basename`, `path_join`, `glob` |
| **JSON** | `json_stringify` (integers stay integers), `json_parse` |
| **CSV** | `csv_parse` (auto-types), `csv_stringify` |
| **YAML** | `yaml_parse`, `yaml_stringify` |
| **TOML** | `toml_parse`, `toml_stringify` |
| **Regex** | `regex_test`, `regex_find`, `regex_find_all`, `regex_replace`, `regex_split`, `regex_captures`, `regex_named_captures` |
| **Date** | `date_now`, `date_timestamp`, `date_parse`, `date_format`, `date_diff`, `date_add`, `date_parts`, `date_from_parts` |
| **Math** | `abs`, `floor`, `ceil`, `round`, `sqrt`, `pow`, `min`, `max`, `clamp`, `random`, `pi`, `e` |
| **String** | `.trim()`, `.upper()`, `.lower()`, `.contains()`, `.replace()`, `.split()`, `.reverse()`, `.slice()`, `.starts_with()`, `.ends_with()`, `.len()`, `.repeat()`, `.pad_left()`, `.pad_right()` |
| **Array** | `.sort()`, `.filter_by()`, `.unique()`, `.flatten()`, `.sum()`, `.min()`, `.max()`, `.average()`, `.take()`, `.skip()`, `.map_field()` |
| **Object** | `keys`, `values`, `entries`, `merge`, `has`, `group_by` |
| **Crypto** | `sha256`, `uuid` / `uuid_v4` |
| **Net** | `url_parse` |
| **DB** | `db_query`, `db_exec` (SQLite, bundled) |
| **Env** | `load_env`, `get_env(key, default)`, `set_env` |
| **Shell** | `shell()` (requires `--allow-shell`) |
| **Vector** | `vector_store_new`, `vector_store_add`, `vector_store_search`, `vector_store_delete`, `vector_store_size`, `cosine_similarity` |
| **Schema** | `schema_validate(value, spec)` |
| **Memory** | `persist_memory()`, `load_memory()` |
| **Base64** | `base64_encode`, `base64_decode` |
| **I/O** | `readline()`, `read_all()`, `write()` (no trailing newline) |
| **Util** | `truncate`, `type_of`, `is_null`, `to_string`, `to_number`, `len` |
| **Observability** | `trace_log(event, data)` |
| **Retry** | `retry(fn, max, opts)` |

---

## Language Interop

| Language | Syntax | How it works |
|---|---|---|
| JavaScript | `use js.axios` | Persistent Node.js subprocess, JSON IPC |
| TypeScript | `use ts.mylib` | Auto-detects `tsx` or `ts-node` |
| Python | `use py.requests` | Persistent Python subprocess, JSON IPC |
| Go | `use go "./service"` | Compiled binary, JSON stdin/stdout |
| Any binary | `use binary "./app"` | Same JSON protocol — Rust, Java, .NET, C++ |

---

## Progressive Syntax

Three levels that compile to the same runtime:

```gx
// Level 1 — Pure intent
Agent greeter
name = "World"
"Hello {name}"

// Level 2 — Named behaviors
Agent assistant
On start:
  Greet

Greet:
  say "Hello!"

// Level 3 — Explicit brain cycle
Agent processor
Plan:
  action = "process"
Execute:
  If action == "process"
    result = transform(input)
Remember:
  memory.last = result
Communicate:
  result
```

---

## AI Providers

| Provider | Env Variable | Default Model |
|---|---|---|
| OpenAI | `OPENAI_API_KEY` | `gpt-4o-mini` |
| Anthropic | `ANTHROPIC_API_KEY` | `claude-sonnet-4-6` |
| Ollama (local) | `OLLAMA_URL` (default `localhost:11434`) | `llama3` |

---

## Security Model

GX is secure by default — all dangerous operations require explicit opt-in:

| Operation | Default | Flag to enable |
|---|---|---|
| Shell execution | Blocked | `--allow-shell` |
| Internal HTTP (SSRF) | Blocked | `--allow-internal-http` |
| File access | Sandboxed to script dir | `--no-sandbox` |

---

## Toolchain

```bash
gx run main.gx              # Run a GX program
gx -e 'say "hello"'         # Run inline source
gx check main.gx            # Syntax check (no execution)
gx init my-project          # Scaffold a new project
gx test                     # Run all *.test.gx files
gx fmt main.gx              # Format source
gx build                    # Compile for distribution
gx install js.axios         # Add JS dependency
gx install py.requests      # Add Python dependency
gx repl                     # Interactive REPL
```

---

## Version History

| Version | Highlights |
|---|---|
| **v0.5.1** | **Production patch** — `db_transaction` block · `sleep(seconds)` · `500ms`/`5s` duration literals · array-form `db_exec(db, sql, [params])` · regex quantifier interpolation fix |
| **v0.5.0** | **DX + stdlib** — `gx -e` inline eval · `sha256`, `uuid` · `glob`, `dirname`, `basename`, `path_join` · `url_parse` · `group_by` · `truncate` · `token_count`, `tokens_used` · `write` (no newline) · `use std.fs\|crypto\|net\|collections` · `load_env` sandbox fix |
| **v0.4.2** | **Source-located runtime errors** — every runtime error now reports `at line N` plus a call-stack trace. Published to crates.io as `gxlang`. |
| **v0.4.1** | **Real closures** · top-level statements · `is_tty()` · `--no-limit` flag · `assert_eq`/`assert_true`/`assert_contains` · `gx test` discovery · "Did you mean?" suggestions |
| **v0.4.0** | Regex · Date/Time · CSV/YAML/TOML · TypeScript+Go+Binary bridges · AI tool use · Streaming AI · Persistent memory (SQLite) · Vector store · Schema validation · `await {}` · Retry with backoff · Observability tracing |
| **v0.3.0** | Security audit · Sandbox & SSRF protection · Shell gate · Module system · `!` operator · Integer JSON · Range slicing · `readline()` |
| **v0.2.5** | HTTPS/TLS fix · Shell stdin · Quoted object keys |
| **v0.2.0** | `think`/`act`/`observe` · `parallel {}` · `retry:` · Multi-agent orchestration |
| **v0.1.0** | Initial release — lexer, parser, interpreter, OpenAI/Anthropic/Ollama |

---

## Contributing

```bash
cargo test                       # 82+ unit tests — must all pass
gx test                          # 16 integration test files, 106+ assertions
cargo clippy -- -D warnings      # zero warnings
cargo fmt --check                # formatted
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

## License

MIT — © 2026 Ahmed Elgarhy / DEVJSX LIMITED (London, UK). Company No: 16618207.
