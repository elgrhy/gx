# GX Language — Master Build Plan

> **Goal:** Make building AI assistants as easy as telling a story.
> A 7-year-old should be able to build a complex AI agent. A machine should be able to write it too.

---

## The Problem GX Solves

Every AI assistant today is a black box. You call an API, something comes back, you hope it's right. You can't audit it, test it properly, or trace why it made a decision. GX changes that:

- Every decision is **explicit** — written in readable code
- Every AI call is **logged** — inputs, outputs, confidence, all in memory
- Every agent is **auditable** — no hidden state
- The language itself is **independent** — like Rust, no cloud dependency, runs anywhere

---

## Current State

| Component | Status | Notes |
|-----------|--------|-------|
| Language syntax | Defined | Well-designed, consistent |
| Example programs | Written | 50+ .gx files as design docs |
| Runtime (`bin/gx`) | Stub only | Reads files, counts patterns, prints fake success |
| Real interpreter | Not started | **This is the first thing to build** |
| AI primitives | Designed | Not executable |
| Package interop | Planned | Not started |
| Toolchain | Planned | Not started |

---

## Phase 1: Make GX Real (Rust Interpreter)
**Goal:** `gx run hello_world.gx` actually works

### What to Build
A tree-walking interpreter in Rust. No bytecode yet — keep it simple.

```
gx/
├── Cargo.toml
├── src/
│   ├── main.rs          CLI: gx run <file>, gx check <file>
│   ├── lexer.rs         Tokenizer: helper, brain, plan, execute, etc.
│   ├── parser.rs        AST builder
│   ├── ast.rs           Node types: Helper, Brain, Recipe, Memory, etc.
│   ├── interpreter.rs   Tree walker, executes helpers
│   ├── memory.rs        Scoped key-value store per helper
│   ├── channels.rs      emit / receive / channel message passing
│   ├── builtins.rs      log(), output(), get_timestamp(), count(), etc.
│   └── error.rs         Friendly error messages with line numbers
```

### Milestones
- [ ] M1: Lexer tokenizes a .gx file correctly
- [ ] M2: Parser builds an AST for `hello_world.gx`
- [ ] M3: Interpreter runs `hello_world.gx` — prints the greeting
- [ ] M4: `memory {}` block works — variables stored and read
- [ ] M5: `brain { plan {} execute {} remember {} communicate {} }` works
- [ ] M6: `recipe` works (named functions with inputs/outputs)
- [ ] M7: `emit` and `receive` channel messaging works
- [ ] M8: `for each`, `if/else`, `try/catch` control flow works
- [ ] M9: Error messages show file + line number
- [ ] M10: `cargo test` suite with 20+ tests

### Definition of Done
```bash
gx run docs/examples/hello_world.gx
# Output: Hello, Brain-First World!

gx run docs/examples/calculator.gx
# Output: (actual calculation results)

gx check myfile.gx
# Output: No errors found / Error at line 12: ...
```

---

## Phase 2: Simple Syntax — For Humans and Machines
**Goal:** A 7-year-old can write a working AI agent. An AI model can generate GX code easily.

### Two Syntax Levels

GX supports two ways to write the same thing. Both compile to the same AST.

**Standard syntax** (current, full control):
```gx
helper "weather_bot" {
  remember { city = "London" }
  brain {
    plan { plan = { action: "get_weather" } }
    execute {
      if plan.action == "get_weather" {
        result = fetch_weather(memory.city)
      }
    }
    remember { memory.last_result = result }
    communicate { emit "weather_ready" { data: result } }
  }
}
```

**Simple syntax** (new, for beginners and AI generation):
```gx
agent "weather bot" {
  remember city = "London"

  when started {
    get weather for memory.city
    say "The weather in {memory.city} is {result}"
  }

  when memory.city changes {
    re-run
  }
}
```

**Natural language mode** (AI-assisted, via CLI):
```bash
gx make "a weather bot that checks my city every morning and texts me"
# GX generates the full agent code and asks you to review it
```

### Simple Syntax Keywords

| Simple | Full Equivalent | Meaning |
|--------|----------------|---------|
| `agent` | `helper` | Define an agent |
| `remember x = y` | `remember { x = y }` | Store a value |
| `when started` | `brain { plan { action: "start" } execute { ... } }` | On boot |
| `when X` | `objective` or `receive` | Trigger on condition |
| `say X` | `emit / output()` | Output to user |
| `ask ai "..."` | AI primitive | Call an AI model |
| `use js.X` | package import | Import npm package |
| `use py.X` | package import | Import Python package |
| `escalate to human` | emit escalation event | Hand off to human |
| `re-run` | restart brain cycle | Loop the brain |

---

## Phase 3: AI Primitives (The Core Differentiator)
**Goal:** Building a non-hallucinating, auditable AI assistant is 5 lines of GX.

### Built-in AI Keywords

```gx
// Ask any AI model — ALL calls auto-logged to memory
result = ask openai {
  prompt: "What is the weather in {memory.city}?",
  context: memory.conversation_history
}
// result.text        — the answer
// result.confidence  — 0.0 to 1.0
// result.trace       — full audit log (auto-saved)
// result.tokens_used — cost tracking

// Ask a local model (no cloud dependency)
result = ask ollama:llama3 {
  prompt: "Summarize: {memory.document}"
}

// Embed for semantic search
vector = embed memory.user_message

// Classify
intent = infer classifier {
  input: memory.user_message,
  classes: ["question", "complaint", "compliment"]
}

// Check confidence before using result
if result.confidence < 0.75 {
  say "I'm not confident enough — let me escalate"
  escalate to human
}
```

### Anti-Hallucination Pattern (built into the language)

```gx
agent "fact_checker" {
  when user sends message {
    ask ai memory.user_message
    
    if result.confidence < 0.8 {
      verify result with second_source
      if still_uncertain {
        say "I don't know — here's what I found: {result.sources}"
      }
    } else {
      say result.text
    }
    
    // Everything above is automatically in memory.trace
  }
}
```

### Supported AI Connectors
- `openai` — GPT-4o, GPT-4, etc.
- `anthropic` — Claude models
- `ollama` — Any local model (Llama, Mistral, etc.) — **no cloud needed**
- `huggingface` — Open source models
- `custom` — BYO API endpoint

---

## Phase 4: Package Interop
**Goal:** Use every npm, pip, and cargo package from GX. Zero ecosystem lock-in.

```gx
// Import packages at file top
use js.axios          // npm: axios
use js.lodash         // npm: lodash
use py.pandas         // pip: pandas
use py.requests       // pip: requests
use rust.serde        // cargo: serde

helper "data_analyst" {
  brain {
    execute {
      // Call npm package — returns GX-native value
      response = js.axios.get("https://api.example.com/data")
      
      // Call Python package
      df = py.pandas.DataFrame(response.data)
      summary = df.describe()
      
      // Use result in GX normally
      memory.analysis = summary
    }
  }
}
```

### How Interop Works
- **JS bridge:** GX spawns a Node.js child process, calls functions via JSON IPC
- **Python bridge:** GX calls Python via PyO3 (embedded Python) or subprocess
- **Rust bridge:** Native linking via Rust FFI in the GX interpreter itself
- All bridges are async-safe and results are converted to GX native types

---

## Phase 5: Toolchain
**Goal:** `brew install gx` and you're productive in 5 minutes.

### CLI Commands
```bash
gx run file.gx              # Run a GX file
gx check file.gx            # Type-check without running
gx build file.gx            # Compile to standalone binary
gx init my-agent            # Scaffold a new GX project
gx install axios            # Install a js/py/rust package for use in GX
gx make "a weather bot"     # AI-generate GX code from description
gx test                     # Run all tests in tests/
gx fmt file.gx              # Format GX code
gx doc file.gx              # Generate documentation
```

### Project Structure (after `gx init`)
```
my-agent/
├── gx.json          # Project manifest (name, version, dependencies)
├── main.gx          # Entry point
├── agents/          # Your helper/agent files
├── tests/           # Test files
└── .gxignore
```

### `gx.json` Example
```json
{
  "name": "my-weather-agent",
  "version": "1.0.0",
  "description": "A weather assistant",
  "entry": "main.gx",
  "dependencies": {
    "js": ["axios", "lodash"],
    "py": ["requests"],
    "rust": [],
    "gx": ["gx-stdlib/0.1.0"]
  }
}
```

---

## Phase 6: Distribution & Community
**Goal:** Anyone in the world can install GX in 30 seconds.

### Install Methods
```bash
# macOS / Linux (recommended)
curl -sSf https://gxlang.dev/install | sh

# macOS Homebrew
brew install gx

# npm (familiar to web devs)
npm install -g gxlang

# Windows
winget install gxlang

# Docker
docker run -it gxlang/gx run main.gx
```

### Deployment Options for GX Programs
```bash
# Build a standalone binary (no GX runtime needed on target machine)
gx build main.gx --target linux-x64
gx build main.gx --target macos-arm64
gx build main.gx --target windows-x64

# Deploy as a Docker container
gx docker main.gx > Dockerfile

# Deploy to cloud (GX Cloud — future)
gx deploy main.gx
```

### CI/CD (GitHub Actions)
```yaml
# .github/workflows/gx.yml
name: GX CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: gxlang/setup-gx@v1
      - run: gx test
      - run: gx build main.gx
```

### Community
- **gxlang.dev** — website with live playground
- **pkg.gxlang.dev** — package registry
- **GitHub Discussions** — community Q&A
- **VS Code Extension** — syntax highlighting, AI autocomplete, inline docs
- **Discord** — community chat

---

## Self-Hosting Plan (Phase 7 — Future)
Once the Rust interpreter is stable:
1. Write a GX lexer in GX → runs on the Rust interpreter
2. Write a GX parser in GX → runs on the Rust interpreter
3. Write a GX interpreter in GX → bootstraps itself
4. GX can now compile itself — true self-hosting achieved
5. Rust interpreter becomes just the seed (like GCC's C bootstrap)

---

## Priorities Summary

| Phase | What | Why It Matters |
|-------|------|---------------|
| 1 | Rust interpreter | Nothing works without this |
| 2 | Simple syntax | Makes it accessible to everyone |
| 3 | AI primitives | The core product differentiator |
| 4 | Package interop | Zero lock-in, use the whole ecosystem |
| 5 | Toolchain | Developer experience determines adoption |
| 6 | Distribution | Reach determines impact |
| 7 | Self-hosting | The long-term vision |

---

*GX is built by DEVJSX LIMITED, London, UK. Founded by Ahmed Elgarhy.*
