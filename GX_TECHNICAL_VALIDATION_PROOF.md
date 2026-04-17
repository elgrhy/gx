# GX Language — Technical Validation

> See [README.md](README.md) and [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md).

## Verified Functionality

```bash
# All of these work:
gx run docs/examples/hello_world.gx       # Hello, Brain-First World!
gx run docs/examples/calculator.gx        # 5 + 3 = 8
gx run docs/examples/simple_agent.gx      # Agent started! ...
gx run docs/examples/package_interop.gx   # js.path.join result: ...
gx init my-test && cd my-test && gx test  # 1 passed
gx build docs/examples/hello_world.gx     # dist/hello_world
```

## CI Badge

CI passes on ubuntu-latest, macos-latest, windows-latest:
- `cargo test` (24 tests)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
