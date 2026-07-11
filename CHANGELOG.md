# Changelog

All notable changes to GX are documented here. This file starts with
**v0.6.0** — for the full history of earlier releases (v0.1.0 through
v0.5.1), see [README.md § Version History](README.md#version-history) and
the "What's New in vX" sections above it, which have served as GX's
release notes since the first release.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/).

## [0.6.0] — 2026-07-11

GX's production runtime, standard library, developer tooling, and language
surface are now complete: everything needed to build and operate a real
production application, not just prototype an agent. This release is
intentionally **not** v1.0 — the public API will be validated further
through real production use (GClaw) before being frozen.

### Security and Capability Runtime

- Unified Capability Runtime: filesystem, process, shell, internal/external
  network, database, and AI-provider access are all gated through one
  authorization path, with every denial audited automatically.
- Fixed: a malicious git/registry dependency's `gx.json` could set its
  `entry` field to an absolute path or a `../` traversal, making
  `import "pkg"` read an arbitrary file off the importer's disk. Entry
  paths are now confined to the package's own directory.
- Fixed: `gx build`'s generated shell launcher embedded the source file in
  a heredoc with a fixed, guessable delimiter — a source file containing
  that exact line as text could terminate the heredoc early and turn the
  rest into literal, unsandboxed shell script executed the moment the
  built binary ran. The delimiter is now derived per-build and verified
  against the actual source before use.
- Fixed: `gx install`'s npm/pip package-name validator allowed a leading
  `-`, the same CLI argument-injection class already closed for `git`'s
  own argument validator.
- Fixed: an HTTP SSRF pre-check mis-parsed `user:pass@host` URLs (cosmetic
  — the authoritative resolver-level check was never bypassable, but the
  pre-check now classifies and audit-logs the real host correctly).
- Webhook/HMAC verification, SSRF defenses (resolved-IP validation on every
  connection including redirects — closes DNS rebinding), and process
  execution (argument-array, never a shell) were audited and confirmed
  sound.

### Process Execution

- Native `process_run`/`process_spawn`: structured argument arrays (no
  shell, no injection surface), capped stdin/stdout/stderr, timeouts, and
  cancellation.
- Fixed: `task_spawn` panicked the whole interpreter if the OS refused to
  create another thread (e.g. under a tight `ulimit -u`); now returns a
  catchable error, same as every other resource-exhaustion path.

### HTTP and Networking

- Full HTTP client (`http_get`/`post`/`put`/`delete`/`http_stream`) and
  server (`serve on port N { route ... }`) runtime, with a two-layer SSRF
  defense (a fast pre-check plus an authoritative resolver-level check on
  every connection, including redirects).
- Server-Sent Events (`respond stream` / `sse_send`) with real
  backpressure.
- Fixed: a `respond stream` client that connects and never reads left its
  responder thread permanently blocked — with no cap, enough such clients
  accumulated one abandoned thread each, unboundedly. Now capped
  server-wide (256 concurrent responders); beyond the cap, a new stream
  request is rejected with a clear error instead of leaking another
  thread.

### Database and Persistence

- Pooled SQLite connections with production PRAGMAs (WAL, busy_timeout,
  foreign keys), real nested transactions via savepoints, and
  panic-safe rollback.
- Fixed: the connection pool grew without bound for a long-running
  Interpreter (a `serve` worker, most realistically) touching many
  distinct database paths. Now capped (128 connections) with idle-only LRU
  eviction — a connection with an active transaction is never evicted,
  even under pool pressure.

### Tasks and Concurrency

- `task_spawn`/`task_wait`/`task_cancel`, bounded pools
  (`{ pool: "name", max_concurrent: N }`), and `task_emit`/`task_progress`
  for incremental progress reporting from a still-running task.
- Fixed: `Bridge::call` (the `js`/`ts`/`py`/`binary`/`go` subprocess
  bridges) had no read timeout — a hung subprocess permanently blocked
  whichever task or HTTP worker called it. Now bounded (300s); a timed-out
  bridge is retired rather than reused in a possibly-desynchronized state,
  so the next call transparently spawns a fresh subprocess.
- Fixed: `Bridge::drop` never reaped its child process — every bridge call
  leaked a zombie or still-running orphan process. Now reaped with a
  bounded grace period on drop.

### AI Context

- AI Context Runtime: persistent conversation history, tool-output
  truncation/flagging, and version-tagged serialization for context state.
- `ask`/`context_ask`'s `error_kind` vocabulary and retry/rate-limit
  handling confirmed complete and correctly documented (see Reliability
  below).

### Diagnostics and Observability

- Structured leveled logging and audit events (always on), plus opt-in
  spans/trace IDs (`--trace`) with automatic instrumentation across
  HTTP/DB/process/task calls.

### Modules and Packages

- `import`, `use js/ts/py/binary/go`, and a package manager (`gx install`)
  with lockfile-pinned (`gx.lock`) git/registry/path dependencies and
  path-traversal-safe cache naming.

### Standard Library

- ~280 builtin functions and methods across strings, arrays, objects,
  JSON/YAML/TOML/CSV/XML, crypto, regex, date/time, and more.

### Testing, Debugger, REPL, and Configuration

- Testing Framework (`test`/`before_each`/`after_each`, deterministic
  randomness, golden-file snapshots), an interactive debugger
  (`breakpoint()`, `gx debug`, step/watch/locals/stack), a REPL with
  real persistent state across lines, and a Configuration Runtime
  (`config_load` layering defaults/file/env with schema validation).

### Serialization and Templates

- JSON Lines, versioned serialization, format-agnostic file import/export,
  and a template renderer (`render_template`) for code generation.

### Developer Tooling

- `gx lsp` (hover, diagnostics, go-to-definition), `gx fmt`, `gx doc`, and
  `--help`/`-h` on every subcommand.

### Reliability and Performance

- Two genuine O(n²) bugs fixed in extremely common operations:
  `arr.push(x)` and `obj[key] = val` in a loop each used to clone the
  whole array/object per call; both are now O(1)/O(1)-amortized in-place
  mutations. Fixing the object case also surfaced and fixed a real, silent
  correctness bug: `x = arr.pop()` previously left `arr` unchanged.
- Fixed a class of process-crashing panics reachable from ordinary,
  non-malicious scripts: `Duration`-related panics on `Infinity`-valued
  numeric literals (`sleep()`, HTTP/process/task timeouts), a string
  `*`/`.repeat()` capacity-overflow panic, and non-UTF-8-boundary-safe
  string truncation in AI-provider error messages. All now fail as a
  normal, catchable error instead of aborting the process.
- Fixed two progressive-syntax (indentation) parser bugs that failed
  *silently*: `on <expr> changes:` and `on cron "...":` were being
  mis-parsed as a dead identifier trigger that could never fire — no
  error, just a block that silently never ran. Also added general
  boolean-expression `when` triggers and the `goal:`/`retry:`/`on_error:`
  agent-header fields to progressive syntax (previously a hard parse error
  despite being read by the interpreter). `receive {}`/`recipe`/
  `objective` remain brace-syntax-only — see the language reference's
  "Progressive syntax: known limitations".
- `docs/language_reference.md`'s `error_kind` vocabulary reference table
  corrected to match actual runtime behavior (it was missing several
  documented-elsewhere-but-omitted-here values for `process_*` and
  `ask`/`context_ask`).

### Backward compatibility

Every fix in this release is either strictly additive (new progressive-
syntax support, new resource caps that only reject previously-crashing or
previously-leaking input) or corrects behavior that was never actually
usable (a silently-dead trigger, a panic). No documented, working v0.5.x
behavior changed.

### Known limitations

- `tests/test_tools.gx` depends on the external `httpbin.org` service being
  reachable; an unrelated outage there fails that one test file
  independent of any GX code change.
- `receive {}`, `recipe`, and `objective` blocks have no progressive-syntax
  form yet (brace syntax only).
