# GX Language — Test Report

> See [README.md](README.md) for current status.

## Rust Unit Tests (24 total)

All tests in `src/` pass. Coverage includes:

- Lexer: tokenization, string literals, keywords, `re-run` token, semicolons
- Parser: helpers, brain blocks, when blocks, if/else, for loops, try/catch, AI expressions, bridge calls
- Interpreter: memory, brain cycle, string interpolation, arithmetic, `when started`, nested memory

```
test result: ok. 24 passed; 0 failed; 0 ignored
```

## Integration Tests

```bash
gx run docs/examples/hello_world.gx     # PASS
gx run docs/examples/calculator.gx      # PASS
gx run docs/examples/simple_agent.gx    # PASS
gx run docs/examples/package_interop.gx # PASS (requires node + python)
gx test tests/                          # 1 passed, 0 failed
```
