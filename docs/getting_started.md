# Getting Started with GX

## Install

```bash
# macOS / Linux (recommended)
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm (any platform with Node.js 16+)
npm install -g gxlang

# Cargo
cargo install gxlang

# From source (requires Rust)
git clone https://github.com/elgrhy/gx.git
cd gx && cargo build --release
sudo cp target/release/gx /usr/local/bin/
```

Verify:
```bash
gx version   # gx 0.6.1
```

---

## Three Ways to Write GX

GX has **three progressive syntax levels** — all compile to the same runtime. Start simple, add structure when you need it.

### Level 1 — Pure intent

```gx
Agent greeter

name = "World"

"Hello {name}"
```

```bash
gx run hello.gx
# Hello World
```

No braces, no ceremony. Variables become memory. Strings auto-print.

---

### Level 2 — Named behaviors

```gx
Agent assistant

topic = "weather"

Greet:
  "Hello! I know about {topic}."

Answer:
  result = ask openai {
    prompt: "Tell me about {topic} in one sentence."
  }
  result.text

On start:
  Greet
  Answer
```

Behaviors (`Greet:`, `Answer:`) are named, reusable blocks. `On start:` runs them in order.

---

### Level 3 — Full brain cycle

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

Explicit `Plan → Execute → Remember → Communicate` phases. Still no braces.

---

### Classic syntax (still fully supported)

```gx
agent "greeter" {
  remember {
    name = "World"
  }

  when started {
    say "Hello, {memory.name}!"
  }
}
```

---

## Inline Eval — No File Needed

Run GX snippets directly without creating a file:

```bash
gx -e 'say "Hello from GX"'
gx -e 'say sha256("hello world")'
gx -e 'say uuid()'
gx -e 'say token_count("how many tokens is this text?")'
```

---

## Create a Project

```bash
gx init my-project
cd my-project
gx run main.gx
```

This creates:
```
my-project/
├── gx.json       # Project config
├── main.gx       # Entry point
├── agents/       # Put your agents here
└── tests/        # Run with: gx test
```

---

## The Brain Cycle (optional)

GX agents can run with simple `when started` blocks, message handlers, or progressive syntax — no brain cycle required. The brain cycle (`Plan → Execute → Remember → Communicate`) is an explicit structure you opt into when you need full control over the decision loop.

```gx
// No brain cycle needed — this is a complete agent
Agent greeter

On start:
  "Hello, world!"
```

When you want explicit phases:

```gx
Agent smart

name = "Ahmed"

Plan:
  Greet

Communicate:
  "Session complete"

Greet:
  "Hello {name}, welcome back!"
```

---

## Memory

Any variable you assign at the agent level becomes persistent memory. Access it anywhere with the bare name or `memory.key`.

```gx
Agent memo

runs = 0
items = []
config = { debug: false }

On start:
  runs += 1
  say "Run {runs}"
```

---

## Control Flow

```gx
// If / else if / else
If score > 90
  say "excellent"
Else if score > 60
  say "ok"
Else
  say "needs work"

// For loop
For item in items
  log(item)

// While
while running {
  line = readline()
  if line == null { break }
  process(line)
}

// Try / catch
try {
  result = risky()
} catch e {
  log("error: " + e)
}
```

---

## String Interpolation

```gx
name = "Ahmed"
count = 42
say "Hello {name}, count is {count}"
say "Literal brace: {{name}}"   // outputs: Literal brace: {name}
```

---

## Functions

```gx
function add(a, b) {
  return a + b
}

Agent calc

On start:
  result = add(3, 4)
  say "3 + 4 = {result}"
```

---

## Stdlib Builtins (v0.5.0)

```gx
use std.crypto
use std.fs
use std.net

// Crypto
h = sha256("hello world")       // SHA-256 hex
id = uuid()                     // UUID v4

// Path / FS
dir  = dirname("/a/b/c.txt")    // "/a/b"
file = basename("/a/b/c.txt")   // "c.txt"
path = path_join("a", "b")      // "a/b"
hits = glob("data/*.csv")       // array of matching paths

// URL
u = url_parse("https://api.example.com:8080/v1?q=gx")
u.host   // "api.example.com"
u.port   // "8080"
u.query  // "q=gx"

// Token tracking
say "Used {tokens_used()} tokens so far"
```

---

## Multi-file

```gx
import "agents/utils.gx"

Agent app

On start:
  greet("Ahmed")   // function from utils.gx
```

---

## Scaffold, Test, Build

```bash
gx init my-agent        # new project
gx run main.gx          # run
gx -e 'say "hi"'        # inline eval
gx check main.gx        # syntax check only
gx test                 # run all tests/
gx build main.gx        # build standalone launcher → dist/main
gx fmt main.gx          # format source
gx repl                 # interactive REPL
```

---

## Multi-Agent Orchestration

Agents can call each other, chain through pipelines, and exchange messages.

```gx
// Call an agent and get its result
doubled = spawn agent "doubler" with { value: 21 }

// Chain via pipeline
result = { value: 5 } |> spawn agent "doubler" |> spawn agent "formatter"

// Send a message to another agent's when message handler
spawn "task" to "worker" with { task: "process data" }
```

---

## Next Steps

- [Language Reference](language_reference.md) — complete syntax and built-ins
- [AI Agents](ai_agents.md) — connect to OpenAI, Anthropic, Ollama
- [Examples](examples/) — runnable `.gx` programs
