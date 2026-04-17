# GX Language — VS Code Extension

Syntax highlighting and snippets for the [GX brain-first programming language](https://github.com/elgrhy/gx).

## Features

- Syntax highlighting for `.gx` files
- 12 built-in snippets (`agent`, `helper`, `brain`, `ask`, `function`, `for`, `if`, `try`, `import`, ...)
- Bracket matching and auto-close
- Comment toggling (`//`)

## Install

Install from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=devjsx.gx-language).

Or install the GX toolchain and get syntax highlighting immediately:

```bash
npm install -g gxlang
```

## Snippets

| Prefix | Inserts |
|--------|---------|
| `agent` | Agent with `when started` block |
| `helper` | Full helper with brain cycle |
| `brain` | Brain block (plan/execute/remember/communicate) |
| `ask` | AI call with confidence check |
| `function` | Function definition |
| `for` | For loop |
| `if` | If-else block |
| `try` | Try-catch block |
| `import` | Import another .gx file |
| `use` | Import JS/Python package |
| `emit` | Emit an event |

## License

MIT — © 2025 DEVJSX LIMITED
