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
├── main.rs               CLI — gx run, gx -e, gx check, gx init, etc.
├── lexer.rs              Tokenizer
├── parser.rs             Brace-syntax AST builder
├── indent_parser.rs      Progressive-syntax AST builder
├── ast.rs                AST node definitions
├── interpreter/
│   └── mod.rs            Tree-walking executor (eval_builtin dispatch)
├── value.rs              Runtime value types (Null, Bool, Number, Str, Array, Object)
├── ai.rs                 AI provider connectors (OpenAI, Anthropic, Ollama)
├── bridge.rs             JS and Python subprocess bridges
├── toolchain.rs          gx init/build/install/fmt/make/test
└── lib.rs                Public embedding API

docs/                     Documentation (Markdown)
docs/examples/            Working .gx example files
tests/                    .gx integration tests (run with: gx test)
npm/                      npm wrapper package (gxlang)
Formula/                  Homebrew formula
.github/workflows/        CI (ci.yml) and release (release.yml)
ROADMAP_v0.5_to_v0.8.md  Phased development roadmap
```

---

## Running Tests

```bash
# Rust unit tests (500+ tests covering lexer, parser, interpreter, and every runtime)
cargo test

# Lint (must pass — CI enforces -D warnings)
cargo clippy -- -D warnings

# Format (must pass — CI enforces)
cargo fmt --check

# GX integration tests (29 test files) — must be run via `gx test`, not `gx run`:
# `gx test` grants the `process` capability and sandboxes relative file I/O to
# the current working directory, which several test files' paths assume.
gx test

# Run an individual test file directly (works for files that don't rely on
# `gx test`'s capability grants or CWD-relative paths)
cargo run -- run tests/test_basics.gx
cargo run -- run tests/test_v05_stdlib.gx
cargo run -- run tests/test_v05_dx.gx

# Quick inline test
cargo run -- -e 'say sha256("abc")'
```

**Known external dependency**: `tests/test_tools.gx` makes real outbound
requests to `https://httpbin.org` to exercise `http_request`. If that
third-party service is unavailable or rate-limits the request, this one test
file fails independent of any GX code change — verify by curling
`https://httpbin.org/get` directly before treating a failure here as a
regression.

---

## Adding a Feature

### 1. New keyword

1. Add a `TokenKind` variant in `lexer.rs`
2. Map the keyword string in `Lexer::keyword_or_ident()`
3. Add `expect_ident()` handling in `parser.rs` if it can appear as an identifier
4. Add the AST node in `ast.rs`
5. Parse it in `parser.rs` (and `indent_parser.rs` if it should work in progressive syntax)
6. Execute it in `interpreter/mod.rs`
7. Write a Rust unit test and a `.gx` integration test file

### 2. New built-in function

Add a match arm in `eval_builtin()` in `src/interpreter/mod.rs`. Also add the name to the `KNOWN_BUILTINS` array near the top of the file so "Did you mean?" works.

```rust
"my_builtin" => {
    let arg = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::Str(format!("result: {}", arg)))
}
```

If the builtin should be unavailable on WASM, wrap it:
```rust
#[cfg(not(target_arch = "wasm32"))]
"my_builtin" => { ... }
```

### 3. New AI provider

Add a dispatch arm in `ai.rs::ask_ai()` and implement `ask_<provider>()`.

### 4. New toolchain command

Add a function in `toolchain.rs` and a CLI arm in `main.rs`.

### 5. Progressive syntax

If the feature should work in progressive syntax (indentation-based), also update `indent_parser.rs`. Test with `is_indent_syntax()`.

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
        x = sha256("abc")
        assert_eq(len(x), 64, "sha256 hex length")
    "#);
    assert!(result.is_ok());
}
```

### GX integration tests

Create `tests/test_my_feature.gx`:

```gx
// tests/test_my_feature.gx

// Test new builtin
result = my_builtin("input")
assert_eq(result, "expected output", "my_builtin basic case")

// Test edge case
assert_eq(my_builtin(""), "", "my_builtin empty string")

print("test_my_feature: all assertions passed")
```

Run with: `gx test` or `gx run tests/test_my_feature.gx`

Test builtins available:
- `assert_eq(a, b, msg)` — equality check with message
- `assert_true(cond, msg)` — boolean check
- `assert_contains(haystack, needle, msg)` — substring / membership check

---

## Commit Messages

Use conventional commits:

```
feat(interpreter): add sha256 and uuid builtins
fix(sandbox): load_env now respects sandbox_dir
docs: update language reference for v0.5.0
test(stdlib): add glob and url_parse smoke tests
```

---

## Pull Request Checklist

- [ ] `cargo fmt` — no changes
- [ ] `cargo clippy -- -D warnings` — no errors
- [ ] `cargo test` — all 82+ tests pass
- [ ] `gx test` — all integration tests pass
- [ ] New feature has a Rust unit test + `.gx` integration test
- [ ] New builtin added to `KNOWN_BUILTINS` in `interpreter/mod.rs`
- [ ] Help text updated in `print_help()` in `main.rs`
- [ ] Docs updated if syntax or builtins changed

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

**© 2026 DEVJSX LIMITED** — Company No: 16618207

**Ahmed Elgarhy** — Founder, DEVJSX | AI Software Architect
