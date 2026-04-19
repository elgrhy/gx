# GX Language — Reference

Complete reference for GX syntax, built-in functions, and AI primitives.

---

## Syntax Levels

GX has three progressive syntax levels that all compile to the same runtime. Use the level that matches the complexity of what you're building.

### Level 1 — Pure intent

No braces, no ceremony. Variables at agent level become memory automatically. String literals auto-print.

```gx
Agent greeter

name = "World"

"Hello {name}"
```

- `Agent Name` (no quotes, no braces) declares an agent
- Variable assignments → stored in `memory`
- String expressions → printed to stdout
- GX infers the brain cycle automatically

### Level 2 — Named behaviors

Reusable labeled blocks. Behaviors share memory with the agent.

```gx
Agent assistant

user = "Ahmed"

Greet:
  say "Hello {user}!"

AskQuestion:
  say "What do you need today, {user}?"

On start:
  Greet
  AskQuestion
```

- `BehaviorName:` followed by indented body → named behavior (zero-arg function)
- Calling `BehaviorName` (no parens) in another block → executes the behavior
- `On start:` → runs on agent startup
- Memory is shared: changes inside a behavior are visible to the caller

### Level 3 — Explicit brain cycle

Full control over Plan → Execute → Remember → Communicate. No braces required.

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

- `Plan:`, `Execute:`, `Remember:`, `Communicate:` → explicit brain phases
- `If`, `For`, `Try` use indentation instead of braces
- `Else` at same indentation as the preceding `If`

### Classic brace syntax

The original syntax is fully supported alongside progressive syntax.

```gx
agent "greeter" {
  remember { name = "World" }
  when started { say "Hello, {memory.name}!" }
}
```

GX auto-detects which syntax to use based on the first meaningful line.

---

## Top-Level Structure

### `helper` / `agent`

Both keywords define the same thing — a cognitive agent. `agent` is the simple-syntax alias.

```gx
helper "name" {
  remember { ... }
  receive { ... }
  brain { ... }
  when trigger { ... }
}

agent "name" {
  remember { ... }
  when trigger { ... }
}
```

### `function` — User-Defined Functions

```gx
function add(a, b) {
  return a + b
}

function greet(name) {
  return "Hello, " + name + "!"
}

// Recursive
function factorial(n) {
  if n <= 1 { return 1 }
  return n * factorial(n - 1)
}

result = add(3, 4)    // 7
msg    = greet("GX")  // "Hello, GX!"
```

Functions are defined at the top level (outside agents). They can be called from anywhere — `when` blocks, `brain` phases, other functions.

### `import` — File Import

```gx
import "agents/utils.gx"
import "lib/math.gx"
```

Loads functions and agents from another `.gx` file. All functions defined in the imported file become available in the current file. Paths are relative to the current working directory.

### `use` — Package Import

```gx
use js.axios        // npm package
use py.requests     // Python package
use js.lodash
use py.pandas
```

After import, the package is callable as `namespace.module.method(args)`:
```gx
use js.path
result = js.path.join("/home", "user")
```

---

## Blocks

### `remember` — Memory Initialization

```gx
remember {
  count = 0
  name = "default"
  items = []
  config = { timeout: 5000 }
}
```

Values declared here are accessible as `memory.key` everywhere in the helper.

### `brain` — Cognitive Cycle

```gx
brain {
  plan {
    // Decide what to do
    plan = { action: "process" }
  }
  execute {
    // Do it
    if plan.action == "process" {
      result = 42
    }
  }
  remember {
    // Store results
    memory.last_result = result
  }
  communicate {
    // Emit events
    emit "done" { value: memory.last_result }
  }
}
```

The brain cycle runs until it finishes or a `re-run` is encountered (which restarts the cycle, max 100 iterations).

### `when` — Trigger Blocks

```gx
// Runs once on startup
when started {
  say "Agent is ready"
}

// Runs if the condition is true after the brain cycle
when memory.count > 10 {
  log("Threshold reached")
}

// Runs when the value changes
when memory.status changes {
  re-run
}

// Receives a message sent by another agent
when message "task" {
  log("Received task: {message.task}")
}
```

The brain cycle is **optional**. Agents can rely entirely on `when` blocks without a `brain` block.

### `receive` — Channel Bindings

```gx
receive {
  channel "input" {
    source: "other_helper"
    type: "message"
    bind: memory.incoming
    on_receive: brain.handler
  }
}
```

---

## Multi-Agent Orchestration

### `spawn agent` — Call Another Agent

```gx
result = spawn agent "summarizer" with { text: "hello world" }
log(result)   // the value from the agent's communicate block
```

### `|>` Pipeline

Pipes the result of one `spawn agent` call directly into the next as input. Non-object values are auto-wrapped as `{ value: X }`.

```gx
result = { value: 5 } |> spawn agent "doubler" |> spawn agent "formatter"
log(result)  // "RESULT: 10"
```

### `spawn "event" to "agent"` — Send a Message

Delivers a message synchronously to the target agent's `when message` handler.

```gx
spawn "task" to "worker" with { task: "process data" }
```

### Callable Agent Pattern

Any `helper` that reads `input` in its brain is treated as callable-only (it won't auto-run at startup). The communicate block's last expression is the return value.

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

### Null Coalescing in Agents

Use `??` to provide defaults when input fields are absent:

```gx
name = input.name ?? "stranger"
```

---

## Statements

### Assignment

```gx
x = 42
memory.count = 0
memory.user.name = "Ahmed"
```

### Augmented Assignment

```gx
memory.count += 1
memory.score -= 5
memory.total *= 2
memory.price /= 100
```

### `if` / `else if` / `else`

```gx
if memory.count > 10 {
  log("big")
} else if memory.count > 5 {
  log("medium")
} else {
  log("small")
}
```

### `for` / `for each`

The `each` keyword is optional — both forms work:

```gx
for each item in memory.items {
  log(item)
}

// equivalent — `each` is optional
for item in memory.items {
  log(item)
}

for x in [1, 2, 3] {
  memory.total += x
}
```

### `try` / `catch`

```gx
try {
  result = js.axios.get("https://api.example.com")
} catch e {
  log("Request failed: " + e)
}
```

### `log` / `say` / `output`

All three print to stdout. `say` and `log` are equivalent; `output` is the same.

```gx
log("message")
say "message"
output("message")
say "Hello, {memory.name}!"      // string interpolation
```

### `emit` — Emit an Event

```gx
emit "event_name" { key: value, other: memory.x }
```

### `broadcast`

```gx
broadcast "event_name"
```

### `re-run`

Restarts the brain cycle from the `plan` block. Maximum 100 iterations before error.

```gx
when memory.retries < 3 {
  memory.retries += 1
  re-run
}
```

### `escalate to human`

Stops the brain cycle and emits an escalation signal.

```gx
if result.confidence < 0.6 {
  escalate to human
}
```

### `wait`

```gx
wait 1000    // wait 1000ms
```

---

## Expressions

### Literals

```gx
42          // integer
3.14        // float
"hello"     // string
true        // bool
false       // bool
null        // null
[1, 2, 3]  // array
{ a: 1 }   // object
```

### String Interpolation

```gx
"Hello, {name}!"
"Count is {memory.count}, timestamp {get_timestamp()}"
```

### Field Access

```gx
memory.name
result.confidence
config.database.host
```

### Index Access

```gx
items[0]
memory.matrix[1][2]
```

### Method Calls

```gx
"hello".length()
"hello world".split(" ")
" hello ".trim()
"hello".to_upper()
"HELLO".to_lower()
"hello world".contains("world")
"hello".replace("l", "r")
[1, 2, 3].length()
```

### Function Calls

```gx
log("message")
get_timestamp()
to_string(42)
len("hello")
```

---

## Built-in Functions

| Function | Description | Example |
|----------|-------------|---------|
| `log(v)` | Print value to stdout | `log("hello")` |
| `say x` | Print value (statement) | `say "hello"` |
| `get_timestamp()` | Unix timestamp in milliseconds | `t = get_timestamp()` |
| `to_string(v)` | Convert to string | `s = to_string(42)` |
| `len(v)` | Length of string or array | `n = len("hello")` |

### String Methods

| Method | Description |
|--------|-------------|
| `.length()` / `.len()` | Character count |
| `.to_upper()` | Uppercase |
| `.to_lower()` | Lowercase |
| `.trim()` | Strip whitespace |
| `.split(sep)` | Split into array |
| `.contains(sub)` | Substring check |
| `.starts_with(s)` | Prefix check |
| `.ends_with(s)` | Suffix check |
| `.replace(from, to)` | Replace substring |

### Array Methods

| Method | Description |
|--------|-------------|
| `.length()` | Element count |
| `.push(v)` | Append (returns new array) |
| `.pop()` | Remove last |
| `.first()` | First element |
| `.last()` | Last element |
| `.contains(v)` | Membership check |
| `.join(sep)` | Join to string |
| `.reverse()` | Reversed copy |

### Object Methods

| Method | Description |
|--------|-------------|
| `.has(key)` / `.has_key(key)` | Key existence check |
| `.keys()` | Array of keys |
| `.values()` | Array of values |

---

## AI Primitives

### `ask` — Call an AI Model

```gx
result = ask openai {
  prompt: "Summarize this: {memory.text}",
  max_tokens: 200,
  temperature: 0.7
}

result = ask anthropic {
  prompt: "What is 2 + 2?",
  system: "You are a math tutor."
}

result = ask ollama {
  prompt: "Explain recursion."
}

result = ask ollama:mistral {
  prompt: "Translate to French: {memory.text}"
}
```

**Response object:**

| Field | Type | Description |
|-------|------|-------------|
| `result.text` | String | The model's response |
| `result.confidence` | Number | 0.0–1.0, adjusted for hedging |
| `result.tokens_used` | Number | Total tokens consumed |
| `result.model` | String | Model name used |
| `result.provider` | String | `openai`, `anthropic`, or `ollama` |
| `result.ok` | Bool | `true` if request succeeded |

**Provider model defaults:**
- `openai` → `gpt-4o-mini`
- `anthropic` → `claude-haiku-4-5-20251001`
- `ollama` → `llama3`

**Environment variables required:**
```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# ollama: no key needed, run: ollama serve
```

### `embed` — Text Embeddings

```gx
vector = embed "text to convert to a vector"
```

Returns `Value::Array` of floats. Uses OpenAI `text-embedding-3-small`. Requires `OPENAI_API_KEY`.

### `infer classifier` — Classification

```gx
label = infer classifier {
  input: memory.user_message,
  classes: ["support", "sales", "spam", "other"]
}
```

Returns the matched class as a string. Uses the model to pick the best matching class.

---

## Package Interop

### Import

```gx
use js.axios
use py.pandas
use js.path
use py.os
```

### Call

After declaring `use js.path`, any call matching `js.path.*` is routed to Node.js:

```gx
result = js.path.join("/home", "user", "docs")
platform = js.os.platform()
```

After `use py.os`:

```gx
cwd = py.os.getcwd()
env_val = py.os.environ.get("HOME")
```

**How it works:**
- JS calls: spawns `node -e` subprocess with the call encoded as JSON, reads JSON result
- Python calls: persistent child process with embedded shim, communicates via JSON over stdin/stdout (avoids 200ms startup per call)

All results are automatically converted to GX native types (string, number, bool, array, object, null).

---

## Provider Aliases

These provider name aliases are accepted in `ask`:

| Alias | Resolves to |
|-------|-------------|
| `gpt` | `openai` |
| `claude` | `anthropic` |
| `local` | `ollama` |

---

**© 2025 DEVJSX LIMITED** — Ahmed Elgarhy
