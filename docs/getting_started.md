# Getting Started with GX

## Install

```bash
# macOS / Linux (recommended)
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm (any platform with Node.js 16+)
npm install -g gxlang

# From source (requires Rust)
git clone https://github.com/elgrhy/gx.git
cd gx && cargo build --release
sudo cp target/release/gx /usr/local/bin/
```

Verify:
```bash
gx version
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

## The Brain Cycle

Every agent follows four phases: **Plan → Execute → Remember → Communicate**.

In Level 1/2 syntax, GX maps your code to these phases automatically. In Level 3 you control them directly.

```gx
Agent smart

name = "Ahmed"

// Level 2: named behavior
Greet:
  "Hello {name}, welcome back!"

// Level 3: explicit phase
Plan:
  Greet

Communicate:
  "Session complete"
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

// For (also works with 'each')
For each item in items
  log(item)

// Try / catch
try:
  result = risky()
catch e
  log("error: " + e)
```

---

## String Interpolation

```gx
name = "Ahmed"
count = 42
say "Hello {name}, count is {count}"
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

## Multi-file

```gx
import "agents/utils.gx"

Agent app

On start:
  // Uses functions from utils.gx
  greet("Ahmed")
```

---

## Scaffold, Test, Build

```bash
gx init my-agent        # new project
gx run main.gx          # run
gx check main.gx        # syntax check only
gx test                 # run all tests/
gx build main.gx        # build standalone launcher → dist/main
gx fmt main.gx          # format source
```

---

## Next Steps

- [Language Reference](language_reference.md) — complete syntax and built-ins
- [AI Agents](ai_agents.md) — connect to OpenAI, Anthropic, Ollama
- [Examples](examples/) — runnable `.gx` programs
