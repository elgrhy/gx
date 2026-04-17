# GX Language

> A brain-first programming language for building transparent, auditable AI assistants.

[![npm version](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://github.com/elgrhy/gx)

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust. No cloud lock-in.

---

## Install

```bash
npm install -g gxlang
```

That's it. The installer downloads the correct pre-built binary for your platform (macOS, Linux, or Windows).

Verify:

```bash
gx version
```

---

## Quick Start

```bash
gx init my-agent
cd my-agent
gx run main.gx
```

---

## The Language

GX has **three syntax levels** — all run on the same interpreter. Pick the level that fits what you're building.

### Level 1 — Pure intent (3 lines, no ceremony)

```gx
Agent greeter

name = "World"

"Hello {name}"
```

Variables become memory automatically. Strings auto-print. GX infers everything else.

### Level 2 — Named behaviors

```gx
Agent assistant

user = "Ahmed"

Greet:
  say "Hello {user}!"

CheckIn:
  result = ask openai { prompt: "What should {user} focus on today?" }
  say result.text

On start:
  Greet
  CheckIn
```

Behaviors are reusable labeled blocks. Memory is shared between all of them.

### Level 3 — Explicit brain cycle

```gx
Agent analyzer

input = "quarterly revenue data"

Plan:
  action = "analyze"

Execute:
  If action == "analyze"
    result = ask anthropic { prompt: "Analyze this: {input}" }

Remember:
  memory.last_result = result.text

Communicate:
  If result.confidence > 0.8
    say result.text
  Else
    escalate to human
```

Full control over Plan → Execute → Remember → Communicate.

### Classic brace syntax (also supported)

```gx
agent "greeter" {
  remember { name = "World" }
  when started { say "Hello, {memory.name}!" }
}
```

---

## AI Primitives

AI is built into the language — not a library, not a wrapper. Every call is automatically logged.

```gx
agent "assistant" {
  when started {
    result = ask openai {
      prompt: "Summarize this in one sentence: {memory.text}",
      max_tokens: 100
    }

    if result.confidence > 0.8 {
      say result.text
    } else {
      say "Not confident enough — escalating"
      escalate to human
    }
  }
}
```

**Every `ask` returns:**

| Field | Type | Description |
|-------|------|-------------|
| `result.text` | String | The model's response |
| `result.confidence` | Number | 0.0–1.0, adjusted for hedging language |
| `result.tokens_used` | Number | Tokens consumed (for cost tracking) |
| `result.model` | String | Model name used |
| `result.provider` | String | `openai`, `anthropic`, or `ollama` |

**Supported providers:**

```gx
result = ask openai    { prompt: "..." }   // GPT-4o-mini
result = ask anthropic { prompt: "..." }   // Claude Haiku
result = ask ollama    { prompt: "..." }   // Local Llama 3 (no API key)
```

**Set your API keys:**

```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# For ollama: brew install ollama && ollama serve
```

---

## Use Any npm or Python Package

```gx
use js.axios
use js.path
use py.pandas
use py.os

agent "fetcher" {
  when started {
    data   = js.axios.get("https://api.example.com/data")
    joined = js.path.join("/output", "results.json")
    cwd    = py.os.getcwd()
    say "Fetched data, saving to {joined} from {cwd}"
  }
}
```

Install packages:

```bash
gx install js.axios       # runs: npm install axios
gx install py.requests    # runs: pip install requests
```

---

## Functions and Imports

```gx
// math.gx
function square(n) {
  return n * n
}

// main.gx
import "math.gx"

agent "calc" {
  when started {
    say square(9)    // 81
  }
}
```

---

## CLI Reference

| Command | Description |
|---------|-------------|
| `gx run file.gx` | Run a GX file |
| `gx run file.gx --debug` | Run with debug output |
| `gx check file.gx` | Syntax check without running |
| `gx init my-project` | Scaffold a new project |
| `gx build file.gx` | Build a standalone launcher |
| `gx install js.axios` | Install an npm package for use in GX |
| `gx install py.requests` | Install a Python package for use in GX |
| `gx fmt file.gx` | Format GX source |
| `gx test` | Run all files in `tests/` |
| `gx make "a weather bot"` | AI-generate GX code |
| `gx version` | Print version |
| `gx help` | Print help |

---

## Project Structure

```
my-agent/
├── gx.json          # Project manifest
├── main.gx          # Entry point
├── agents/          # Additional agent files
├── tests/           # Test files (run with gx test)
└── .gitignore
```

`gx.json`:
```json
{
  "name": "my-agent",
  "version": "0.1.0",
  "entry": "main.gx",
  "dependencies": {
    "js": [],
    "py": [],
    "gx": []
  }
}
```

---

## Memory — Fully Auditable State

Every agent has a flat `memory` scope. All AI calls are automatically appended to `memory.ai_trace`.

```gx
agent "tracker" {
  remember {
    count = 0
    history = []
  }

  when started {
    memory.count += 1
    memory.history = memory.history.push("run #{memory.count}")
    log(memory.ai_trace)    // see every AI call ever made
  }
}
```

---

## When Blocks

```gx
when started {
  // runs once on startup
}

when memory.count > 10 {
  // runs when condition is true after each brain cycle
}

when memory.status changes {
  re-run    // restart the brain cycle
}
```

---

## Escalation

```gx
if result.confidence < 0.6 {
  escalate to human    // stops the agent, surfaces for human review
}
```

---

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| macOS | arm64 (Apple Silicon) | Supported |
| macOS | x86_64 (Intel) | Supported |
| Linux | x86_64 | Supported |
| Linux | arm64 | Supported |
| Windows | x86_64 | Supported |

---

## More

- **GitHub:** https://github.com/elgrhy/gx
- **Language Reference:** https://github.com/elgrhy/gx/blob/main/docs/language_reference.md
- **Examples:** https://github.com/elgrhy/gx/tree/main/docs/examples
- **Getting Started:** https://github.com/elgrhy/gx/blob/main/docs/getting_started.md
- **GX vs Python/JS/Rust:** https://github.com/elgrhy/gx/blob/main/docs/compare.md

---

## License

MIT — © 2025 [DEVJSX LIMITED](https://devjsx.com) · Ahmed Elgarhy
