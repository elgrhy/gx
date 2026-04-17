# GX Language — Claude Code Context

## What is GX?

GX is a brain-first programming language for building transparent, auditable AI assistants. The language makes every AI decision explicit, every AI call logged, and every agent fully debuggable.

**Owner:** Ahmed Elgarhy, Founder of DEVJSX LIMITED (London, UK). Company No: 16618207.

---

## Current State (v0.1.0 — Real Implementation)

The GX interpreter is fully built and working. `gx run file.gx` executes real GX code.

**What works:**
- Lexer, parser, AST, tree-walking interpreter (all in Rust)
- Two parsers: brace syntax (`parser.rs`) + progressive indentation syntax (`indent_parser.rs`)
- `agent`/`helper` with `brain { plan {} execute {} remember {} communicate {} }`
- `when started {}`, `when expr {}`, `when expr changes {}` trigger blocks
- AI primitives: `ask openai/anthropic/ollama`, `embed`, `infer classifier`
- Package interop: `use js.X`, `use py.X` (subprocess bridges)
- User-defined functions: `function name(args) { body }`
- File imports: `import "file.gx"`
- Full toolchain: `gx run/check/build/init/test/fmt/make/install`
- All operators: `+`, `-`, `*`, `/`, `%`, `+=`, `-=`, `*=`, `/=`, `==`, `!=`, `<`, `>`, `<=`, `>=`
- Unary minus, `len()`, `range()`, string/array methods

---

## Progressive Syntax (New)

GX supports three syntax levels that all compile to the same AST/runtime:

### Level 1 — Pure intent
```gx
Agent greeter
name = "World"
"Hello {name}"
```
- No braces, no `remember {}`, no `when started {}`
- Variables at agent level → `memory` entries
- String literals → auto-print
- Detected automatically (no `{` in file)

### Level 2 — Named behaviors
```gx
Agent assistant
Greet:
  "Hello {name}"
On start:
  Greet
```
- `BehaviorName:` → extracted as a zero-arg function
- Calling `BehaviorName` (no parens) → auto-calls the function
- Memory changes propagate back from behaviors to the agent
- `On start:` → `when started` block

### Level 3 — Explicit brain cycle
```gx
Agent counter
Plan:
  action = "increment"
Execute:
  If action == "increment"
    count += 1
Remember:
  memory.count = count
Communicate:
  count
```
- `Plan:`, `Execute:`, `Remember:`, `Communicate:` → brain phases
- `If`, `For`, `Try` use indentation instead of braces

---

## Architecture

```
src/
├── main.rs           CLI entry point
├── lexer.rs          Tokenizer (keywords, operators, literals)
├── parser.rs         Recursive descent parser → AST (brace syntax)
├── indent_parser.rs  Line-by-line parser → AST (progressive syntax)
├── ast.rs            AST node types (Program, HelperDef, Stmt, Expr, ...)
├── interpreter.rs    Tree-walking executor
├── value.rs          Runtime values (Null, Bool, Number, Str, Array, Object)
├── ai.rs             AI provider connectors (OpenAI, Anthropic, Ollama)
├── bridge.rs         JS (node -e subprocess) / Python (persistent process) IPC
├── toolchain.rs      gx init/build/install/fmt/make/test
└── lib.rs            Public API: parse_source(), run_source(), check_source()
```

**Key design decisions:**
- Tree-walking interpreter — no bytecode, simple to debug
- Two front-ends (brace + indent) compile to one AST
- Flat `memory` scope per agent — predictable, auditable
- Memory fallback: bare `name` resolves to `memory.name` in progressive syntax
- Behavior calls are zero-arg functions with shared memory (changes propagate back)
- Every AI call auto-logged to `memory.ai_trace`
- JS bridge: one-shot `node -e` subprocess per call
- Python bridge: persistent child process with JSON IPC

---

## Tests

```bash
cargo test                 # 30 Rust unit tests
cargo run -- run tests/test_basics.gx
cargo run -- run tests/test_control_flow.gx
cargo run -- run tests/test_strings.gx
cargo run -- run tests/test_agent.gx
cargo run -- run tests/test_functions.gx
cargo run -- run tests/test_import.gx
cargo run -- run tests/test_progressive_syntax.gx
```

CI runs: `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check` on ubuntu/macos/windows.

---

## What NOT to Do

- Do not claim GX is self-hosting — the interpreter is written in Rust, not GX
- Do not claim there's an OS kernel or DNKN networking — these don't exist
- Do not break the progressive syntax detection: `is_indent_syntax()` in `indent_parser.rs`
- Do not remove the memory fallback (`name` → `memory.name`) without updating all progressive syntax tests
- The brace syntax tests in `tests/*.gx` use `for each`, `{}` blocks, etc. — don't break them

## Key Invariants

- `cargo test` must pass 30 tests
- `cargo clippy -- -D warnings` must be clean
- `cargo fmt --check` must be clean
- Both brace and indentation parsers must produce valid Programs
- CI on GitHub Actions must stay green (ubuntu + macos + windows)
