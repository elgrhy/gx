# Changelog

All notable changes to GX are documented here. This file starts with
**v0.6.0** — for the full history of earlier releases (v0.1.0 through
v0.5.1), see [README.md § Version History](README.md#version-history) and
the "What's New in vX" sections above it, which have served as GX's
release notes since the first release.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/).

## [0.7.0] — 2026-07-15 — AgentX Feedback & Scope-Chain Unification

Findings from building AgentX (~6,000 lines, 34 files, 17 agents, 11 lib
modules, a scheduler, a live-dashboard CLI, a full test suite) end-to-end
on v0.6.1. A full engineering review of the underlying architectural
causes is in `docs/language-review-agentx-feedback-2026-07.md`. Backward
compatible — every change below is additive, a bug fix for behavior
nothing could have intentionally depended on, or a fix that only ever
makes a *previously silently broken* program work correctly instead of
changing a previously *correct* one. Two items the review identified as
genuinely breaking (see "Deferred" below) were deliberately **not**
shipped in this release.

### Fixed — Critical

- **A brace-syntax file using `agent` as a plain variable name inside a
  function body — or containing any string/comment/object-key shaped
  like `"On error:"` anywhere in the file — silently misrouted the
  *entire file* to the indentation parser**, which then dropped every
  construct it didn't recognize, producing an empty program that exited
  0 with no output or error. Root cause: syntax-mode detection scanned
  every line of the file for four patterns instead of only the file's
  first line, where every real progressive-syntax file's header
  actually lives. Both this and the indentation parser's previous
  silent-skip of any unrecognized top-level line are now loud, specific
  parse errors instead.
- **A top-level `NAME = value` assignment was invisible from inside any
  function defined later in the same file** — a real, reported
  data-corruption-class bug (a top-level constant/array silently read as
  `null` inside every function, for an entire development period, with
  no error). `Env` had no lexical scope chain; top-level globals and
  named `function` declarations lived in side-tables only `run_helper`
  (for agents) ever read. `call_user_function`/
  `call_user_function_propagating` now inject file-root globals into
  every function call, the same way agents already worked — with normal
  shadowing (a same-named parameter or local reassignment wins, and
  never writes back to the real global).
- **A named `function foo() {}` was not a referenceable value** — `say
  foo` (bare, no parens) silently printed `null`, and `task_spawn(foo)`
  (documented as taking "a zero-arg closure") failed with a confusing
  "expected a function, got null" instead of working, unlike `x = fn()
  {}; task_spawn(x)`. A bare reference to a named function now
  synthesizes the same `Value::Closure` representation `fn(){}` produces,
  unifying the two as real, passable values.
- **A method call inside `{...}` string interpolation whose argument was
  itself a quoted string silently failed to evaluate** — `"{arr.join(',
  ')}"` printed the literal, un-interpolated `{...}` text with no error,
  because GX's lexer had no single-quote string syntax at all and the
  interpolation's internal re-lex of the expression silently fell back
  to literal text on failure. Single-quoted strings are now supported
  (identical semantics to double-quoted), which fixes this directly; the
  interpolation scanner is also now aware of both quote kinds while
  extracting expression source, so a brace or the other quote character
  inside a nested string no longer perturbs extraction.

### Fixed — High

- `"{{" + "x" + "}}"` (concatenated at runtime) produced `"{x}}"` instead
  of the correct `"{x}"` — a string containing only a doubled `}}` with
  no `{` at all skipped the `{{`/`}}`-unescaping loop entirely.
- `gx check a.gx b.gx c.gx` silently checked only the first file argument
  and dropped the rest with no error or notice — now checks every file
  given and fails if any one of them has an error.
- `gx fmt` on progressive-syntax files was not idempotent — a second
  `gx fmt`/`gx fmt --check` pass would report the file as needing
  reformatting again, because the indentation-normalizing pass computed a
  line's trimmed body with a method that only strips *trailing*
  whitespace, so the original leading indentation was written a second
  time right after the freshly computed one, growing on every pass. Found
  while investigating a separate, reported-but-unreproduced formatter bug
  (see "Investigated, not reproduced" below) — this is unrelated to that
  report but was a real bug in the same subsystem.
- `install.sh` was broken independent of which version it pointed at:
  the download URL requested a literal binary filename release.yml has
  never published under (every real release asset is an archive), and
  the function's own success/failure status could never reflect a failed
  download underneath it, so the designed fallback to a source build
  never actually triggered. `GX_VERSION` is also no longer a hardcoded
  literal — it's resolved from GitHub's latest-release API at install
  time (or pinned explicitly: `GX_VERSION=x.y.z sh install.sh`).

### Added

- **`&&`/`||`** now tokenize as documented (previously only the `and`/`or`
  word forms worked; `&&`/`||` hit "unexpected character" errors).
- **`argv()` / `script_args()`** — `gx run file.gx -- arg1 arg2` (and the
  `gx file.gx -- arg1 arg2` shorthand) passes everything after a literal
  `--` through to the script unchanged, including a value that happens to
  look like one of `gx`'s own flags. There was previously no way at all
  for a script to receive command-line arguments.
- **`--project-sandbox`** — sandbox file I/O to the nearest ancestor
  directory containing a `gx.json`, instead of only the entry script's own
  directory, for projects laid out in subdirectories (`agents/`, `lib/`, a
  shared `data/`) that need to reach a sibling directory without
  flattening the whole project or dropping to `--no-sandbox`. Falls back
  to the existing default if no ancestor has a `gx.json` — opting in never
  widens access beyond what was asked for.
- **`date_add_iso(date, n, unit)`** — same arithmetic as `date_add`, but
  always returns an ISO-8601 string, matching `date_now()`'s
  representation. `date_add` itself keeps its existing numeric return
  (see "Deferred" below) but is now documented prominently as the one
  date-producing builtin whose output type doesn't match its own input.
- **`gx check` now warns on a bare identifier used as a whole, discarded,
  non-last statement** (e.g. `write "x"` without parens, which silently
  parses as two independent no-op statements rather than one call) —
  exempting progressive syntax's zero-argument "named behavior" auto-call
  sugar and a block's last statement (an idiomatic implicit return),
  verified against the entire real `.gx` test corpus with zero false
  positives.
- `gx fmt`'s canonical style now uses conventional, dense spacing around
  parentheses/brackets (`f(x)`, not `f ( x )`) instead of padding every
  delimiter uniformly.

### Investigated, not reproduced

- The report's most severe finding — `gx fmt` silently truncating the
  last character of an identifier immediately before a closing `}` — did
  **not** reproduce against this exact source tree across the original
  repro plus eight structural variants. `format_source` is a genuine
  token-stream reprinter with an exhaustive match over every token kind;
  no code path exists that could plausibly cause it. A *related but
  distinct* bug (whole-keyword deletion via a non-exhaustive match) was
  already fixed one day before the v0.6.1 tag, which is the most likely
  explanation if the reporter's binary predated that fix. A permanent
  identifier-round-trip property test was added regardless — formatter
  trust doesn't depend on this specific mechanism ever having existed.

### Deferred

Two items from the engineering review are genuinely breaking changes and
were deliberately **not** shipped in this release, to keep it backward
compatible:

- Making an unbound identifier a hard runtime error (rather than
  evaluating to `null`) would immediately break any program relying on
  the previous behavior in ways Tier 0/1 of this release don't already
  fix. The recommended migration path — ship as a `gx check` warning
  first (done, see "Added" above, though scoped narrowly to the
  bare-statement case rather than every expression position), observe,
  only then consider a hard error in a future major version — is
  followed here.
- Changing `date_add`'s return type to match `date_now()`'s ISO-string
  representation would break any call site relying on the current
  numeric result. `date_add_iso` ships as the non-breaking alternative
  instead (see "Added" above).

### Migration notes

Nothing in this release requires a code change. Two things are worth
knowing about if you're upgrading a large existing project, since both
are genuine (if narrow) behavior changes — neither can turn a
previously-*correct* program incorrect, only make a previously-*broken*
one start working, or change cosmetic formatter output:

- **A function can now see a top-level constant of the same name it
  previously couldn't.** If any function reassigns a local variable
  whose name happens to collide with an unrelated top-level global
  *and* was — knowingly or not — relying on the two being invisible to
  each other, the local reassignment still correctly shadows the global
  within that function (see the regression tests in
  `interpreter::tests` for the exact shadowing semantics), so this can
  only matter if code was *reading* that name expecting `null` and
  branching on it. This was already undefined, silently-wrong behavior
  before this release; if anything depended on it, that dependency was
  itself the bug.
- **`gx fmt`'s output has changed** (denser call-site spacing, and a
  fixed non-idempotency bug on progressive-syntax files). Every
  previously-formatted file will show as "changed" on the next `gx fmt`
  run — purely cosmetic, but worth knowing about before a CI
  `gx fmt --check` gate runs on an existing codebase for the first time
  after upgrading. Run `gx fmt .` once to pick up the new style in one
  commit.

## [0.6.1] — 2026-07-12 — GClaw Production Feedback & Hardening

Findings from migrating GClaw (~55 files, 23 agents, 8 bridge integrations)
from v0.5.x to v0.6.0. Every item below traces to a reproduced bug, not a
hypothetical one. Backward compatible — no previously-working script
changes behavior.

### Fixed — Critical

- **`spawn agent "x" with { ... }` silently returned `null` for any agent
  built from `when message "..."` blocks instead of `brain { }`** — the
  single most damaging finding: it broke entire subsystems (dashboards,
  search, safety validation) with no error at all. `brain { }` and
  `when message` stay genuinely distinct concepts — `spawn agent` is not
  routed into a matching `when message` handler, even when its `action`
  names one, since a handler's contract should be knowable from its own
  declaration, not from which call form happens to invoke it. Instead,
  targeting an agent with no `brain { }` now fails immediately with a clear
  error naming the agent and what it actually exposes, instead of
  returning `null`.
- **`spawn "event" to "agent"` (fire-and-forget) silently queued a message
  forever when the target agent didn't exist** — a typo'd or removed agent
  name used to queue into an internal event bus under a key nothing would
  ever drain: a permanently-undeliverable message, indistinguishable from a
  successful send, plus a small unbounded leak. Now fails immediately with
  a clear error naming the agent, unless the target genuinely exists —
  an existing agent that just doesn't (yet) declare a matching handler is
  unaffected and still queues, exactly as before.
- **`http_post`/`http_put` double-encoded a pre-stringified JSON body** —
  `http_post(url, json_stringify(x))` sent a quoted, escaped string literal
  instead of the object the server expected. A `Value::Str` body is now
  sent as the literal raw bytes, matching every other language's HTTP
  client convention for a string body.
- **`remember.x` silently evaluated to `null`** instead of erroring or
  working — `remember` is the declaration keyword, `memory` is the
  accessor, and the two are easy to reach for interchangeably. `remember.x`
  now aliases `memory.x` instead of silently losing the value.
- **A malformed `gx.json` `dependencies.*` shape silently became deny-all**
  — an array of `{"name": ..., "path": ...}` objects (a reasonable, more
  informative shape to write by hand) filtered down to an empty allowlist
  with `filter_map`, denying every call in that namespace with no signal
  the *shape*, not the intent, was wrong. Now rejected loudly at manifest
  load time.
- **The 2-part bridge call form (`playwright_bridge.navigate(...)` after
  `use js.playwright_bridge`) silently resolved to `null.navigate(...)`** —
  only the undocumented 3-part form (`js.playwright_bridge.navigate(...)`)
  actually worked. The natural 2-part form now resolves correctly.

### Fixed — High

- `ask ollama` now honors the `timeout` param (previously silently
  dropped) and reuses the same pooled, capability-checked HTTP agent every
  other provider uses — with `internal_network` pre-authorized specifically
  for ollama, so the single most common workflow (a local model on
  `localhost`) doesn't newly require `--allow-internal-http`.
- `use binary "path"`/`use go "path"`/`use rust_bin "path"` — previously
  unreachable through **either** GX parser at all, despite the capability
  system and runtime dispatch already supporting them. `use <ns> "<path>"
  [as <alias>]` now works for `js`/`ts`/`py`/`binary`/`go`/`rust_bin`,
  also giving js/ts/py bridges a way to point at a local project file
  instead of only a `require()`/`importlib` bare-specifier lookup.
- `context_ask(ctx, "ollama", ...)` now actually wires into Ollama's
  `/api/chat` (message-array based, the same shape used for
  openai/anthropic) instead of hard-rejecting every call.
- Cross-file function/agent name collisions (`import`'s "last one wins")
  are now a `gx check` finding, not just a runtime log line easy to miss in
  a production log stream.

### Added

- **`gx check` now runs real static diagnostics**, not just a parse check
  — across the whole project (the entry file plus everything it
  transitively `import`s): a spawn target with no `brain { }` (including a
  `when message`-only agent, regardless of whether `action` names a real
  handler), a fire-and-forget `spawn "event" to "agent"` target that
  resolves to no declared agent, an agent declared but never spawned, a
  cross-file name collision, and SQL built by string
  concatenation/interpolation instead of a `?` placeholder. Every check is
  conservative by design (a dynamically-constructed target is skipped, not
  guessed at) to keep the false-positive rate low enough to trust in CI.
- `response_format` on `ask openai { ... }` — a direct pass-through to
  OpenAI's own structured-output param (no equivalent exists on Anthropic's
  API to pass through to).
- A `trim_strategy: "summarize"` context now warns (instead of staying
  silent) when it evicts messages by plain removal — it still doesn't
  generate an actual summary; see `context_summarize_and_trim` for that.
- A "Writing a Bridge Script" doc section with complete, correct JS and
  Python examples — the actual calling convention (a plain module, no
  stdin/IPC handling of its own) had no worked example anywhere before
  this, and every bridge script in a real production migration was written
  the wrong way as a result.

### Deferred (tracked, not implemented this round)

- An automatic tool-execution loop (`agent_loop`-style builtin) — the
  highest-leverage remaining gap for competing with Claude/ChatGPT-class
  agentic products, but a genuinely new primitive deserving its own design
  pass rather than a rushed addition here.
- WebSocket support — explicitly out of scope already; SSE covers
  server→client push today.
- Full `trim_strategy: "summarize"` implementation (an actual
  provider-driven summary) — the eviction-vs-summarization gap is now
  *visible* (see Fixed — High above) rather than fixed outright, since
  auto-triggering an AI call during ordinary context trimming would itself
  be a "silent AI call" — the exact failure class this release otherwise
  spent its effort closing.

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
