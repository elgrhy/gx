# GX Language — Reference

Complete reference for GX v0.5.0 syntax, built-in functions, and AI primitives.

---

## Syntax Levels

GX has three progressive syntax levels that all compile to the same runtime.

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
- Memory is shared: changes inside a behavior propagate back to the caller

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

The original syntax — fully supported alongside progressive syntax.

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

// Closures / lambdas
double = fn(x) { x * 2 }
result = double(21)   // 42

// Functions as values
transform = fn(x) { x * 10 }
apply = fn(f, v) { f(v) }
apply(transform, 5)   // 50
```

Functions are defined at the top level (outside agents). They can be called from anywhere — `when` blocks, `brain` phases, other functions.

### `import` — File Import

```gx
import "agents/utils.gx"
import "lib/math.gx"    as math
import "utils/fmt.gx"   as fmt
```

Loads functions and agents from another `.gx` file. All functions defined in the imported file become available in the current file. With `as alias`, call as `math.add(...)`.

### `use` — Package Import

```gx
use js.axios        // npm package
use py.requests     // Python package
use ts.analytics    // TypeScript (tsx or ts-node)
use go "./service"  // Go binary
use binary "./app"  // any compiled binary

// Optional stdlib namespace (no-op — functions already available globally)
use std.crypto
use std.fs
use std.net
use std.collections
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

Values here are accessible as `memory.key` everywhere in the helper.

### `brain` — Cognitive Cycle

```gx
brain {
  plan {
    plan = { action: "process" }
  }
  execute {
    if plan.action == "process" {
      result = 42
    }
  }
  remember {
    memory.last_result = result
  }
  communicate {
    emit "done" { value: memory.last_result }
  }
}
```

### `when` — Trigger Blocks

```gx
when started {
  say "Agent is ready"
}

when memory.count > 10 {
  log("Threshold reached")
}

when memory.status changes {
  re-run
}

when message "task" {
  log("Received task: {message.task}")
}

when cron "0 9 * * 1-5" {
  log("Good morning, weekday")
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

```gx
for each item in memory.items {
  log(item)
}

// `each` is optional — both forms work
for item in memory.items {
  log(item)
}

for x in [1, 2, 3] {
  memory.total += x
}

for n in range(1, 11) {
  log(n)
}
```

### `while` / `break` / `continue`

```gx
while running {
  line = readline()
  if line == null { break }
  if line.starts_with("#") { continue }
  process(line)
}
```

### `try` / `catch`

```gx
try {
  result = js.axios.get("https://api.example.com")
} catch NetworkError e {
  log("Network failed: " + e)
} catch e {
  log("Other error: " + e)
}
```

### `log` / `say` / `output` / `write`

```gx
log("message")
say "message"
output("message")      // same as say
write("no newline")    // print without trailing newline (v0.5.0+)
say "Hello, {memory.name}!"
```

### `emit` / `broadcast`

```gx
emit "event_name" { key: value }
broadcast "event_name"
```

### `re-run`

Restarts the brain cycle from the `plan` block. Maximum 100 iterations.

### `escalate to human`

Stops the brain cycle and emits an escalation signal.

### `return`

Return a value from a function:

```gx
function double(x) {
  return x * 2
}
```

---

## Expressions

### Literals

```gx
42          // integer
3.14        // float
"hello"     // string
true / false
null
[1, 2, 3]   // array
{ a: 1 }    // object
```

### Operators

```gx
// Arithmetic
+  -  *  /  %

// Comparison
==  !=  <  >  <=  >=

// Logic
&&  ||  !

// Null coalescing
value ?? default_value

// Assignment
=  +=  -=  *=  /=

// Range slice (on strings and arrays)
str[0..5]
arr[1..4]
```

### String Interpolation

```gx
"Hello, {name}!"
"Count is {memory.count}, result {1 + 2 * 3}"
"Literal brace: {{name}}"
```

### Field and Index Access

```gx
memory.name
result.confidence
config.database.host
items[0]
matrix[1][2]
url_obj.scheme
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

---

## Built-in Functions

### Output

| Function | Description |
|---|---|
| `log(v)` | Print value to stdout with newline |
| `say x` | Print value (statement form) |
| `write(v)` | Print without trailing newline |
| `print_inline(v)` | Alias for `write` |

### AI

| Function | Description |
|---|---|
| `ask openai { ... }` | Call OpenAI (gpt-4o-mini default) |
| `ask anthropic { ... }` | Call Anthropic (claude-sonnet-4-6 default) |
| `ask ollama { ... }` | Call local Ollama (llama3 default) |
| `embed "text"` | Text embeddings via OpenAI |
| `infer classifier { input, classes }` | Classify text into named categories |

AI response fields: `.text`, `.confidence`, `.tokens_used`, `.model`, `.provider`, `.ok`

### Token Tracking (v0.5.0)

| Function | Description |
|---|---|
| `token_count(str)` | Heuristic token estimate (~4 chars/token) |
| `tokens_used()` | Cumulative tokens from all `ask` calls this run |
| `total_tokens()` | Alias for `tokens_used()` |

### Crypto (v0.5.0)

| Function | Description |
|---|---|
| `sha256(str)` | SHA-256 hash as 64-char lowercase hex |
| `uuid()` | Generate UUID v4 string |
| `uuid_v4()` | Alias for `uuid()` |

```gx
use std.crypto   // optional — already available globally

h  = sha256("hello world")
id = uuid()   // "f47ac10b-58cc-4372-a567-0e02b2c3d479"
```

### Path / Filesystem (v0.5.0)

| Function | Description |
|---|---|
| `dirname(path)` | Parent directory of path |
| `basename(path)` | Filename component of path |
| `path_join(a, b, ...)` | Join path segments (cross-platform) |
| `glob(pattern)` | Return array of paths matching shell glob pattern |

```gx
use std.fs   // optional

dirname("/home/user/file.txt")    // "/home/user"
basename("/home/user/file.txt")   // "file.txt"
path_join("a", "b", "c.txt")     // "a/b/c.txt"
glob("reports/*.csv")             // ["reports/jan.csv", ...]
```

`glob` is sandboxed — only returns paths within the script's directory.

### URL Parsing (v0.5.0)

| Function | Description |
|---|---|
| `url_parse(url)` | Parse URL into object with fields |

```gx
use std.net   // optional

u = url_parse("https://api.example.com:8080/v1?q=gx#top")
u.scheme    // "https"
u.host      // "api.example.com"
u.port      // "8080"
u.path      // "/v1"
u.query     // "q=gx"
u.fragment  // "top"
```

### String Utilities

| Function | Description |
|---|---|
| `truncate(str, max)` | Clip to max chars, append `…` |
| `truncate(str, max, ellipsis)` | Clip to max chars, append custom ellipsis |
| `len(v)` | Length of string or array |
| `to_string(v)` | Convert value to string |
| `to_number(v)` | Convert value to number |
| `type_of(v)` | Return type name as string |
| `is_null(v)` | True if value is null |

```gx
truncate("hello world", 8)          // "hello w…"
truncate("hello world", 8, "...")   // "hello..."
truncate("hi", 20)                  // "hi" (no change)
```

### Data Helpers (v0.5.0)

| Function | Description |
|---|---|
| `group_by(arr, key)` | Group array of objects by field value |

```gx
use std.collections   // optional

rows = [
  { name: "Alice", dept: "eng" },
  { name: "Bob",   dept: "eng" },
  { name: "Carol", dept: "hr" }
]
by_dept = group_by(rows, "dept")
// { "eng": [{...},{...}], "hr": [{...}] }
```

### HTTP

| Function | Description |
|---|---|
| `http_get(url)` | GET request |
| `http_post(url, body)` | POST request |
| `http_put(url, body)` | PUT request |
| `http_delete(url)` | DELETE request |
| `http_stream(url, body)` | Streaming HTTP |
| `http_upload(url, file)` | Upload file |

Response fields: `.body`, `.status`, `.headers`, `.ok`, `.data` (auto-parsed JSON)

### File I/O

| Function | Description |
|---|---|
| `read_file(path)` | Read file as string |
| `write_file(path, content)` | Write file |
| `append_file(path, content)` | Append to file |
| `delete_file(path)` | Delete file |
| `file_exists(path)` | Returns bool |
| `list_dir(path)` | Returns array of filenames |
| `make_dir(path)` | Create directory |

All file operations are sandboxed to the script's directory. Use `--no-sandbox` to allow wider access.

### Environment

| Function | Description |
|---|---|
| `load_env(path)` | Load `.env` file (sandboxed) |
| `get_env(key)` | Get env var or null |
| `get_env(key, default)` | Get env var with fallback |
| `set_env(key, value)` | Set env var for current process |
| `env(key)` | Alias for `get_env` |

### JSON / Serialization

| Function | Description |
|---|---|
| `json_stringify(v)` | Serialize to JSON (integers stay integers) |
| `json_parse(str)` | Parse JSON string |
| `csv_parse(str)` | Parse CSV with auto-typed values |
| `csv_stringify(arr)` | Serialize array of objects to CSV |
| `yaml_parse(str)` | Parse YAML string |
| `yaml_stringify(v)` | Serialize to YAML |
| `toml_parse(str)` | Parse TOML string |
| `toml_stringify(v)` | Serialize to TOML |

### Math

| Function | Description |
|---|---|
| `abs(n)` | Absolute value |
| `floor(n)` / `ceil(n)` | Round down / up |
| `round(n)` | Round to nearest integer |
| `sqrt(n)` | Square root |
| `pow(base, exp)` | Exponentiation |
| `min(a, b, ...)` | Minimum of arguments or array |
| `max(a, b, ...)` | Maximum of arguments or array |
| `clamp(v, lo, hi)` | Clamp value to range |
| `random()` | Random float 0.0–1.0 |
| `pi` | 3.14159... |
| `e` | 2.71828... |

### Regex

| Function | Description |
|---|---|
| `regex_test(str, pattern)` | Returns bool |
| `regex_find(str, pattern)` | First match or null |
| `regex_find_all(str, pattern)` | All matches as array |
| `regex_replace(str, pattern, replacement)` | Replace first match |
| `regex_split(str, pattern)` | Split by pattern |
| `regex_captures(str, pattern)` | Array of capture groups |
| `regex_named_captures(str, pattern)` | Object of named captures |

### Date / Time

| Function | Description |
|---|---|
| `date_now()` | Current time as ISO 8601 string |
| `date_timestamp()` | Current Unix timestamp (ms) |
| `date_parse(str)` | Parse date string → Unix timestamp |
| `date_format(ts, fmt)` | Format timestamp with strftime pattern |
| `date_diff(a, b, unit)` | Difference in `"days"`, `"hours"`, `"minutes"`, `"seconds"` |
| `date_add(ts, n, unit)` | Add n units to timestamp |
| `date_parts(ts)` | Object: `{ year, month, day, hour, minute, second, weekday }` |
| `date_from_parts(obj)` | Build timestamp from parts object |

### Array Methods

| Method | Description |
|---|---|
| `.push(v)` | Append element (returns new array) |
| `.pop()` | Remove last element |
| `.sort()` | Sort ascending |
| `.reverse()` | Reverse order |
| `.unique()` | Deduplicate |
| `.flatten()` | Flatten one level |
| `.sum()` | Sum of numeric elements |
| `.min()` / `.max()` | Min / max value |
| `.average()` | Mean value |
| `.take(n)` | First n elements |
| `.skip(n)` | Skip first n elements |
| `.join(sep)` | Join to string |
| `.filter_by(key, value)` | Filter objects by field |
| `.map_field(key)` | Extract field from each object |
| `.contains(v)` | Membership check |
| `.first()` / `.last()` | First / last element |

### Object

| Function | Description |
|---|---|
| `keys(obj)` | Array of keys |
| `values(obj)` | Array of values |
| `entries(obj)` | Array of `[key, value]` pairs |
| `merge(obj, ...)` | Merge objects (later wins) |
| `has(obj, key)` | Key existence check |
| `group_by(arr, key)` | Group array of objects by field |

### Vector Store

```gx
store = vector_store_new("name")
vector_store_add(store, "id", embed("text"), "label")
hits  = vector_store_search(store, embed("query"), top_k)

// hits[i].id, hits[i].label, hits[i].score
sim = cosine_similarity([1.0, 0.0], [0.7, 0.7])
```

### Schema Validation

```gx
spec = { name: "string", age: "number", email: { type: "string", required: false } }
r = schema_validate(user_input, spec)
if !r.ok {
  for each err in r.errors { log("Error: " + err) }
}
```

### Database (SQLite)

```gx
rows   = db_query("SELECT * FROM users WHERE active = ?", [true])
count  = db_exec("INSERT INTO events (name) VALUES (?)", ["login"])
```

### Persistent Memory

```gx
persist_memory()   // save memory to ~/.gx/state/<agent>.db
load_memory()      // restore from SQLite
```

### Observability

```gx
trace_log("event.name", { key: value })
// Emits JSONL to stderr: {"ts":...,"agent":"...","event":"...","data":{...}}
```

### Retry

```gx
result = retry(fn() {
  return ask openai { prompt: "classify this" }
}, 5, { delay: 1000, backoff: "exponential" })
// Retries up to 5×: 1s, 2s, 4s, 8s, 16s (capped at 30s)
```

### Await — Concurrent Branches

```gx
await {
  weather: http_get("https://api.weather.com/london"),
  news:    http_get("https://api.news.com/top")
} into data

log(data.weather.body)
log(data.news.body)
```

### Shell

```gx
result = shell("ls -la")   // requires --allow-shell flag
result.stdout
result.stderr
result.exit_code
```

### Base64

```gx
enc = base64_encode("hello world")
dec = base64_decode(enc)
```

### I/O

```gx
line = readline()      // read one line from stdin (or null on EOF)
all  = read_all()      // read all stdin
```

### Type Utilities

```gx
type_of(42)         // "number"
type_of("hi")       // "string"
type_of([1,2])      // "array"
type_of({ a: 1 })   // "object"
type_of(null)       // "null"
is_null(null)       // true
is_tty()            // true if stdin is a terminal
```

---

## AI Primitives

### `ask` — Call an AI Model

```gx
result = ask openai {
  prompt: "Summarize: {memory.text}",
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

// Streaming
result = ask openai {
  prompt: "Write a long story",
  stream: true   // chunks print in real-time; result.text has full assembled text
}

// AI tool use / function calling
result = ask openai {
  prompt: "What is the weather in London?",
  tools: [get_weather],
  model: "gpt-4o"
}
```

**Response object:**

| Field | Type | Description |
|---|---|---|
| `result.text` | String | The model's response |
| `result.confidence` | Number | 0.0–1.0 (lower when model hedges) |
| `result.tokens_used` | Number | Tokens for this call |
| `result.model` | String | Model name used |
| `result.provider` | String | `openai`, `anthropic`, or `ollama` |
| `result.ok` | Bool | `true` if request succeeded |
| `result.tool_calls` | Array | Tool calls requested by the model |

**Provider defaults:**
- `openai` → `gpt-4o-mini`
- `anthropic` → `claude-sonnet-4-6`
- `ollama` → `llama3`

**Provider aliases:**
- `gpt` → `openai`
- `claude` → `anthropic`
- `local` → `ollama`

**Environment variables required:**
```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# ollama: no key needed — run: ollama serve
```

### `embed` — Text Embeddings

```gx
vector = embed "text to convert to a vector"
```

Returns `Array` of floats. Uses OpenAI `text-embedding-3-small`. Requires `OPENAI_API_KEY`.

### `infer classifier` — Classification

```gx
label = infer classifier {
  input: memory.user_message,
  classes: ["support", "sales", "spam", "other"]
}
```

Returns the matched class as a string.

---

## Multi-Agent Orchestration

### `spawn agent` — Call Another Agent

```gx
result = spawn agent "summarizer" with { text: "hello world" }
log(result)
```

### `|>` Pipeline

```gx
result = { value: 5 } |> spawn agent "doubler" |> spawn agent "formatter"
```

Non-object values are auto-wrapped as `{ value: X }`.

### `spawn "event" to "agent"` — Send a Message

```gx
spawn "task" to "worker" with { task: "process data" }
```

---

## Package Interop

```gx
use js.path
result = js.path.join("/home", "user")

use py.os
cwd = py.os.getcwd()

use ts.analytics
report = ts.analytics.generate(data)

use binary "./my_processor"
output = binary.transform(payload)
```

**How it works:**
- JS calls: one-shot `node -e` subprocess per call
- Python calls: persistent child process with JSON IPC (no 200ms startup per call)
- TypeScript: auto-detects `tsx` or `ts-node`
- Go/Binary: compiled binary with JSON stdin/stdout protocol

---

## Security Model

| Flag | Effect |
|---|---|
| *(default)* | File I/O sandboxed to script dir; shell and internal HTTP blocked |
| `--allow-shell` | Enable `shell()` builtin |
| `--allow-internal-http` | Allow HTTP to private/localhost IPs |
| `--no-sandbox` | Disable file-path sandboxing |
| `--no-limit` | Remove while-loop iteration cap |

---

**© 2026 DEVJSX LIMITED** — Ahmed Elgarhy
