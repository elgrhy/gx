# GX Language

**Brain-first programming language for building transparent, auditable AI assistants.**

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully debuggable. Built in Rust. Runs anywhere. No cloud lock-in.

GX stays brain-first, but it's a general-purpose language for shipping real
production applications, not just prototyping agents: a capability-sandboxed
runtime gates filesystem/process/shell/network/database/AI-provider access by
default; native Process, HTTP (client + server, with SSRF defenses),
Database (pooled SQLite, transactions/savepoints), Crypto, Task
(cancellable, bounded concurrency), and AI Context runtimes; a Module &
Package Runtime with lockfile-pinned dependencies; a production standard
library; and a full developer-tooling surface — a testing framework, an
interactive debugger, a REPL, a Configuration Runtime, Serialization and
Template runtimes, an LSP, a formatter, and documentation generation. Every
runtime is reachable from both progressive (indentation) and classic
(brace) syntax.

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
gx --version   # gx 0.7.0
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
    say "Hello, {name}! GX v0.7.0 is running."
  }
}
```

---

## What's New in v0.7.0

Findings from building AgentX (~6,000 lines, 34 files, 17 agents) end-to-end
on v0.6.1, plus a full engineering review of the underlying architectural
causes (see `docs/language-review-agentx-feedback-2026-07.md`). Backward
compatible — every change is additive or fixes behavior nothing could have
intentionally depended on. Full detail in [CHANGELOG.md](CHANGELOG.md).

- **A brace-syntax file using `agent` as a variable name (or any text shaped
  like `"On error:"`) no longer silently misroutes to the indentation
  parser** — syntax-mode detection now only checks the file's first line,
  where every real progressive-syntax header actually lives, instead of
  scanning the whole file.
- **Top-level constants and named functions are now real, visible values
  from anywhere** — a top-level `NAME = value` is visible inside every
  function (previously silently read as `null`), and a bare reference to a
  named function (`task_spawn(my_func)`) now works exactly like `fn(){}`
  instead of silently evaluating to `null`.
- **String interpolation with a single-quoted string argument now
  evaluates** (`"{arr.join(', ')}"`) — GX gained single-quoted string
  literal support, which is what this needed.
- **`&&`/`||`** now work, as documented. **`argv()`/`script_args()`** give
  scripts real command-line arguments for the first time
  (`gx run file.gx -- arg1 arg2`). **`--project-sandbox`** lets a
  multi-directory project (`agents/`, `lib/`, a shared `data/`) share file
  access without flattening the layout or disabling sandboxing entirely.
  **`date_add_iso()`** is the safe, string-in-string-out alternative to
  `date_add` (which keeps its existing numeric return for compatibility).
- **`gx check`** now warns on a bare identifier used as a whole discarded
  statement (the `write "x"`-without-parens footgun), and **`gx check
  a.gx b.gx`** actually checks every file given instead of silently only
  the first.
- **`gx fmt`** fixes a real non-idempotency bug on progressive-syntax
  files and uses conventional, dense call-site spacing (`f(x)`, not
  `f ( x )`).
- **`install.sh`** no longer points at a stale, broken hardcoded version —
  resolved dynamically from GitHub's latest release, with the download
  itself fixed (it never actually worked, at any version).

---

## What's New in v0.6.1

Production-hardening patch based on real-world feedback from migrating GClaw (~55 files, 23 agents, 8 bridge integrations) onto v0.6.0. Backward compatible — no previously-working script changes behavior. Full detail in [CHANGELOG.md](CHANGELOG.md).

- **`spawn agent` requires a callable `brain{}`** — targeting an agent with no `brain{}` (including a `when message`-only agent) now fails immediately with a clear error naming the agent and what it actually exposes, instead of silently returning `null`. `brain{}` and `when message` stay genuinely distinct concepts; neither is auto-routed into the other.
- **Fire-and-forget target validation** — `spawn "event" to "agent"` now fails clearly if the target agent isn't declared anywhere in the project, instead of silently queuing a message nothing can ever deliver.
- **`http_post`/`http_put` no longer double-encode a pre-stringified body** — a `Value::Str` body is sent as literal raw bytes, matching every other language's HTTP client convention.
- **`remember.x` aliases `memory.x`** instead of silently evaluating to `null`.
- **Malformed `gx.json` dependency shapes fail loudly** instead of silently becoming deny-all.
- **Bridge call fixes** — the natural 2-part bridge call form now works; `use <ns> "<path>" [as <alias>]` gives js/ts/py/binary/go/rust_bin bridges a local-file/executable form (binary/go/rust_bin were previously unreachable through either parser at all).
- **`gx check` runs real whole-project static diagnostics** — not just a parse check: unreachable spawn targets, undeliverable fire-and-forget sends, dead agents, cross-file name collisions, and SQL built by string concatenation/interpolation, all calibrated for a low false-positive rate.
- **Ollama**: `ask ollama` now honors `timeout` and uses a pooled, capability-checked connection; `context_ask` supports Ollama via `/api/chat`.
- **`response_format` on `ask openai`** — direct pass-through for structured output.
- **Bridge documentation** — a complete "Writing a Bridge Script" guide with worked JS/Python examples.

---

## What's New in v0.6.0

### Runtime Infrastructure — Serialization, Templates, Task Progress, and two real performance fixes

The production runtime, language surface, standard library, and developer
tooling were already complete. This closes the remaining runtime-
infrastructure gaps: JSON Lines and versioned serialization, format-
agnostic file import/export, a general-purpose template renderer, and
progress reporting from background tasks — plus two significant,
empirically-measured performance fixes to core interpreter operations
that shipped as part of the same audit.

```gx
// Serialization: JSON Lines, versioned data, format-agnostic I/O
records = jsonl_parse(read_file("events.jsonl"))
saved = versioned_stringify(state, 2)
data_export("report.yaml", { status: "ok" })

// Templates: render an external template against runtime data
tmpl = read_file("component.template")
write_file("Button.jsx", render_template(tmpl, { name: "Button" }))

// Tasks: report incremental progress from a still-running task
h = task_spawn(fn() {
  task_emit({ step: 1, total: 5 })
  return "done"
})
task_progress(h)
```

- **Serialization Runtime** — `jsonl_parse`/`jsonl_stringify` (JSON
  Lines); `versioned_stringify`/`versioned_parse` (generalizes the AI
  Context Runtime's own version-tag-and-reject-on-drift pattern to any
  persisted data); `data_import`/`data_export` (format-agnostic file I/O
  across `.json`/`.yaml`/`.yml`/`.toml`/`.csv`/`.xml`/`.jsonl`, with
  optional schema validation on export). Every existing format
  (JSON/YAML/TOML/CSV/XML) was already deterministic — confirmed, not
  changed. Binary serialization and custom serializers were investigated
  and found not justified for GX today.
- **Template & Code Generation Runtime** — `render_template(template,
  data)`: `{{dotted.path}}` substitution against a runtime data object,
  loaded from a file (not a GX string literal — GX's own `"{expr}"`
  interpolation runs at parse time and would mangle `{{...}}` first).
  Deliberately not a template *engine* — no control-flow syntax inside
  `{{ }}`; a repeated block is just an ordinary GX loop.
- **Async & Event Runtime** — investigated and found already
  comprehensively solved: `sleep()` for timers, `while + sleep` inside a
  task for cancellable intervals, `when cron` for scheduling, `emit`/
  `when message` for pub/sub, `when <expr> changes` for reactive signals.
  The one real gap — a still-running task having no way to report
  incremental progress before finishing — is filled by
  `task_emit`/`task_progress`, added to the existing Task Runtime rather
  than a new parallel system.
- **Performance** — a full audit found two genuine O(n²) bugs in
  extremely common operations: `arr.push(x)` in a loop (each call cloned
  the whole array; 40,000 pushes took 45+ seconds) and
  `obj[key] = val`/`obj.field = val` in a loop (each call cloned the
  whole object; 50,000 assignments took over a minute). Both are now
  O(1)/O(1)-amortized in-place mutations — 40,000 pushes and 50,000
  field assignments each now complete in well under a second. Fixing the
  second bug also surfaced and fixed a real, silent correctness bug:
  `x = arr.pop()` previously left `arr` completely unchanged (only a
  bare `arr.pop()` statement actually shrank it), which could hang a
  `while len(arr) > 0 { x = arr.pop() }` loop forever.

See [Serialization Runtime](docs/language_reference.md#serialization-runtime),
[Template & Code Generation Runtime](docs/language_reference.md#template--code-generation-runtime),
and the [Task Runtime](docs/language_reference.md#task-runtime)'s new
progress-reporting section for the full reference, and
`docs/examples/production_serialization.gx` /
`production_task_progress.gx` for runnable examples.

### Runtime Completion — REPL, Debugger, Testing Framework, Configuration Runtime

The production runtime, the language surface, and the standard library
were already complete. This closes the developer-experience gap around
all four: a REPL that actually holds state, a real interactive debugger,
named/isolated test cases with setup and teardown, and a single ergonomic
entry point for layered application configuration. All four are built on
capabilities that already existed internally (`run_stmt`'s per-statement
checkpoint, the existing parser/lexer, `assert_count`/`assert_failures`,
`json_parse`/`yaml_parse`/`toml_parse`/`env`/`schema_validate`) — none of
them needed a redesign of the interpreter to expose.

```gx
// REPL: state now persists across lines, multiline blocks buffer
// correctly, imports/`:help`/`:vars`/history all work.
$ gx repl
gx> x = 42
gx> x + 1
43

// Debugger: pause from inside a script, or from the outside with no
// source changes.
breakpoint()
// $ gx debug script.gx --break 12

// Testing Framework: named, isolated cases; setup/teardown via memory.*;
// deterministic randomness; golden-file snapshots.
before_each(fn() { memory.db = test_temp_dir() + "/t.db" })
test("insert then query", fn() {
  set_random_seed(1)
  assert_golden(run_query(memory.db), "tests/golden/query.json")
})

// Configuration Runtime: one call instead of hand-chaining four
// primitives every app previously repeated.
config = config_load({
  defaults: { port: 3000 },
  file: "config.json",
  env_prefix: "APP_",
  schema: { port: "number" },
})
```

- **REPL** — a persistent `Env` held for the whole session (previously
  every line ran through a fresh `run_program`, so `x = 42` on one line
  and `x` on the next silently printed `null`); real multiline-block
  detection via bracket-token counting, not error-message guessing;
  `:help`/`:vars`/`:history`/`:trace`, persistent history in
  `~/.gx_history`.
- **Debugger** — `breakpoint()` (works anywhere, no flag needed), `gx
  debug`/`gx run --break line1,line2`, and an interactive prompt
  (`continue`/`step`/`locals`/`stack`/`print <expr>`/`watch <expr>`)
  built on the same per-statement hook the Task Runtime's cancellation
  already used.
- **Testing Framework** — `test(name, fn)` for named, isolated cases;
  `before_each`/`after_each` sharing state via `memory.*`;
  `set_random_seed(n)` for deterministic `random`/`shuffle`/...;
  `test_temp_dir()`; `assert_golden(actual, path)` with a
  `GX_UPDATE_GOLDEN=1` update workflow.
- **Configuration Runtime** — `config_load(options)`: `defaults` < `file`
  (auto-detected `.json`/`.yaml`/`.yml`/`.toml`) < environment overrides
  (type-coerced, and provably unable to inject a key the app didn't
  already declare) < explicit `overrides`, with optional
  `schema`-validated fail-fast.

See [REPL](docs/language_reference.md#gx-repl--interactive-repl),
[Debugger Runtime](docs/language_reference.md#debugger-runtime),
[Testing Framework](docs/language_reference.md#testing-framework), and
[Configuration Runtime](docs/language_reference.md#configuration-runtime)
for the full reference, and `docs/examples/production_debugger.gx` /
`production_testing.gx` / `production_config.gx` for runnable examples.

### Production AI Context Runtime — managed conversations, token budgeting, automatic trimming

GX's AI primitives (`ask`, `Think`, `embed`, `infer classifier`) were all
single-shot: every call built its prompt from scratch, with no concept of a
conversation. Multi-turn chat, tool-calling loops, and any kind of
token-budget management were entirely the script's own responsibility — and
`memory.ai_trace` (every `ask` result, appended forever) grew without
bound. This is the runtime every AI application actually needs underneath
that: managed conversation state, provider-neutral message assembly, and
automatic, configurable trimming so a long-running agent can't quietly
grow its prompt (and its bill) without limit.

```gx
ctx = context_create({ system: "You are a helpful assistant.", max_history_tokens: 6000 })
ctx = context_add_message(ctx, "user", "What's the capital of France?")

result = context_ask(ctx, "openai", { model: "gpt-4o-mini" })
say result.text
ctx = result.context   // the assistant's reply is already appended, auto-trimmed if needed
```

A context is deliberately plain GX data — a system prompt, an array of
role-tagged messages, some token-budget configuration — not a handle into a
registry the way `process_spawn`/`task_spawn` are, because unlike an OS
process or a background thread, there's no external resource to track. That
one design choice is what makes **context isolation**, **inheritance**, and
**cloning** free: GX's existing value semantics already guarantee a context
passed into a `task_spawn`/`spawn agent` closure is an independent
snapshot, and `context_clone`/`ctx2 = ctx` already deep-copy. It's also why
persistence needs no new API at all — `context_serialize`/
`context_deserialize` (versioned, so a schema change fails loudly instead
of loading silently-wrong) plus ordinary `db_exec`/`db_query` is the whole
story.

Trimming is automatic and configurable (`trim_strategy: "drop_oldest"` /
`"drop_oldest_pair"` / `"none"`), bounded by both a token-budget estimate
and a hard message-count ceiling (defense in depth against the heuristic
estimator being wrong), and tool-result messages get their own configurable
`tool_output_max_chars` cap — one of the most common real sources of
prompt explosion. `context_summarize_and_trim` is the actual "automatic
summarization" extension point: GX provides the mechanical
replace-N-messages-with-one-summary operation; deciding when to call it and
generating the summary text (almost always its own AI call) is left to the
application — this runtime doesn't call a model on your behalf to decide
that for you.

Every AI call — the new `context_ask` and the existing single-shot
primitives alike — now runs through the same capability-checked,
connection-pooled HTTP agent every other outbound request uses (previously
its own unpooled, un-audited `ureq::post` calls), gets an automatic
`ai.request` diagnostics span, and returns a structured `error_kind`
(`rate_limited`/`auth_error`/`invalid_request`/`server_error`/...) plus a
parsed `Retry-After` header — the actual "retry hook"/"rate-limit hook"
this runtime provides, reusing the existing general-purpose `retry()`
builtin rather than inventing a new retry engine.

See [AI Context Runtime](docs/language_reference.md#ai-context-runtime) for
the full reference and `docs/examples/production_ai_context.gx` for a
runnable example.

### Production Task Runtime — safe, observable, cancellable concurrency

Every concurrent primitive GX had — `spawn agent ... timeout`, `parallel {
... }` — was built the same fragile way: a bare `std::thread::spawn` with
no handle, no way to cancel it, and no guarantee it was ever joined. On
timeout, both just **stopped waiting** and abandoned the still-running
background thread — a real orphan-task bug, not a hypothetical one. There
was also no general-purpose way for a script to run its *own* concurrent
work safely.

```gx
h = task_spawn(fn() {
  return process_run({ command: "some-long-job" })
}, { timeout: 30000, label: "job" })

result = task_wait(h, 5000)
if result.timed_out { task_cancel(h) }
```

`task_spawn`/`task_wait`/`task_wait_all`/`task_wait_any`/`task_cancel`/
`task_status`/`task_id`/`is_cancelled` are the new language-level
concurrency primitive every other runtime now builds on — `spawn agent ...
timeout` and `parallel { ... }` are reimplemented on top of it internally
(same GX syntax, unchanged), which is what fixes their orphan-thread bug:
a timed-out call is now a cancelled *task* — tracked, cancellable, and
guaranteed cleaned up by the owning `Interpreter`'s `Drop` even if nothing
ever calls `task_wait` on it again.

Cancellation is cooperative (there's no safe way to force-kill a running
thread) — checked automatically on every statement, so no individual loop
needs to check it itself, plus `sleep()`. It propagates through the
runtimes that need it: cancelling a task **kills any child OS process it
started** (`process_run`/`process_spawn`), and cancelling a task cancels
every task nested under it (`task_spawn` called from inside another task) —
structured concurrency, so a task can never leave an orphaned child task
behind either. Bounded parallel execution is built in too:
`task_spawn(fn, { pool: "workers", max_concurrent: 5 })` caps how many
same-named-pool tasks actually run at once, queuing the rest.

Every task automatically gets a diagnostics span (nested correctly under
whatever span was active when it was spawned) and inherits the spawning
script's capabilities — the same integration `spawn agent`/`parallel{}`
already had, now available for any concurrent work a script wants to do
itself, not just agent calls.

As part of verifying the Database Runtime's task-isolation guarantees under
real concurrent load, this also fixed a real, previously-latent bug there:
`db_transaction` used a plain (deferred) `BEGIN`, which let two concurrent
transactions both read a row (a shared lock) and then race to *upgrade* to
a write lock — a race `busy_timeout` can't resolve by retrying, surfacing
as an immediate "database is locked" error. It's `BEGIN IMMEDIATE` now,
which serializes concurrent writers correctly instead.

See [Task Runtime](docs/language_reference.md#task-runtime) for the full
reference and `docs/examples/production_tasks.gx` for a runnable example.

### Production Diagnostics & Observability Runtime

Every subsystem added so far (HTTP, Process, Database, Capability) used to
log independently, if at all — a `[gx server] GET /x -> 500: ...` `eprintln!`
here, a `[trace]`-prefixed one there, no way to tell which log lines
belonged to the same request, and nothing at all for a failed database
query or a spawned process. GX now has one runtime every subsystem reports
through instead: structured JSON Lines on stderr, correlated by a
`trace_id` (one per top-level run, or one per incoming HTTP request) and
nested `span_id`s.

Two tiers, so the common case stays free. **Tier 1** — `log_debug`/
`log_info`/`log_warn`/`log_error(message, data?)` and automatic
`capability_denied` audit events — is always on, filtered by `--log-level`
(default `info`; `GX_LOG_LEVEL` env var as a fallback). **Tier 2** — spans,
correlation IDs, nested timing — only exists when a script or its invoker
opts in with `--trace`; with it off, `span("...") { ... }` degrades to a
single boolean check and just runs its body, no allocation, no UUID
generation.

```gx
span("checkout") {
  log_info("processing order", { order_id: id })
  db_transaction(db_path) {
    db_exec(db, "UPDATE orders SET status = 'paid' WHERE id = ?", [id])
  }
  result = http_post("https://api.example.com/notify", { order_id: id })
}
```

```bash
gx run checkout.gx --trace --log-level debug
```

```json
{"kind":"span","name":"db.exec","trace_id":"4fd0...","span_id":"1f8b...","parent_span_id":"830c...","duration_ms":3.1,"outcome":"ok","data":{"db":"orders.db"}}
```

Automatic spans now wrap every HTTP client call (`http_get`/`post`/`put`/
`delete`/`http_request`), every incoming HTTP server request (its own fresh
`trace_id`, correctly isolated per request across the worker pool), every
`db_query`/`db_exec`/`db_transaction` (including nested savepoints), every
`process_run`/`process_spawn`/`shell`, and every `spawn agent`/`parallel {
... }` child (which inherits the parent's `trace_id` for correlation
instead of starting an unrelated one). Manual instrumentation is available
too: `span(name) { ... }`, `trace_id()`, `span_id()`. Every span is ended
exactly once regardless of how its body exits — an early error, a caught
exception, or (verified directly) a Rust panic — using the same
`catch_unwind`-based cleanup guarantee already established for
`db_transaction` and SSE response streaming.

See [Diagnostics & Observability](docs/language_reference.md#diagnostics--observability)
for the full reference and `docs/examples/production_diagnostics.gx` for a
runnable example.

### Production Database Runtime — connection pooling, WAL, real nested transactions, migrations

`db_query`/`db_exec` used to open (and immediately drop) a brand-new SQLite
connection on *every single call* — no PRAGMA ever took effect, no
prepared statement was ever reused, and with the HTTP server's worker pool
(previous milestone) now able to run several routes concurrently against
the same `.db` file, this meant relying entirely on SQLite's default
locking: no WAL mode, `busy_timeout=0`. "database is locked" errors under
any real concurrent write load weren't hypothetical. Connections are now
pooled per path, opened once, and configured with `journal_mode=WAL`,
`busy_timeout=5000ms`, and `foreign_keys=ON` — verified under 30 concurrent
HTTP requests writing to one database with zero lock errors.

`db_transaction` had two real correctness bugs, not just missing features.
A transaction nested inside another (a very ordinary pattern — a reusable
"does its own transaction" helper called from a larger transactional
workflow) silently clobbered the outer transaction's connection, since
there was only one global "active transaction" slot. `db_query`/`db_exec`
resolved *that same global slot* regardless of which database path was
actually passed in, so calling a different database from inside an active
transaction silently executed against the wrong one. Both fixed: nesting
now uses real SQLite `SAVEPOINT`s, and the connection lookup is purely
path-based. A panic inside a transaction body — now survivable at the
HTTP-worker level thanks to the previous milestone's `catch_unwind` — used
to leave the transaction state dangling, which would have let the *next*,
unrelated request on that same worker silently run against a stale,
uncommitted transaction; now guaranteed to roll back and clean up
regardless of how the body exits.

New: `db_migrate(path, [sql, ...])` (version tracking via SQLite's own
`PRAGMA user_version`, idempotent, each migration in its own transaction),
`db_backup(path, dest)` (SQLite's real online-backup API — safe against a
concurrently-written source, unlike a plain file copy), `db_integrity_check(path)`,
and `db_vacuum(path)`.

See [Database (SQLite)](docs/language_reference.md#database-sqlite) for
the full reference and `docs/examples/production_database.gx` for a
runnable example.

### Production HTTP Runtime — a real SSRF defense, request headers, concurrency, SSE

The HTTP client's SSRF protection used to check only the URL string once,
before the request — meaning it missed three real bypass classes: a
redirect from an allowed external host to `169.254.169.254`, a hostname
that simply *resolves* to a private address, and an IP written in a
non-dotted-decimal form (`http://2130706433/` is `127.0.0.1`). It's now
backed by a custom DNS resolver that validates the *actual resolved
address* on every connection `ureq` makes, including every redirect hop —
closing all three, verified with local (not flaky-external-service)
regression tests. `http_stream`/`http_upload` were also quietly bypassing
this protection *and* had no timeout at all, since they called `ureq`
directly instead of going through the shared, capability-aware agent;
`http_upload`'s file paths bypassed the Capability Runtime's sandboxing
entirely. All fixed. Every result now carries `body_bytes`/`truncated`
(bodies cap at 32 MiB, never silently) and a stable `error_kind`
(`timeout`/`dns_error`/`blocked`/`http_status`/...) so a script can branch
on failure cause without string-matching. A per-call `timeout` (seconds)
is now accepted directly.

The HTTP server gained the single fix that mattered most: **`request.headers`
now exists.** It didn't before — which meant verifying a webhook signature
(Slack, Discord, GitHub, Stripe) was structurally impossible, not just
inconvenient, since there was no way to read the signature header at all.
Also new: `request.params` (`/users/:id` path parameters), `request.query_params`
(parsed query string), `request.remote_addr`, a 32 MiB request body cap
(rejected with `413` before the route runs), and `respond stream { ... }` +
`sse_send(event, data)` for real Server-Sent Events instead of one buffered
response per request. The server itself moved from handling one request at
a time to a small worker pool (`tiny_http`'s own recommended pattern) — a
slow route waiting on an AI provider no longer blocks every other route,
including unrelated webhooks. A route that throws now returns a generic
500 to the client and logs the real error server-side, instead of echoing
internal error text back to whoever sent the request.

See [HTTP Client](docs/language_reference.md#http-client) and
[HTTP Server](docs/language_reference.md#http-server) for the full
reference, and `docs/examples/production_http.gx` for a runnable webhook +
SSE + path-params example.

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

### Production Module & Package Runtime — reproducible, multi-file GX projects

`gx.json`'s `name`/`version`/`entry`/`dependencies.gx` fields have existed
since `gx init` first scaffolded them, but nothing ever read them — every
field was inert, and `import`'s own file resolution had two real bugs: it
only ever processed the *top-level* program's own imports (so `a.gx`
importing `b.gx`, which itself imported `c.gx`, silently never loaded
`c.gx`), and it resolved paths relative to the current working directory
*before* the importing file's own directory, so the same script could
import a different file depending on where `gx` happened to be invoked
from. This runtime fixes both, and turns the previously-inert manifest
fields into a real dependency system.

```gx
import "leftpad"          // a package, resolved via gx.lock + the local cache
import "./lib/utils.gx"   // a plain file, resolved relative to this file
```

Imports are now **transitive** (an imported file's own imports are
followed, each file parsed at most once no matter how many places import
it) and **deterministic** (always importer-directory-relative first, never
CWD-dependent). Import cycles are caught and reported with the full chain
rather than overflowing the stack, and a name silently overwritten by a
same-named import from a different file now logs a diagnostics warning
instead of vanishing without a trace.

```json
{
  "name": "my-app",
  "version": "0.1.0",
  "dependencies": {
    "gx": {
      "leftpad": { "git": "https://github.com/example/gx-leftpad", "rev": "v1.0.0" },
      "shared-lib": { "path": "../shared-lib" }
    }
  }
}
```

```bash
gx install            # resolve dependencies.gx, fetch/verify, write gx.lock
gx install --offline   # fail clearly instead of touching the network
gx publish             # validate + hash this package, write a .gxpkg.json descriptor
```

`gx.lock` pins the exact resolved version and a SHA-256 integrity hash per
dependency; every later resolution re-verifies a cached package's hash
before using it, so a tampered or corrupted local cache entry is rejected,
not silently used. There is deliberately no hosted package registry — GX
has no server infrastructure to publish one, and building a fake one would
be exactly the kind of feature added "because other languages have it."
Git-based distribution (`gx install`, then depend on a tagged commit) is a
real, complete, offline-capable-after-first-fetch source that needs no new
infrastructure — `gx publish` prints the actual git-tag workflow rather
than pretending to upload anywhere. Path dependencies (`{ "path": "../x" }`)
are for local/monorepo development and are never cached or integrity-
checked, since always reflecting current on-disk content is the entire
point of using one.

See the Module & Package Runtime section in
[docs/language_reference.md](docs/language_reference.md) for the full
`gx.json`/`gx.lock` reference, dependency source kinds, and the capability-
scope boundary (dependency code runs with the same capabilities as the
script that imports it — no per-package sandboxing yet).

### Production Language Surface & Developer Experience

The runtime had grown a real, useful capability set — but not every part of
it was actually *usable* from GX itself. This pass studied the whole
language surface (syntax, builtins, error handling, capability
introspection) end to end and closed the gaps that mattered, without
touching runtime infrastructure or breaking anything existing.

**`retry()` now retries what it was always meant to.** `http_*`/
`process_*`/`task_wait`/`ask`/`context_ask` all signal an operational
failure by *returning* `{ ok: false, ... }`, never by throwing. `retry(fn,
...)` only ever retried a *thrown* error — so `retry(fn() { return
http_get(url) }, 3)` silently never retried a failed request at all, the
very case `retry` exists for. It now retries both.

```gx
result = retry(fn() { return http_get(url) }, 3, { backoff: "exponential" })
```

**Two error-signaling conventions, one bridge.** GX has always had both a
*throwing* convention (`db_query`, file I/O, `readline`) and a *returning*
one (`http_*`, `process_*`, `task_wait`, `ask`) — with no unifying idiom,
code written for one silently does nothing when used against the other. A
`try/catch` around `http_get` never fires; an `if !result.ok` after
`db_query` never runs. New: `unwrap(result)` throws on a `{ ok: false,
... }` result exactly the way `db_query` already does on failure, so
either convention can be handled the same way when that's what you want:

```gx
try {
  data = unwrap(http_get(url))
} catch e {
  log(e.message + " (" + e.kind + ")")
}
```

**`has_capability()` — ask before you try.** Previously the only way to
learn whether an operation was allowed was to attempt it and catch a
`capability_denied` error. `has_capability("external_network")` answers
that up front, with no side effect and no audit-log entry for an
operation that was never attempted — useful for a script that wants to
choose a strategy (network vs. cached fallback) rather than structure
that choice as error handling.

**`db_transaction`/`span`/`parallel` now work in progressive syntax.**
All three previously existed only in brace syntax — a first-class
supported syntax level (see [Syntax Levels](docs/language_reference.md#syntax-levels))
had no way to use transactions, diagnostics spans, or concurrent
statement branches at all:

```
db_transaction(db_path):
  db_exec(db_path, "INSERT INTO accounts(name, balance) VALUES (?, ?)", [name, 0])

parallel:
  memory.a = compute_a()
  memory.b = compute_b()
```

See [Error Handling](docs/language_reference.md#error-handling) in
docs/language_reference.md for the full write-up, including the
`error_kind` vocabulary reference across subsystems (they don't all use
the same one — that's documented now too, not just discoverable by
reading Rust source).

### Production Developer Tooling & Language Intelligence

The runtime and language surface were both substantial already — this
pass is about GX *feeling* like a mature language to actually write, not
just one that's capable underneath.

```
$ gx run script.gx
Error: expected identifier, got Say
  --> script.gx:4:5
   |
 4 |     say "unreachable"
   |     ^
```

Parser/lexer/interpreter errors now render with the offending source line
and, where available, a caret at the exact column — `gx run`, `gx check`,
`gx -e`, and `gx repl` all get this for free from one shared renderer.
Uncaught assertion failures now show their call stack too, matching every
other kind of uncaught error (a caught assertion's `e.message` still
stays exactly what the script wrote, unaffected).

**`gx doc <file.gx|dir> [--out <file.md>]`** — new. Generates a Markdown
API reference: every function/agent/tool, its signature, and any doc
comment immediately preceding it.

**`gx fmt <file.gx|dir> [--check]`** — now accepts a directory (not just
one file, matching `gx test`), and `--check` for CI (report only, exit
non-zero if anything would change, write nothing). Building `--check`
surfaced and fixed two real, pre-existing formatter bugs: the brace-
syntax formatter's token-to-text conversion silently *deleted* most
keywords beyond a small hand-picked set (`assert x == y` could become
`x == y`; `fn(n) { ... }` could become invalid syntax) — it's now an
exhaustive match where a missed token kind is a compile error, not a
silent deletion. String literals containing `\n`/`\t`/`\\`/`\"` were also
re-emitted as literal decoded characters instead of escape sequences,
corrupting multi-line output. Both verified fixed against every `.gx`
file in this repository.

**`gx lsp`** — new. A real, working Language Server over stdio (JSON-RPC
2.0, hand-rolled — no new dependency): live diagnostics as you type,
hover documentation for builtins and your own functions/agents/tools, and
go-to-definition within a file. Point any LSP-capable editor's GX
configuration at `gx lsp`. Cross-file go-to-definition, rename,
find-references, autocomplete, and semantic highlighting are honestly
out of scope for this pass — see
[Developer Tooling](docs/language_reference.md#developer-tooling) for the
full list of what's there and what isn't.

**Every `gx <command>` now accepts `--help`/`-h`** — previously only the
bare `gx help` worked; `gx run --help` fell through to `run`'s own
argument parsing and printed a confusing "file not found: --help".

See [Developer Tooling](docs/language_reference.md#developer-tooling) in
docs/language_reference.md for the full write-up.

### Production Standard Library

GX already had a large builtin surface — ~280 functions and methods. This
pass wasn't about adding more of them for their own sake; it was about
studying that surface from a developer's seat and asking what's actually
missing, inconsistent, or invisible.

**The biggest finding: `.map()`/`.filter()` didn't work as methods.**
Every other array operation — `.sort()`, `.take()`, `.sum()`, 30+ of them
— was already a method, but the two most fundamental ones weren't.
`arr.map(fn)` printed `"unknown method"` and silently returned `null`.
Fixed, additively — `map(arr, fn)` still works exactly as before, and now
so does `arr.map(fn)` (the exact same code path, so the two can never
drift apart). `reduce`, `some`, `every`, and `find_index` didn't exist in
*any* form and were added the same way (both `reduce(arr, fn, initial)`
and `arr.reduce(fn, initial)`):

```gx
paid_totals = orders.filter(fn(o) { return o.status == "paid" }).map(fn(o) { return o.total })
grand_total = orders.reduce(fn(acc, o) { return acc + o.total }, 0)
has_pending = orders.some(fn(o) { return o.status == "pending" })
```

**`random_int(min, max)` / `random_choice(arr)` / `shuffle(arr)`** — only
`random()` (a 0–1 float) existed before. Picking a random array element
or a whole number in a range meant hand-rolling
`floor(random() * (max - min + 1)) + min`, an expression that's easy to
get subtly wrong (forgetting the `+1`, or not flooring). `shuffle` uses
one seeded generator stepped across the whole call, not a fresh
system-clock seed per swap — the latter risks two swaps landing on the
same clock tick and producing a poorly-shuffled result.

**`pick(obj, keys)` / `omit(obj, keys)`** — shaping an object down to (or
excluding) a specific set of fields, extremely common when preparing an
API response or a log line that shouldn't include a password hash, used
to mean reconstructing the object key-by-key by hand.

**`extname(path)`** rounds out the path helpers (`dirname`/`basename`
already existed).

**`xml_parse`/`xml_stringify`** — GX had CSV/YAML/TOML/JSON but zero XML
support. Added as a deliberately narrow, hand-written parser (no new
dependency): mixed content doesn't preserve element/text ordering, and —
this is the important part — **no DTD or entity definitions are ever
processed**, only the five predefined XML entities and numeric character
references. That's the standard defense against XXE injection and
"billion laughs" denial-of-service, both built entirely out of entity
definitions this parser never resolves.

**Documentation**: ~15 array/string/object methods were already
implemented but missing from the reference entirely (`.sort_by()`,
`.distinct()`, `.concat()`, `.pad_start()`, `.slice()`, and more) — pure
discoverability fixes, no code changes. Also documented a real, existing
naming collision worth knowing about: `index_of`/`find` mean *substring
search* as free functions but *value-equality array search* as methods —
same name, different operation, depending on how you call it.

See [Array Methods](docs/language_reference.md#array-methods),
[Object](docs/language_reference.md#object), and
[JSON / Serialization](docs/language_reference.md#json--serialization) in
docs/language_reference.md for the full reference.

### Final Production Hardening — security fixes, resource caps, progressive-syntax parity

A last adversarial pass over the whole runtime before this release, plus
five specific production-hardening items identified by an earlier
engineering review. Every fix below is additive/localized — no redesign,
full backward compatibility with every documented v0.5.x behavior.

**Security fixes:**
- A malicious git/registry dependency's own `gx.json` could set its
  `entry` field to an absolute path or a `../` traversal, making
  `import "pkg"` read an arbitrary file off the importer's disk the
  moment the package was imported. `resolve_package_import_impl` now
  confines a resolved `entry` to the package's own directory.
- `gx build`'s generated launcher embedded the source file in a shell
  heredoc with a fixed delimiter — a `.gx` file containing that exact
  line as text would terminate the heredoc early and turn the rest into
  literal, unsandboxed shell script. The delimiter is now UUID-derived
  and verified against every line of the actual source before use.
- `Bridge::drop` (the `js`/`ts`/`py`/`binary`/`go` subprocess bridges)
  never reaped its child process — every call leaked a zombie (or a
  still-running orphan). Fixed with a bounded reap-and-kill in `Drop`.
- A hung bridge subprocess could block its calling task or HTTP worker
  forever (`Bridge::call`'s `read_line` had no deadline). Now bounded by
  a 300s timeout; a timed-out bridge is retired rather than reused in a
  possibly-desynced state, so the next call transparently spawns a fresh
  one.
- A handful of ordinary operations — `sleep()`, HTTP/process/task
  timeouts, string `*`/`.repeat()`, and AI-provider error-message
  truncation — could crash the whole process on pathological but
  reachable input (`Infinity`-valued numeric literals, multi-byte UTF-8
  characters at a truncation boundary). All now fail as a normal,
  catchable error instead.

**New resource caps** (all documented in
[Resource Limits](docs/language_reference.md#resource-limits)): a
concurrent `respond stream` (SSE) responder-thread cap, a pooled SQLite
connection cap with idle-only LRU eviction (a connection with an active
transaction is never evicted), and `task_spawn` now reports an OS
thread-creation failure as a catchable error instead of panicking the
whole interpreter.

**Progressive-syntax parity**: `on <expr> changes:` and
`on cron "...":` were being silently mis-parsed as a dead identifier
trigger that could never fire — no error, just a block that silently
never ran. Fixed, along with general boolean-expression `when` triggers
(`on count > 5:`, previously only a bare identifier worked) and the
`goal:`/`retry:`/`on_error:` agent-header fields (previously a hard
parse error in progressive syntax despite being read by the
interpreter). `receive {}`/`recipe`/`objective` remain brace-syntax-only
— see [Progressive syntax: known limitations](docs/language_reference.md#progressive-syntax-known-limitations).

See [CHANGELOG.md](CHANGELOG.md) for the complete, itemized list.

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
| **v0.7.0** | **AgentX feedback & scope-chain unification** — top-level constants and named functions now visible as real values everywhere; syntax-mode misdetection fix; single-quote strings; `&&`/`||`; `argv()`; `--project-sandbox`; `date_add_iso()`; `gx check` bare-identifier lint + multi-file fix; `gx fmt` idempotency fix + denser spacing; `install.sh` fixed. Backward compatible. See [What's New in v0.7.0](#whats-new-in-v070) and [CHANGELOG.md](CHANGELOG.md). |
| **v0.6.1** | **Production hardening** — GClaw migration feedback: `spawn agent`/fire-and-forget target validation, HTTP string-body fix, strict manifest validation, bridge syntax + local-path fixes, Ollama timeout/managed-context support, whole-project `gx check` diagnostics, OpenAI structured output. Backward compatible. See [What's New in v0.6.1](#whats-new-in-v061) and [CHANGELOG.md](CHANGELOG.md). |
| **v0.6.0** | **Production Runtime** — Capability/Crypto/Process/HTTP/Database/Task/AI Context/Module & Package runtimes, Diagnostics & Observability, Testing Framework, Debugger, REPL, Configuration/Serialization/Template runtimes, LSP, formatter, doc generation · security and resource-cap hardening · progressive-syntax parity fixes. See [What's New in v0.6.0](#whats-new-in-v060) and [CHANGELOG.md](CHANGELOG.md). |
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
cargo test                       # 500+ unit tests — must all pass
gx test                          # 29 integration test files
cargo clippy -- -D warnings      # zero warnings
cargo fmt --check                # formatted
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

## License

MIT — © 2026 Ahmed Elgarhy / DEVJSX LIMITED (London, UK). Company No: 16618207.
