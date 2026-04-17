# Contributing to GX Language

Thank you for contributing to GX. This document covers everything you need to get started.

---

## Prerequisites

- **Rust** (stable) — [rustup.rs](https://rustup.rs)
- **Node.js** (optional, for JS bridge tests)
- **Python 3** (optional, for Python bridge tests)
- **Git**

---

## Setup

```bash
git clone https://github.com/elgrhy/gx.git
cd gx
cargo build
cargo test
```

That's it. No external build scripts, no GCC, no NASM.

---

## Project Structure

```
src/
├── main.rs          CLI — gx run, gx check, gx init, etc.
├── lexer.rs         Tokenizer
├── parser.rs        AST builder
├── ast.rs           AST node definitions
├── interpreter.rs   Tree-walking executor
├── value.rs         Runtime value types (Null, Bool, Number, Str, Array, Object)
├── ai.rs            AI provider connectors (OpenAI, Anthropic, Ollama)
├── bridge.rs        JS and Python subprocess bridges
├── toolchain.rs     gx init/build/install/fmt/make/test
└── lib.rs           Public embedding API

docs/examples/       Working .gx example files
tests/               Rust unit tests (in src/) + .gx integration tests
Formula/             Homebrew formula
npm/                 npm wrapper package (gxlang)
.github/workflows/   CI (ci.yml) and release (release.yml)
```

---

## Running Tests

```bash
# Rust unit tests (24 tests covering lexer, parser, interpreter)
cargo test

# Lint (must pass — CI enforces -D warnings)
cargo clippy -- -D warnings

# Format (must pass — CI enforces)
cargo fmt --check

# Run a GX example
cargo run -- run docs/examples/hello_world.gx
cargo run -- run docs/examples/simple_agent.gx
cargo run -- run docs/examples/calculator.gx
```

---

## Adding a Feature

### 1. New keyword

1. Add a `TokenKind` variant in `lexer.rs`
2. Map the keyword string in `Lexer::keyword_or_ident()`
3. Add `expect_ident()` handling in `parser.rs` if the keyword can appear as an identifier
4. Add the AST node in `ast.rs`
5. Parse it in `parser.rs`
6. Execute it in `interpreter.rs`
7. Write a Rust test and a `.gx` test file

### 2. New built-in function

Add a match arm in `Interpreter::call_builtin()` in `interpreter.rs`.

### 3. New AI provider

Add a dispatch arm in `ai.rs::ask_ai()` and implement `ask_<provider>()`.

### 4. New toolchain command

Add a function in `toolchain.rs` and a CLI arm in `main.rs`.

---

## Code Style

- Run `cargo fmt` before every commit
- Run `cargo clippy -- -D warnings` and fix all warnings
- No `unsafe` code
- No `unwrap()` in production paths — use `?` or explicit error handling
- Write a test for every new language feature

---

## Writing Tests

### Rust unit tests

Add to the relevant `mod tests {}` block in each source file:

```rust
#[test]
fn test_my_feature() {
    let result = run_source(r#"
        agent "test" {
            when started { say "ok" }
            brain { plan {} execute {} remember {} communicate {} }
        }
    "#);
    assert!(result.is_ok());
}
```

### GX integration tests

Create `tests/test_my_feature.gx`:

```gx
helper "test_my_feature" {
  brain {
    plan { plan = { action: "test" } }
    execute {
      if plan.action == "test" {
        result = 1 + 1
        if result == 2 {
          log("PASS: arithmetic works")
        } else {
          log("FAIL: arithmetic broken")
        }
      }
    }
    remember {}
    communicate {}
  }
}
```

Run with: `gx test tests/`

---

## Commit Messages

Use conventional commits:

```
feat(lexer): add TokenKind::When variant
fix(interpreter): memory not persisted across when blocks
docs: update API reference with embed examples
test(parser): add test for nested field access
```

---

## Pull Request Checklist

- [ ] `cargo fmt` — no changes
- [ ] `cargo clippy -- -D warnings` — no errors
- [ ] `cargo test` — all pass
- [ ] New feature has a test (Rust unit test + .gx test file)
- [ ] Example in `docs/examples/` if it's a user-facing feature
- [ ] Docs updated if syntax changed

---

## Reporting Issues

Use [GitHub Issues](https://github.com/elgrhy/gx/issues). Include:

- GX version (`gx version`)
- OS and architecture
- Minimal `.gx` file that reproduces the issue
- Expected vs actual output

---

## Community

- GitHub Issues — bug reports and feature requests
- GitHub Discussions — questions and ideas

---

**© 2025 DEVJSX LIMITED** — Company No: 16618207

**Ahmed Elgarhy** — Founder, DEVJSX | AI Software Architect
