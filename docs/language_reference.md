# GX Language — API Reference

Complete reference for GX syntax, built-in functions, and AI primitives.

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
// Runs once on startup, before the brain cycle
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
```

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

```gx
for each item in memory.items {
  log(item)
}

for each x in [1, 2, 3] {
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
