# GX Language

A brain-first programming language for building transparent, auditable AI assistants.

**The core idea:** Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust, runs anywhere, no cloud lock-in.

---

## Install

```bash
# macOS / Linux
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm (any platform with Node.js)
npm install -g gxlang

# Homebrew (coming after v0.1.0 release)
brew install gx

# From source (requires Rust)
git clone https://github.com/elgrhy/gx.git && cd gx && cargo build --release
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

GX has **three progressive syntax levels** — all compile to the same runtime. Use the level that fits what you're building.

### Level 1 — Pure intent (no ceremony)

```gx
Agent greeter

name = "World"

"Hello {name}"
```

No braces. Variables become memory automatically. Strings auto-print. GX infers the brain cycle.

### Level 2 — Named behaviors

```gx
Agent assistant

Greet:
  "Hello {name}!"

CheckIn:
  result = ask openai { prompt: "How are things with {name}?" }
  result.text

On start:
  Greet
  CheckIn
```

Reusable, composable behavior blocks. Memory is shared between behaviors.

### Level 3 — Explicit brain cycle

```gx
Agent counter

count = 0

Plan:
  action = "increment"

Execute:
  If action == "increment"
    count += 1

Remember:
  memory.count = count

Communicate:
  "Count is now {count}"
```

Full control over Plan → Execute → Remember → Communicate. Still no braces required.

### Classic brace syntax (also fully supported)

```gx
agent "greeter" {
  remember { name = "World" }
  when started { say "Hello, {memory.name}!" }
}
```

---

## AI Primitives

AI is built into the language. Every call is automatically logged.

```gx
agent "assistant" {
  remember {
    history = []
  }

  when started {
    result = ask openai {
      prompt: "Explain quantum computing in one sentence.",
      max_tokens: 100
    }

    if result.confidence > 0.8 {
      say result.text
    } else {
      say "I'm not confident enough — escalating"
      escalate to human
    }
  }
}
```

**Every `ask` call returns:**
- `result.text` — the response
- `result.confidence` — 0.0 to 1.0 (adjusted for hedging language)
- `result.tokens_used` — for cost tracking
- `result.model` / `result.provider`

**Supported providers:** `openai`, `anthropic`, `ollama` (local, no API key needed)

```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# For ollama: brew install ollama && ollama pull llama3
```

---

## Package Interop

Use npm, pip, and cargo packages directly. No ecosystem lock-in.

```gx
use js.path
use py.os

agent "file_agent" {
  when started {
    joined = js.path.join("/home/user", "docs")
    say "Path: {joined}"

    cwd = py.os.getcwd()
    say "CWD: {cwd}"
  }
}
```

Install packages:

```bash
gx install js.axios      # runs: npm install axios
gx install py.requests   # runs: pip install requests
```

---

## CLI Reference

```bash
gx run file.gx                     # Run a GX file
gx run file.gx --debug             # Run with debug output
gx check file.gx                   # Syntax check without running
gx init my-project                 # Scaffold a new project
gx build file.gx                   # Build standalone launcher
gx build file.gx --output dist/app # Build to specific path
gx install js.axios                # Install npm package
gx install py.requests             # Install Python package
gx fmt file.gx                     # Format GX source
gx test                            # Run all tests/
gx test tests/specific.gx         # Run specific test
gx make "a weather bot"            # AI-generate GX code
gx version                         # Print version
gx help                            # Print help
```

---

## Project Structure

After `gx init my-agent`:

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

## Language Reference

### Types
| Type | Example |
|------|---------|
| String | `"hello"`, `"Hi {memory.name}"` |
| Number | `42`, `3.14`, `-7` |
| Bool | `true`, `false` |
| Array | `[1, 2, 3]` |
| Object | `{ key: "value", n: 42 }` |
| Null | `null` |

### Operators
```gx
a + b      // add / concat
a - b      // subtract
a * b      // multiply
a / b      // divide
a % b      // modulo
a == b     // equal
a != b     // not equal
a < b      // less than
a > b      // greater than
a <= b     // less or equal
a >= b     // greater or equal
a and b    // logical and
a or b     // logical or
not a      // logical not
```

### Control Flow
```gx
if condition {
  // ...
} else if other {
  // ...
} else {
  // ...
}

for each item in collection {
  log(item)
}

try {
  risky_call()
} catch e {
  log("Error: " + e)
}
```

### Memory
```gx
// Declare in remember block
remember {
  count = 0
  name = "default"
  items = []
}

// Read anywhere
log(memory.count)

// Write anywhere
memory.count += 1
memory.name = "updated"

// Nested
memory.stats.runs += 1
```

### Built-in Functions
```gx
log("message")               // print to stdout
say "message"                // print to stdout (simple syntax)
get_timestamp()              // current Unix timestamp (ms)
to_string(value)             // convert to string
len("hello")                 // string/array length
```

### String Interpolation
```gx
name = "Ahmed"
say "Hello, {name}!"
say "Count: {memory.count}, time: {get_timestamp()}"
```

### AI Primitives
```gx
// Ask an AI model
result = ask openai { prompt: "your prompt", max_tokens: 200 }
result = ask anthropic { prompt: "your prompt" }
result = ask ollama:llama3 { prompt: "your prompt" }

// Embed text (returns float array)
vector = embed "text to embed"

// Classify
label = infer classifier {
  input: memory.text,
  classes: ["positive", "negative", "neutral"]
}
```

### When Blocks (Simple Syntax)
```gx
when started {
  // runs once on startup, before brain cycle
}

when memory.count > 10 {
  // runs when condition is true
}

when memory.status changes {
  // runs when value changes
  re-run  // restart the brain cycle
}
```

### Escalation
```gx
escalate to human    // emit escalation event and stop brain cycle
```

### Multi-Agent Orchestration (v0.1.5)

Agents can call each other, chain results through pipelines, and pass messages.

**Spawn an agent** (returns the communicate value):
```gx
result = spawn agent "summarizer" with { text: "hello world" }
log(result)  // "'hello world' has 2 word(s)"
```

**Chain calls** (output of one becomes input of next):
```gx
doubled   = spawn agent "doubler"    with { value: 21 }
formatted = spawn agent "formatter" with { value: doubled }
log(formatted)  // "RESULT: 42"
```

**Pipeline with `|>`** (scalar values are auto-wrapped as `{ value: X }`):
```gx
result = { value: 5 } |> spawn agent "doubler" |> spawn agent "formatter"
log(result)  // "RESULT: 10"
```

**Send a message** to a `when message` handler:
```gx
spawn "task" to "worker" with { task: "process data" }
```

**Receive messages** in a helper:
```gx
helper "worker" {
  when message "task" {
    log("Worker received: {message.task}")
  }
}
```

**Define a callable agent** (any helper that reads `input` becomes call-only — it won't auto-run):
```gx
helper "doubler" {
  brain {
    plan { }
    execute { result = input.value * 2 }
    remember { }
    communicate { result }
  }
}
```

---

## Examples

See [`docs/examples/`](docs/examples/) for working examples:

- [`hello_world.gx`](docs/examples/hello_world.gx) — print to stdout
- [`simple_agent.gx`](docs/examples/simple_agent.gx) — `agent` + `when started`
- [`calculator.gx`](docs/examples/calculator.gx) — brain cycle with memory
- [`ai_assistant.gx`](docs/examples/ai_assistant.gx) — `ask openai`, confidence check
- [`package_interop.gx`](docs/examples/package_interop.gx) — `use js.path`, `use py.os`

---

## Architecture

```
src/
├── main.rs           CLI entry point
├── lexer.rs          Tokenizer (brace syntax)
├── parser.rs         AST builder (brace syntax)
├── indent_parser.rs  Parser for progressive indentation syntax
├── ast.rs            AST node types
├── interpreter.rs    Tree-walking executor
├── value.rs          Runtime value types
├── ai.rs             AI provider connectors (OpenAI, Anthropic, Ollama)
├── bridge.rs         JS/Python subprocess IPC
├── toolchain.rs      gx init/build/install/fmt/make/test
└── lib.rs            Public API for embedding
```

**Key design decisions:**
- Tree-walking interpreter (no bytecode) — simple, debuggable, correct
- Flat `memory` scope (no lexical scoping) — predictable for AI agents
- Synchronous execution — no async/await complexity in Phase 1
- Every AI call auto-logged to `memory.ai_trace`
- JS bridge: one-shot `node -e` subprocess per call
- Python bridge: persistent child process with JSON IPC (avoids 200ms startup per call)

---

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 | Rust interpreter — lexer, parser, AST, tree-walker | Done |
| 2 | Simple syntax — `agent`, `when started`, `re-run`, `escalate to human` | Done |
| 3 | AI primitives — `ask`, `embed`, `infer classifier` | Done |
| 4 | Package interop — `use js.X`, `use py.X` | Done |
| 5 | Toolchain — `gx init/build/install/fmt/make/test` | Done |
| 6 | Distribution — curl installer, GitHub Actions CI/release, npm package | Done |
| 7 | Multi-agent orchestration — `spawn agent`, `\|>` pipelines, `when message` | Done |
| 8 | Self-hosting — rewrite GX interpreter in GX itself | Planned |

CI: `cargo test` (40 tests), `cargo clippy -D warnings`, `cargo fmt --check` — all pass on ubuntu/macos/windows.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

MIT

---

**© 2025 DEVJSX LIMITED** — Company No: 16618207, 128 City Road, London EC1V 2NX

**Ahmed Elgarhy** — Founder, DEVJSX | AI Software Architect
