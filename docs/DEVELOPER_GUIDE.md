# GX Language — Developer Guide

How to work on the GX interpreter itself. For user docs, see [API_REFERENCE.md](API_REFERENCE.md).

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
├── main.rs          CLI entry point — parses argv, dispatches to commands
├── lexer.rs         Tokenizer — reads .gx source, emits Vec<Token>
├── ast.rs           AST node types — Program, HelperDef, Stmt, Expr, ...
├── parser.rs        Parser — turns Vec<Token> into Program AST
├── interpreter.rs   Executor — tree-walks the AST, maintains Env
├── value.rs         Runtime value — Null, Bool, Number, Str, Array, Object
├── ai.rs            AI connectors — ask_ai(), embed_text(), infer_classifier()
├── bridge.rs        Subprocess bridges — JS (node -e) and Python (IPC)
├── toolchain.rs     CLI tools — init, build, install, fmt, make, test
└── lib.rs           Library API — run_source(), check_source()
```

---

## How Execution Works

1. `Lexer::new(source).tokenize()` → `Vec<Token>`
2. `Parser::new(tokens).parse()` → `Program`
3. `Interpreter::new().run_program(&program)` → runs all helpers

### Lexer

`TokenKind` enum covers all GX keywords and literals. Key design notes:
- Semicolons are treated as newlines (`TokenKind::Newline`)
- `re-run` is a single token — detected in `read_ident()` by checking if `"re"` is followed by `-run`
- String interpolation (`"Hello, {name}!"`) is handled at the parser/interpreter level, not the lexer

### Parser

`Parser` struct has `namespaces: HashSet<String>` — populated by `use` declarations. This lets `parse_postfix()` decide whether `js.path.join(...)` is a `BridgeCall` or a `FieldAccess`.

`expect_ident()` accepts ~30 keyword tokens as valid identifiers — GX is not strictly keyword-reserved, so `memory.plan = 1` is valid.

`normalize_provider()` maps `"claude"` → `"anthropic"`, `"gpt"` → `"openai"`, etc.

### Interpreter

`Env` is a flat `HashMap<String, Value>` — no parent chain, no lexical scoping. This is intentional for Phase 1: GX agents have a single shared flat scope, which is simple and predictable.

`memory` is stored as `Value::Object(HashMap)` inside `Env` under the key `"memory"`. All `memory.x` accesses resolve through `env.get("memory")` then field lookup.

**Execution order for a helper:**
1. Initialize `memory` from `remember {}` block
2. Run all `when started {}` blocks
3. Run the `brain {}` cycle (plan → execute → remember → communicate)
4. Handle `re-run` signal (restart from step 3, max 100 cycles)
5. Run remaining `when expr {}` and `when expr changes {}` blocks

### Values

`Value` enum:
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

`Value` implements `Display`, `PartialEq`, and `Clone`.

### AI Module

`ask_ai(provider, model, params)` dispatches to `ask_openai`, `ask_anthropic`, or `ask_ollama`. All use `ureq` (synchronous HTTP — no async needed).

Confidence scoring:
- OpenAI: based on `finish_reason` (`"stop"` → 0.9, `"length"` → 0.7)
- Anthropic: always starts at 0.9 (no logprobs exposed)
- Both: adjusted down by `adjust_confidence_for_hedging()` which scans for phrases like "I think", "maybe", "not certain"

### Bridge Module

**JS bridge:** For each call, spawns `node -e` with the call embedded as JSON, reads JSON from stdout. Overhead: ~50ms per call (one-shot process).

**Python bridge:** `Bridge` struct with a persistent `child: Child` process running an embedded Python shim. Communicates via JSON over stdin/stdout. Overhead: ~5ms per call after startup. `Drop` impl sends `{"type":"exit"}` to clean up.

---

## Adding a New Keyword

Example: adding `repeat N times { ... }`:

**1. Lexer** (`src/lexer.rs`):
```rust
// In keyword_or_ident():
"repeat" => TokenKind::Repeat,
"times" => TokenKind::Times,
```

**2. AST** (`src/ast.rs`):
```rust
// In Stmt enum:
Repeat { count: Expr, body: Vec<Stmt>, line: usize },
```

**3. Parser** (`src/parser.rs`):
```rust
// In parse_stmt():
TokenKind::Repeat => {
    let line = self.line();
    self.advance();
    let count = self.parse_expr()?;
    self.expect(&TokenKind::Times)?;
    let body = self.parse_block()?;
    Ok(Stmt::Repeat { count, body, line })
}
```

**4. Interpreter** (`src/interpreter.rs`):
```rust
// In run_stmt():
Stmt::Repeat { count, body, .. } => {
    let n = self.eval_expr(count, env)?.as_number().unwrap_or(0.0) as usize;
    for _ in 0..n {
        self.run_stmts(body, env)?;
    }
    Ok(Value::Null)
}
```

**5. Test** (`src/interpreter.rs` test module):
```rust
#[test]
fn test_repeat() {
    let src = r#"
        helper "t" {
            brain {
                plan {}
                execute {
                    memory.n = 0
                    repeat 3 times { memory.n += 1 }
                }
                remember {}
                communicate {}
            }
        }
    "#;
    assert!(run_source(src).is_ok());
}
```

---

## CI / Release

**CI** (`.github/workflows/ci.yml`): runs on every push to `main` and on PRs.
- `cargo test` on ubuntu, macos, windows
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Example file checks

**Release** (`.github/workflows/release.yml`): triggered by `v*` tags.
- Cross-compiles for: `x86_64-linux`, `aarch64-linux`, `x86_64-macos`, `aarch64-macos`, `x86_64-windows`
- Creates GitHub Release with all binaries attached

To cut a release:
```bash
git tag v0.2.0
git push origin v0.2.0
```

---

## Roadmap

| Phase | Status |
|-------|--------|
| 1 — Rust interpreter | Done |
| 2 — Simple syntax (`agent`, `when`, `re-run`) | Done |
| 3 — AI primitives (`ask`, `embed`, `infer`) | Done |
| 4 — Package interop (`use js.X`, `use py.X`) | Done |
| 5 — Toolchain (`init`, `build`, `install`, `fmt`, `make`, `test`) | Done |
| 6 — Distribution (curl, npm, Homebrew formula, CI/release) | Done |
| 7 — Self-hosting (rewrite interpreter in GX itself) | Planned |

---

**© 2025 DEVJSX LIMITED** — Ahmed Elgarhy
