# GX Roadmap — v0.5.0 through v0.8.0

**v0.5.0, v0.5.1, and v0.6.0 are shipped.** v0.7.0 is next. Sequenced in **phases**, each a shippable release.
Decisions locked with the owner:
- **Phased delivery** (not one big release).
- **No type system** — GX stays fully dynamic. Revisit later if adoption demands it.
- **Stdlib = native Rust builtins** exposed via `use std.<module>` (fast, zero-dependency), not GX-written modules.

Codebase facts this plan is grounded in (confirmed):
- CLI dispatch: `src/main.rs` (`run/check/init/build/install/fmt/make/test/repl/version/help`; flags
  `--debug --allow-shell --allow-internal-http --no-sandbox --no-limit`).
- Toolchain commands (`init/build/install/fmt/make/test`): `src/toolchain.rs` (~904 lines).
- Interpreter is a module directory: `src/interpreter/` (e.g. `builtins_vector.rs`). Builtins dispatch lives here.
- AI layer `src/ai.rs` already returns `tokens_used` per call and has a private `truncate()` helper — so token
  data already flows; we just need to surface it to GX and add a builtin.
- Parser `src/parser.rs` is 2359 lines (the "bloat" item). Lexer `src/lexer.rs` 644. Indent parser 924.
- No `std/` directory exists. No package manager. `gx fmt` exists but feedback says output is inconsistent → needs a real pass.
- `parallel {}` runs sequentially today.

Legend: ✅ ship target · ◐ partial-exists · ✨ new

---

## Phase 1 — v0.5.0 "Daily-driver DX + stdlib + token awareness" ✅ SHIPPED

Released 2026-05-31. All items below are live.

### 1.1 Output control (`say` newline problem) ✨
- Add `print(x)` builtin: writes `to_display(x)` to stdout **without** trailing newline (+ flush).
- Keep `say`/existing print-with-newline unchanged (back-compat).
- Optional: `say(x, end: "")`-style named arg later; `print` is the minimal fix.
- File: `src/interpreter/` builtins dispatch. Test: prompt-then-readline on same line.

### 1.2 `gx -e '<source>'` eval flag ✨
- In `src/main.rs` dispatch, add `"-e" | "eval" => { run inline source string }`.
- Reuse `cmd_run` path but source from arg instead of file; sandbox base = CWD; honor same flags.
- Test: `gx -e 'print("hi")'` prints `hi` with no temp file.

### 1.3 Script-relative path resolution ✨ (fixes `load_env(".env")`)
- Today sandbox is the script's dir, but relative paths in builtins resolve against CWD.
- Resolve relative file/env paths against `interp.base_path`'s directory (the script dir), not CWD.
- Affects: `read_file`, `write_file`, `append_file`, `file_exists`, `list_dir`, `load_env`.
- Keep absolute paths absolute. Keep sandbox check intact.
- Also: allow `load_env` at file root (not only inside `when started`) — wire it into the top-level run path.
- Tests: run from a different CWD; `load_env(".env")` still finds the file next to the script.

### 1.4 String/stdlib `truncate(value, max)` ✨
- Promote the private `ai.rs::truncate` concept to a public builtin: `truncate(str_or_value, max)`.
- Works on strings (char-safe) and stringifies non-strings first. Appends `…` when clipped (configurable later).
- Removes the hand-rolled `truncate()` every project writes.

### 1.5 Token awareness ✨ (the "GX could own this" idea, minimal slice)
- `token_count(text)` builtin: fast heuristic estimate (≈ chars/4, whitespace-aware) tagged as estimate.
  Good enough to *budget before* an API call; documented as approximate.
- `$tokens_used` magic variable: cumulative tokens across the session, summed from the `tokens_used`
  already returned by `src/ai.rs` on every `ask`/`embed`. Maintain a counter on `Interpreter`, expose as a
  read-only variable resolved in the evaluator.
- Docs note: exact tokenizers (tiktoken/Claude) are out of scope for v0.5; heuristic only.

### 1.6 Native stdlib v1 (`use std.<module>`) ✨ — Rust builtins
Wire a `use std.X` resolver that registers a namespaced builtin set. Start with the modules the feedback named:
- `std.fs`: `path_join(...parts)`, `dirname(p)`, `basename(p)`, `glob(pattern)` (sandbox-respecting).
- `std.collections`: `group_by(arr, key_fn|key)`, plus expose existing `sort`/`unique` under the namespace.
- `std.crypto`: `sha256(s)`, `uuid()` (v4).
- `std.net`: `url_parse(s)` → object `{scheme, host, port, path, query, fragment}`.
- Implementation: a `src/interpreter/stdlib/` submodule (mod.rs + fs.rs/collections.rs/crypto.rs/net.rs).
  `sha256`/`uuid` via small dependencies (`sha2`, `uuid`) or hand-rolled to keep zero-dep promise — decide at impl.

### 1.7 Docs + tests + help
- Update `print_help()` builtin list and `CLAUDE.md`.
- Add `tests/test_stdlib.gx`, `tests/test_dx.gx`; keep `cargo test` green; `clippy -D warnings`; `fmt --check`.

**Shipped:** `truncate`, `write` (no newline), `gx -e`, sandbox fix for `load_env`, `use std.fs/crypto/collections/net`, `token_count`, `tokens_used()`, `sha256`, `uuid`, `glob`, `dirname`, `basename`, `path_join`, `url_parse`, `group_by`. crates.io `gxlang@0.5.0` + npm `gxlang@0.5.0` live.

---

## Phase 2 — v0.6.0 "Real parallelism + package manager + managed context"

### 2.1 Real `parallel {}` ✨ (currently sequential — misleading)
- Execute each branch on its own OS thread (the `spawn agent` path already uses real threads — reuse that
  machinery). Snapshot needed environment per branch (closures already snapshot — v4.2 fix).
- Join all; collect results in branch order. Surface per-branch errors (fail-fast vs collect — pick fail-fast,
  document). Guard shared mutable memory (clone-in/merge-out, or document that branches get isolated copies).
- Tests: timing test proving concurrency; determinism of result ordering.

### 2.2 Package manager `gx install <pkg>` + `gx.json` manifest ✨
- Extend the existing `gx.json` (already read for JS/Py module allowlists in `main.rs`) with a `dependencies`
  section for **GX packages**: `{ "name": "...", "version": "...", "dependencies": { "gx": { "<pkg>": "^1.2.0" } } }`.
- `gx install <pkg>[@version]`: resolve from a simple registry (start: git URL / local path / tarball over HTTPS),
  write to `gx.json` + lockfile (`gx.lock`), unpack into `gx_modules/`.
- `import "pkg:<name>/file.gx"` resolution that looks in `gx_modules/` then up the tree.
- Versioning: semver match; lockfile pins exact. Keep it deliberately small (npm-lite, no transitive hell v1).
- File: `src/toolchain.rs` (install/resolve) + new `src/registry.rs`.

### 2.3 Managed conversation context (agent-level) ✨ — the bigger idea
- Agent config keys: `max_context_tokens`, `trim_strategy: "last_n" | "summarize"(later)`, `persist: "path"`.
- Tool config: `max_result_chars` → auto-truncate a tool's return value before it enters history.
- Runtime owns a managed history buffer per agent: auto-append, auto-trim oldest when over budget (using
  `token_count`), auto-load/save when `persist` set. Exposes `$messages` (managed) read access.
- `when rate_limited { ... }` lifecycle hook: on a 429 from `src/ai.rs`, fire the hook (with `backoff`/attempt
  context) instead of crashing; default behavior = exponential backoff + retry N times.
- This is what would have prevented the 116KB-HTML-in-history incident.

**Exit criteria v0.6.0:** `parallel {}` is genuinely concurrent; `gx install` pulls a versioned GX package and
runs; an agent can declare `max_context_tokens` + `trim_tool_results` and the runtime manages history; 429s hit
`when rate_limited` instead of crashing.

---

## Phase 3 — v0.7.0 "LSP / IDE"
- `gx lsp`: stdio Language Server (tower-lsp). Reuse lexer/parser for: diagnostics (parse + check),
  hover docs (builtins + agent/brain keywords), completion (builtins, memory keys, behaviors),
  go-to-definition (functions, behaviors, imports), document symbols.
- Ship the existing `editors/` VSCode extension wired to the server; document Neovim setup.
- Files: new `src/lsp/` module + `gx lsp` subcommand in `main.rs`.

## Phase 4 — v0.7.x "Formatter that's actually consistent"
- Make `gx fmt` a real opinionated pass over the AST (or token stream) for **both** brace and indent syntaxes:
  consistent indentation, spacing around operators, `{}`/block style, trailing-newline, idempotent
  (`fmt(fmt(x)) == fmt(x)`). Add `gx fmt --check` for CI.
- File: rewrite `toolchain::fmt` backed by a new `src/formatter.rs`.

## Phase 5 — v0.8.0 "Debugger"
- `gx debug <file.gx>`: breakpoints (`break` on line/`debugger` stmt), step/next/continue, variable
  inspection (dump `memory` + locals), call-stack view (we already have source-located stack traces from v0.4.2 —
  build on that). Start as an interactive CLI debugger; DAP adapter later for IDE integration.

---

## Parser bloat (cross-cutting, low priority — owner-flagged)
- `src/parser.rs` 2359 lines w/ 45+ keyword→string mappings. Not a release blocker.
- Plan when touched: extract the keyword table into a single `keywords.rs` map; split parser into
  `parser/{expr,stmt,decl}.rs`. Do opportunistically alongside Phase 3 (LSP reuses parser internals).

---

## Feedback → Phase traceability
| Feedback item | Phase |
|---|---|
| `say` always newline / inline prompt | 1.1 |
| No `-e` flag | 1.2 |
| CWD-relative paths / `load_env(".env")` | 1.3 |
| `load_env` needs agent scope | 1.3 |
| No `truncate()` in stdlib | 1.4 |
| `token_count` / rate-limit transparency / `$tokens_used` | 1.5, 2.3 |
| No stdlib (fs/collections/crypto/net) | 1.6 |
| `parallel {}` sequential | 2.1 |
| No package manager / `gx.json` versions | 2.2 |
| Managed context / `max_context` / `trim_tool_results` / `when rate_limited` | 2.3 |
| `retry()` tricky (while often simpler) | eased by 2.3 rate-limit hook; document pattern |
| LSP / IDE | 3 |
| Formatter inconsistent | 4 |
| Debugger | 5 |
| No type system | out of scope (owner decision) |
| Parser bloat | cross-cutting, opportunistic |
