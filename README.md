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

## What's New in v0.2.5

- **HTTPS fix** — `ask openai`, `http_post`, and all HTTP primitives now work reliably on every machine (switched from bundled rustls to the system's native TLS stack)
- **`shell()` stdin** — child processes launched via `shell("...")` now inherit the parent's stdin, so `shell("cat")` with piped input works correctly
- **String object keys** — object literals now accept quoted keys: `{ "Content-Type": "application/json" }` — required for building HTTP headers and working with JSON APIs directly in GX

---

## The Official GX Style (v0.2.0)

GX is opinionated. Every agent follows a clear, readable structure — **goal → observe → think → act → remember → communicate**. You can see exactly what an agent does without running it.

```gx
agent "lead_generator" {
  goal: "Find and contact 10 qualified real-estate leads this week"

  retry: 3
  on_error: escalate

  when started {
    observe {
      context: "Dubai Marina, 2BR, AED 120k budget"
    }

    think {
      prompt: "Extract 10 qualified leads matching: {context}",
      model: "openai",
      min_confidence: 0.82
    }

    act {
      if result.confidence > 0.82 {
        log("Processing {len(result.leads)} leads")
        for each lead in result.leads {
          log("Lead: {lead.name} — {lead.email}")
        }
      } else {
        escalate to human
      }
    }

    remember {
      memory.total_leads += 1
      memory.last_run = get_timestamp()
    }

    communicate {
      say "Processed leads for: {context}"
    }
  }
}
```

Without an AI key, use the built-in brain cycle directly — no `think` required:

```gx
agent "classifier" {
  goal: "Route incoming questions to the right specialist"

  when started {
    question = "Can my employer withhold my gratuity in UAE?"

    category = spawn agent "domain_expert" with { question: question }
    log("Routed to: {category}")
  }
}
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

## GX → JavaScript Transpiler (Self-Hosting)

GX can compile itself to JavaScript. The tokenizer, parser, and evaluator are all written in GX — and the JS transpiler compiles them to a standalone Node.js script that runs `.gx` programs without Rust.

### Bootstrap: GX running GX without Rust

```bash
# Compile the GX self-interpreter to JavaScript (~2 seconds)
gx run self/jsc_bootstrap.gx > gx_interp.js

# Run a GX program using the compiled JS interpreter (no Rust needed)
GX_FILE=self/test_self.gx node gx_interp.js
# → All self-interpreter tests passed!
```

The produced `gx_interp.js` (committed to this repo) is a 1641-line, self-contained Node.js script. It implements the full GX tokenizer, parser, and evaluator in JavaScript — generated by the GX → JS transpiler, which is itself written in GX.

### Compile any GX file to JavaScript

```bash
# Compile hello.gx to hello.js
GX_FILE=hello.gx gx run self/jsc.gx > hello.js

# Run the compiled output
node hello.js
```

The transpiler maps all GX constructs to idiomatic JavaScript — `for each` loops to `for...of`, method chains to runtime helpers, string interpolation to template literals.

### Self-hosting source files

```
self/
├── lexer.gx         GX tokenizer written in GX
├── parser.gx        Recursive descent parser written in GX (functional index-passing style)
├── eval.gx          Tree-walking evaluator written in GX (functional env-threading)
├── js_codegen.gx    GX AST → JavaScript code generator
├── jsc.gx           JS transpiler entry point (compile one file)
├── jsc_bootstrap.gx Bootstrap compiler (uses native parse_gx() for speed)
├── gx_runtime.js    JavaScript runtime prepended to all compiled output
├── gx_main_logic.gx Entry point logic for the compiled interpreter
└── test_self.gx     Test suite for the self-hosted interpreter
```

The committed `gx_interp.js` can be regenerated at any time:

```bash
gx run self/jsc_bootstrap.gx > gx_interp.js
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
| Object | `{ key: "value", "Content-Type": "application/json" }` |
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
a ?? b     // null coalescing — returns b if a is null
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

while count < 10 {
  count += 1
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
range(start, end)            // produce [start, start+1, ..., end-1]
ord("A")                     // character code point
chr(65)                      // code point → character
is_digit("3")                // true/false character tests
is_alpha("a")
is_alnum("z")
is_whitespace(" ")
floor(3.9)                   // math helpers
ceil(3.1)
abs(-5)
sqrt(16.0)
set_key(obj, "k", val)       // return new object with key set (immutable update)
json_parse(str)              // parse JSON string
json_stringify(val)          // serialise to JSON
parse_gx(src)                // parse GX source → AST value (used by the JS transpiler)
read_file(path)              // read file as string
write_file(path, content)    // write string to file
file_exists(path)            // boolean
env("VAR")                   // read environment variable
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
├── interpreter.rs    Tree-walking executor + parse_gx() builtin
├── value.rs          Runtime value types
├── ai.rs             AI provider connectors (OpenAI, Anthropic, Ollama)
├── bridge.rs         JS/Python subprocess IPC
├── toolchain.rs      gx init/build/install/fmt/make/test
└── lib.rs            Public API for embedding

self/                 GX self-hosting layer (the language written in itself)
├── lexer.gx          Tokenizer written in GX
├── parser.gx         Parser written in GX (functional index-passing style)
├── eval.gx           Evaluator written in GX (functional env-threading)
├── js_codegen.gx     GX AST → JavaScript transpiler
├── jsc.gx            Single-file JS compile entry point
├── jsc_bootstrap.gx  Bootstrap compiler using native parse_gx()
├── gx_runtime.js     JS runtime library prepended to all compiled output
├── gx_main_logic.gx  Self-interpreter entry point logic
└── test_self.gx      Self-interpreter test suite

gx_interp.js          Compiled GX interpreter — runs .gx files via Node.js (no Rust)
```

**Key design decisions:**
- Tree-walking interpreter (no bytecode) — simple, debuggable, correct
- Flat `memory` scope (no lexical scoping) — predictable for AI agents
- Synchronous execution — no async/await complexity in Phase 1
- Every AI call auto-logged to `memory.ai_trace`
- JS bridge: one-shot `node -e` subprocess per call
- Python bridge: persistent child process with JSON IPC (avoids 200ms startup per call)
- GX passes all values by copy — the self-hosting evaluator threads `env` and `fns` explicitly through every function call
- `parse_gx(src)` builtin runs the Rust parser at native speed, returning a GX value tree for the JS transpiler — bypasses the O(n²) token-clone cost of the GX-written parser

---

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 | Rust interpreter — lexer, parser, AST, tree-walker | ✅ Done |
| 2 | Simple syntax — `agent`, `when started`, `re-run`, `escalate to human` | ✅ Done |
| 3 | AI primitives — `ask`, `embed`, `infer classifier` | ✅ Done |
| 4 | Package interop — `use js.X`, `use py.X` | ✅ Done |
| 5 | Toolchain — `gx init/build/install/fmt/make/test` | ✅ Done |
| 6 | Distribution — curl installer, GitHub Actions CI/release, npm package | ✅ Done |
| 7 | Multi-agent orchestration — `spawn agent`, `\|>` pipelines, `when message` | ✅ Done |
| 8 | Opinionated sugar — `goal`, `think`, `act`, `observe`, `loop until`, `repeat N times`, `parallel`, `retry`, `timeout`, `on_error` | ✅ Done |
| 9 | Native tools — `http_request`, `send_email`, `scrape`, `notify`, `read/write_file`, `json_parse`, `ord`/`chr`/`is_digit` | ✅ Done |
| 10 | Self-hosting — GX interpreter written in GX, GX → JS transpiler, Node.js bootstrap | ✅ Done |

CI: `cargo test` (40 tests), `cargo clippy -D warnings`, `cargo fmt --check` — all pass on ubuntu/macos/windows.

---

## Self-Hosting Details

Phase 10 is complete. Here is what "self-hosting" means for GX:

**Stage 1 — Self-interpreter:** `self/lexer.gx` + `self/parser.gx` + `self/eval.gx` implement the full GX tokenizer, parser, and evaluator in GX itself. Running `gx run self/main.gx` interprets a `.gx` file using only GX code (the Rust runtime only drives the outer loop).

**Stage 2 — JS transpiler:** `self/js_codegen.gx` + `self/jsc.gx` compile GX AST nodes to JavaScript source. Every GX construct maps to idiomatic JS — `for each` → `for...of`, null coalescing → `??`, string interpolation → template literals, all builtins (`len`, `range`, `set_key`, etc.) → typed JS runtime helpers in `self/gx_runtime.js`.

**Stage 3 — Bootstrap:** `self/jsc_bootstrap.gx` compiles the entire self-hosting stack to a single JavaScript file. The `parse_gx(src)` builtin runs the native Rust parser at full speed and returns the AST as a GX value tree, feeding it directly into `js_codegen.gx` — this eliminates the O(n²) token-clone overhead that would arise from passing large arrays through the GX-written parser's recursive functions.

```
gx run self/jsc_bootstrap.gx > gx_interp.js   # 2 seconds, 1641 lines of JS
GX_FILE=self/test_self.gx node gx_interp.js    # All self-interpreter tests passed!
```

`gx_interp.js` is committed to the repo. It is a self-contained Node.js GX interpreter — no Rust, no native dependencies.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

MIT

---

**© 2025 DEVJSX LIMITED** — Company No: 16618207, 128 City Road, London EC1V 2NX

**Ahmed Elgarhy** — Founder, DEVJSX | AI Software Architect
