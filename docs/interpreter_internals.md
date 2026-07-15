# GX Language — Developer Guide

How to work on the GX interpreter itself. For user docs, see [language_reference.md](language_reference.md).

---

## Setup

```bash
git clone https://github.com/elgrhy/gx.git
cd gx
cargo build
cargo test
```

Requirements: Rust stable (rustup.rs). Nothing else.

---

## Source Map

```
src/
├── main.rs               CLI — gx run, gx -e, gx check, gx init, etc.
├── lexer.rs              Tokenizer — reads .gx source, emits Vec<Token>
├── ast.rs                AST node types — Program, HelperDef, Stmt, Expr, ...
├── parser.rs             Brace-syntax parser — turns Vec<Token> into Program AST
├── indent_parser.rs      Progressive-syntax parser — line-by-line, indentation-based
├── interpreter/
│   └── mod.rs            Tree-walking executor — eval_expr, run_stmt, eval_builtin
├── value.rs              Runtime value — Null, Bool, Number, Str, Array, Object
├── ai.rs                 AI connectors — ask_ai(), embed_text(), infer_classifier()
├── bridge.rs             Subprocess bridges — JS (node -e) and Python (persistent IPC)
├── toolchain.rs          CLI tools — init, build, install, fmt, make, test
└── lib.rs                Library API — run_source(), check_source(), parse_source()
```

---

## How Execution Works

1. `is_indent_syntax(source)` — detects which parser to use
2. If indent syntax: `indent_parser::parse(source)` → `Program`
3. If brace syntax: `Lexer::new(source).tokenize()` → `Vec<Token>` → `Parser::new(tokens).parse()` → `Program`
4. `Interpreter::new().run_program(&program)` → executes all helpers

Both parsers produce identical `Program` ASTs. The runtime is the same for both.

---

### Lexer

`TokenKind` enum covers all GX keywords and literals. Key design notes:
- Semicolons are treated as newlines (`TokenKind::Newline`)
- `re-run` is a single token — detected in `read_ident()` by checking if `"re"` is followed by `-run`
- String interpolation (`"Hello, {name}!"`) is handled at the parser/interpreter level, not the lexer
- `expect_ident()` accepts ~30 keyword tokens as valid identifiers — GX is not strictly keyword-reserved, so `memory.plan = 1` is valid

### Parser (brace syntax)

`Parser` struct has `namespaces: HashSet<String>` — populated by `use` declarations. This lets `parse_postfix()` decide whether `js.path.join(...)` is a `BridgeCall` or a `FieldAccess`.

`normalize_provider()` maps `"claude"` → `"anthropic"`, `"gpt"` → `"openai"`, `"local"` → `"ollama"`.

### Indent Parser (progressive syntax)

`is_indent_syntax(source)` returns `true` when the source contains no `{` character (the heuristic that distinguishes progressive from brace syntax). Do not break this detection.

Progressive syntax maps:
- `Agent Name` → `HelperDef { name: "Name", ... }`
- `var = val` at agent level → `remember {}` entry
- `BehaviorName:` → zero-arg function + `when started` call
- `On start:` → `when started {}` block
- `Plan:` / `Execute:` / `Remember:` / `Communicate:` → brain phases

Memory fallback: bare `name` in progressive syntax resolves to `memory.name` if `name` is not in local scope.

### Interpreter

`Interpreter` struct key fields:
- `env: Env` — flat `HashMap<String, Value>` per scope; no parent chain
- `sandbox_dir: Option<PathBuf>` — file I/O is restricted to this directory
- `allow_shell: bool` — gates `shell()` builtin
- `allow_internal_http: bool` — gates HTTP to private/localhost IPs
- `total_tokens_used: u64` — cumulative tokens from all `ask` calls
- `base_path: Option<String>` — path of the currently-running script

`memory` is stored as `Value::Object(HashMap)` inside `Env` under the key `"memory"`. All `memory.x` accesses resolve through `env.get("memory")` then field lookup.

**Execution order for a helper:**
1. Initialize `memory` from `remember {}` block
2. Run all `when started {}` blocks
3. Run the `brain {}` cycle (plan → execute → remember → communicate)
4. Handle `re-run` signal (restart from step 3, max 100 cycles)
5. Run remaining `when expr {}` and `when expr changes {}` blocks

**Builtin dispatch:** `eval_builtin(name, args)` is a large `match` on the function name string. New builtins are added as match arms. All builtins that read/write files must call `self.safe_path(&raw)` first to enforce sandboxing. Platform-only builtins (sha256, uuid, glob) are gated with `#[cfg(not(target_arch = "wasm32"))]`.

`KNOWN_BUILTINS` is a `const` array of all builtin names — used for "Did you mean?" suggestions. Add new builtins here when implementing them.

`record_tokens(n)` is called after every `ask_ai` call to increment `total_tokens_used`. `tokens_used()` and `total_tokens()` builtins return this value.

### Values

```rust
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}
```

`Value` implements `Display`, `PartialEq`, and `Clone`. `json_stringify` preserves integer-valued floats as integers (no `42.0` in output — `42`).

### AI Module

`ask_ai(provider, model, params)` dispatches to `ask_openai`, `ask_anthropic`, or `ask_ollama`. All use `ureq` (synchronous HTTP — no async needed). Returns a struct including `tokens_used: u64`.

Confidence scoring:
- OpenAI: based on `finish_reason` (`"stop"` → 0.9, `"length"` → 0.7)
- Anthropic: always starts at 0.9 (no logprobs exposed)
- Both: adjusted down by `adjust_confidence_for_hedging()` which scans for phrases like "I think", "maybe", "not certain"

Streaming: when `stream: true`, chunks are printed to stdout as they arrive; the full assembled text is returned in `result.text`.

### Bridge Module

**JS bridge:** For each call, spawns `node -e` with the call embedded as JSON, reads JSON from stdout. Overhead: ~50ms per call (one-shot process).

**Python bridge:** `Bridge` struct with a persistent `child: Child` process running an embedded Python shim. Communicates via JSON over stdin/stdout. Overhead: ~5ms per call after startup. `Drop` impl sends `{"type":"exit"}` to clean up.

---

## Adding a New Builtin

1. Add a match arm in `eval_builtin()` in `src/interpreter/mod.rs`:

```rust
"my_builtin" => {
    let arg = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::Str(format!("result: {}", arg)))
}
```

2. For platform-only builtins (require native deps, no WASM):

```rust
#[cfg(not(target_arch = "wasm32"))]
"my_builtin" => {
    // use native crate here
    Ok(Value::Str("...".to_string()))
}
```

3. Add the name to `KNOWN_BUILTINS` array near the top of `interpreter/mod.rs`.

4. Update `print_help()` in `src/main.rs` if it's user-facing.

5. Write a test in `tests/test_myfeature.gx` and run `gx test`.

---

## Adding a New Keyword

Example: adding `repeat N times { ... }`:

**1. Lexer** (`src/lexer.rs`):
```rust
"repeat" => TokenKind::Repeat,
"times" => TokenKind::Times,
```

**2. AST** (`src/ast.rs`):
```rust
Repeat { count: Expr, body: Vec<Stmt>, line: usize },
```

**3. Parser** (`src/parser.rs`):
```rust
TokenKind::Repeat => {
    let line = self.line();
    self.advance();
    let count = self.parse_expr()?;
    self.expect(&TokenKind::Times)?;
    let body = self.parse_block()?;
    Ok(Stmt::Repeat { count, body, line })
}
```

**4. Interpreter** (`src/interpreter/mod.rs`):
```rust
Stmt::Repeat { count, body, .. } => {
    let n = self.eval_expr(count)?.as_number().unwrap_or(0.0) as usize;
    for _ in 0..n {
        self.run_stmts(body)?;
    }
    Ok(Value::Null)
}
```

**5. Test** (`tests/test_repeat.gx`):
```gx
memory.n = 0
repeat 3 times { memory.n += 1 }
assert_eq(memory.n, 3, "repeat 3 times")
print("test_repeat: all passed")
```

---

## CI / Release

**CI** (`.github/workflows/ci.yml`): runs on every push to `main` and on PRs.
- `cargo test` on ubuntu, macos, windows
- `cargo clippy -- -D warnings`
- `cargo fmt --check`

**Release** (`.github/workflows/release.yml`): triggered by `v*` tags OR `workflow_dispatch`.
- Cross-compiles for: `x86_64-linux`, `aarch64-linux`, `x86_64-macos`, `aarch64-macos`, `x86_64-windows`
- Creates GitHub Release with all binaries attached
- Publishes to crates.io (`gxlang`) and npm (`gxlang`) — idempotent (skips if version already exists)
- Requires secrets: `CARGO_REGISTRY_TOKEN`, `NPM_TOKEN` (must be an Automation-type token for npm)

To cut a release:
```bash
# Bump version in Cargo.toml and npm/package.json first
git tag v0.7.0
git push origin v0.7.0
```

---

## Release Status

| Version | Status | Date |
|---|---|---|
| v0.1.0 | Shipped | — |
| v0.2.0 | Shipped | — |
| v0.2.5 | Shipped | — |
| v0.3.0 | Shipped | — |
| v0.4.0 | Shipped | — |
| v0.4.1 | Shipped | — |
| v0.4.2 | Shipped | 2026-05-30 |
| v0.5.0 | Shipped | 2026-05-31 |
| v0.5.1 | Shipped | 2026-06-06 |
| v0.6.0 | Shipped | 2026-07-11 |
| v0.6.1 | Shipped | 2026-07-12 |
| **v0.7.0** | **Shipped** | **2026-07-15** |
| v0.8.0 | Planned | — |

See [ROADMAP_v0.5_to_v0.8.md](../ROADMAP_v0.5_to_v0.8.md) for what's next.

---

**© 2026 DEVJSX LIMITED** — Ahmed Elgarhy
