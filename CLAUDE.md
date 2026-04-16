# GX Language — Claude Code Context

## What is GX?

GX is a brain-first programming language designed to make building AI assistants as easy as writing a recipe — accessible to a 7-year-old, powerful enough for enterprise production systems.

**Core philosophy:**
- AI agents should be transparent, auditable, and non-hallucinating
- Building AI should feel natural, not technical — humans AND machines write GX
- GX is not starting from zero: it can use packages from JS (npm), Python (pip), and Rust (crates)
- The produced AI assistant has no black box — every decision is visible and traceable

**Owner:** Ahmed Elgarhy, Founder of DEVJSX LIMITED (London, UK). Company No: 16618207.

---

## Current State (Honest)

The language syntax is well-defined. The runtime (`bin/gx`) is a **stub** — it reads .gx files and counts patterns but does NOT execute any GX code. No lexer, parser, AST, or interpreter exists yet.

All `.gx` files in the repo (runtime, compiler, AI assistant, etc.) are design documents written in GX syntax. They cannot run yet.

**The task is to build a real interpreter in Rust, then make GX self-hosting.**

---

## Architecture Plan

```
Phase 1: Rust Interpreter (makes GX real)
    src/
    ├── lexer.rs         — tokenize .gx source
    ├── parser.rs        — build AST
    ├── ast.rs           — AST node types
    ├── interpreter.rs   — tree-walking executor
    ├── memory.rs        — per-helper scoped memory
    ├── channels.rs      — message passing between helpers
    ├── builtins.rs      — log(), output(), get_timestamp(), etc.
    └── main.rs          — CLI entry point (gx run file.gx)

Phase 2: AI Primitives (the core differentiator)
    — ask, infer, embed as native keywords
    — every AI call auto-logged to helper memory
    — confidence scoring built in
    — connectors: OpenAI, Anthropic, Ollama (local)

Phase 3: Package Interop (use everything, start from nothing)
    — js: prefix → calls npm packages via Node.js bridge
    — py: prefix → calls Python packages via subprocess/PyO3
    — rust: prefix → links Rust crates natively
    — Example: use js.axios, use py.pandas, use rust.tokio

Phase 4: Simple Syntax (7-year-old mode)
    — Short-form syntax for common patterns
    — AI-assisted code generation built into the CLI
    — Natural language → GX code: `gx make "a weather bot"`

Phase 5: Toolchain & Deployment
    — gx init, gx run, gx build, gx install
    — brew install gx / curl installer / npm install -g gxlang
    — VS Code extension with syntax + AI autocomplete
    — gxlang.dev with live playground
```

---

## GX Language Syntax Reference

### Full Syntax (current, expressive)

```gx
helper "agent_name" {
  can_do: ["capability_1", "capability_2"]

  remember {
    key = value
  }

  receive {
    channel "input_channel" {
      source: "some_helper"
      type: "message_type"
      bind: memory.variable
      on_receive: brain.handler_name
    }
  }

  brain {
    plan {
      if memory.condition {
        plan = { action: "do_something" }
      }
    }

    execute {
      if plan.action == "do_something" {
        result = do_something(memory.key)
      }
    }

    remember {
      memory.last_result = result
    }

    communicate {
      emit "event_name" { data: result }
    }
  }

  recipe "function_name" {
    needs: input_var
    gives: output_var
    brain { ... }
  }

  objective "goal_name" {
    when memory.condition == true
    then { action: "trigger_action" }
  }
}
```

### Simple Syntax (target — Phase 4)

```gx
agent "weather bot" {
  knows {
    city = "London"
  }

  when asked for weather {
    check city from user or use memory.city
    ask ai "what is the weather in {city}?"
    say result
  }

  when result.confidence < 0.8 {
    say "I'm not sure, let me check again"
    escalate to human
  }
}
```

### AI Primitives (Phase 2)

```gx
// Ask an AI model — result is always logged to memory
result = ask openai {
  prompt: "Summarize this: {memory.text}",
  context: memory.conversation_history,
  max_tokens: 200
}

// result.text       — the response
// result.confidence — how confident (0.0 to 1.0)
// result.trace      — full audit log, auto-saved to memory

// Embed text for semantic search
vector = embed "this is a document about space exploration"

// Classify input
label = infer classifier { input: user_message, classes: ["support", "sales", "spam"] }
```

### Package Interop (Phase 3)

```gx
// Use npm packages
use js.axios
use js.lodash

// Use Python packages
use py.pandas
use py.sklearn

// Use Rust crates
use rust.serde
use rust.tokio

helper "data_agent" {
  brain {
    execute {
      // Call npm package directly
      data = js.axios.get("https://api.example.com/data")

      // Call Python package
      df = py.pandas.read_csv("data.csv")
      model = py.sklearn.linear_model.LinearRegression()

      // Use in GX logic normally
      memory.processed = df.head(10)
    }
  }
}
```

---

## Key Files

| File | Purpose |
|------|---------|
| `build/gx_minimal.c` | The current stub — starting point for Rust port |
| `bin/gx` | Built binary (stub only, does not execute GX) |
| `docs/examples/hello_world.gx` | Simplest valid GX program |
| `docs/examples/calculator.gx` | Basic helper with brain cycle |
| `gx_ai_assistant.gx` | AI assistant design (not executable yet) |
| `gx_runtime.gx` | Runtime design (not executable yet) |
| `gx_compiler_implementation.gx` | Compiler design (not executable yet) |
| `MASTER_PLAN.md` | Full build roadmap |

---

## What NOT to Do

- Do not claim any existing .gx files "work" — they are design docs, not running code
- Do not extend the C stub — we are moving to Rust
- Do not add features before the core interpreter can run `hello_world.gx`
- Do not over-engineer: get `hello_world.gx` working first, then build up

## What to Do Next

1. `cargo init` in the repo root and build the Rust interpreter
2. Get `docs/examples/hello_world.gx` printing "Hello, Brain-First World!" for real
3. Then add brain cycle, memory, recipes
4. Then AI primitives
5. Then package interop

---

## Definition of Done for Phase 1

- [ ] `gx run docs/examples/hello_world.gx` prints the greeting
- [ ] `gx run docs/examples/calculator.gx` runs the brain cycle
- [ ] Helpers, brain (plan/execute/remember/communicate), memory, log() all work
- [ ] Error messages show file name and line number
- [ ] `cargo test` passes

## Testing Strategy

Every new language feature gets a `.gx` test file in `tests/` plus a Rust unit test.
Run all tests: `cargo test && gx run tests/run_all.gx`
