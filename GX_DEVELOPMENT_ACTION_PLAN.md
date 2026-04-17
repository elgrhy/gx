# GX Language — Development Action Plan

> Phases 1–6 are complete. See [MASTER_PLAN.md](MASTER_PLAN.md) for the full roadmap.

## Next: Phase 7 — Self-Hosting

Write the GX lexer, parser, and interpreter in GX itself, running on the Rust interpreter. Once stable, GX can compile itself — the Rust interpreter becomes just the bootstrap seed (like GCC's C bootstrap).

Steps:
1. Write `gx-lexer.gx` — tokenizes GX source using string operations
2. Write `gx-parser.gx` — builds an AST as GX objects
3. Write `gx-interpreter.gx` — tree-walks and executes
4. Bootstrap: run `gx run gx-interpreter.gx gx-interpreter.gx` — self-hosting achieved

## Near-term Improvements

- VS Code extension (syntax highlighting, snippets)
- Homebrew formula SHA256 (requires tagging v0.1.0 release)
- `gx doc` command — generate HTML docs from `.gx` files
- Comment preservation in `gx fmt`
- `gx publish` — publish agents to a registry
