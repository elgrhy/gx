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

### Crypto (v0.5.0 / v0.6.0)

| Function | Description |
|---|---|
| `sha256(str)` | SHA-256 hash as 64-char lowercase hex |
| `uuid()` | Generate UUID v4 string |
| `uuid_v4()` | Alias for `uuid()` |
| `hmac_sha256(key, message)` | HMAC-SHA256 as 64-char lowercase hex |
| `hmac_sha512(key, message)` | HMAC-SHA512 as 128-char lowercase hex |
| `secure_compare(a, b)` | Constant-time string equality — use instead of `==` for secrets/signatures |
| `secure_random(length)` | `length` cryptographically-random bytes, hex-encoded (0–1,048,576) |
| `ed25519_generate_keypair()` | `{public_key, private_key}` — both 32-byte hex |
| `ed25519_sign(private_key, message)` | Ed25519 signature as 128-char hex |
| `ed25519_verify(public_key, message, signature)` | `true`/`false` — never throws on malformed *or oversized* input |
| `jwt_sign(payload, secret)` | HS256 JWT string. `payload` must be an object; `secret` must be >= 32 bytes |
| `jwt_verify(token, secret)` | `{valid, payload, header, reason}` — never throws on a bad, oversized, or short-secret token |

All cryptographic math (hashing, MAC, elliptic-curve signing, randomness) is
provided by audited Rust crates (`hmac`, `sha2`, `ed25519-dalek`, `subtle`,
`getrandom`, `jsonwebtoken`) — GX only validates arguments and shapes the
result. Keys, digests, and signatures are always plain lowercase hex strings.

#### Input-size limits

Every function that can be handed untrusted input (an HMAC message, an
Ed25519 signature, a JWT bearer token) enforces a maximum size **before**
doing any decoding or hashing, so a caller can't force unbounded CPU/memory
work just by sending an oversized string — e.g. piping an entire, unbounded
HTTP request body into `hmac_sha256` or `jwt_verify`.

| Function / argument | Limit |
|---|---|
| `hmac_sha256`/`hmac_sha512` key | 4 KiB |
| `hmac_sha256`/`hmac_sha512` message | 10 MiB |
| `ed25519_sign`/`ed25519_verify` private/public key (hex) | 256 chars |
| `ed25519_sign`/`ed25519_verify` signature (hex) | 512 chars |
| `ed25519_sign`/`ed25519_verify` message | 10 MiB |
| `jwt_sign`/`jwt_verify` secret | 32 bytes minimum, 1 KiB maximum |
| `jwt_sign` payload (serialized JSON) | 8 KiB |
| `jwt_verify` token | 8 KiB |
| `secure_random` length | 0–1,048,576 bytes |

**Generation functions** (`hmac_sha256`, `hmac_sha512`, `ed25519_sign`,
`jwt_sign`) raise a runtime error on oversized or invalid input — these are
developer-invoked, so a loud failure is correct.

**Verification functions** never throw, even when the input is oversized:
- `ed25519_verify` returns `false`.
- `jwt_verify` returns `{ valid: false, payload: null, header: null, reason: "input_too_large" }`
  for an oversized token or secret, or `{ valid: false, reason: "secret must be at least 32 bytes..." }`
  for a secret under the 32-byte minimum.

This means neither function needs a `try`/`catch` wrapped around it just to
handle a hostile or malformed request — the failure is always a value, never
an exception.

```gx
use std.crypto   // optional — already available globally

h  = sha256("hello world")
id = uuid()   // "f47ac10b-58cc-4372-a567-0e02b2c3d479"
```

#### HMAC verification (generic webhook)

Most webhook providers (Stripe, GitHub, generic HMAC-signed integrations)
send a hex-encoded HMAC-SHA256 of the raw request body. Always compare with
`secure_compare`, never `==` — a plain string comparison leaks timing
information about how many leading bytes matched.

```gx
fn verify_webhook(body, signature_header, secret) {
  expected = hmac_sha256(secret, body)
  return secure_compare(expected, signature_header)
}

if verify_webhook(request.body, request.headers["X-Signature"], env("WEBHOOK_SECRET")) {
  say "webhook verified"
} else {
  say "rejected: bad signature"
}
```

#### Slack webhook verification

Slack signs each request with `X-Slack-Signature: v0=<hex>` over the string
`v0:{timestamp}:{raw_body}`, using your Slack signing secret as the HMAC key.

```gx
fn verify_slack(timestamp, raw_body, slack_signature, signing_secret) {
  base = "v0:{timestamp}:{raw_body}"
  expected = "v0=" + hmac_sha256(signing_secret, base)
  return secure_compare(expected, slack_signature)
}
```

#### Discord webhook verification

Discord signs interactions with Ed25519 rather than HMAC: it sends
`X-Signature-Ed25519` (hex signature) and `X-Signature-Timestamp`, signed over
`timestamp + raw_body`, verified against your application's public key.

```gx
fn verify_discord(timestamp, raw_body, signature_header, public_key) {
  message = timestamp + raw_body
  return ed25519_verify(public_key, message, signature_header)
}
```

#### Ed25519 key generation, signing, and verification

```gx
kp = ed25519_generate_keypair()   // {public_key, private_key} — store private_key securely

signature = ed25519_sign(kp.private_key, "attack at dawn")
ed25519_verify(kp.public_key, "attack at dawn", signature)   // true
ed25519_verify(kp.public_key, "attack at dusk", signature)   // false — tampered message
```

`private_key`/`public_key` are the raw 32-byte Ed25519 seed/point, hex-encoded
— not a libsodium-style 64-byte "extended" secret key.

> **Warning — public and private keys look identical.** Both `public_key` and
> `private_key` are plain 32-byte hex strings of the same length, with no
> prefix or tag distinguishing one from the other. GX cannot detect if you
> pass the wrong one to the wrong function — `ed25519_sign` will happily
> "sign" with a key that was meant to be public, producing a signature that
> silently won't verify against the key you actually intended. There is no
> loud error for this mistake, only a verification failure somewhere
> downstream. Concretely:
> - Never send `private_key` anywhere over the network or log it.
> - Double-check which field (`kp.public_key` vs `kp.private_key`) you're
>   passing at each call site — the types don't save you here.
> - A future GX version may introduce typed or prefixed key formats
>   (e.g. `pub_<hex>` / `priv_<hex>`) specifically to make this mistake loud
>   instead of silent; the current hex-only format is a deliberate v1
>   simplification, not a guarantee that it will stay this way.

#### JWT creation and verification

Only HS256 is supported in v1 — `jwt_verify` rejects every other algorithm,
including a forged `"alg":"none"` token, before ever checking the signature.

```gx
secret = env("JWT_SECRET")
now = date_timestamp()

token = jwt_sign({ sub: "user-123", role: "admin", exp: now + 3600 }, secret)

result = jwt_verify(token, secret)
if result.valid {
  say "authenticated as {result.payload.sub}"
} else {
  say "rejected: {result.reason}"   // e.g. "token has expired", "invalid signature"
}
```

`jwt_verify` always returns a result object — it never throws for an invalid,
expired, or malformed token, so no `try`/`catch` is required around it.
`exp` is required and validated; `nbf` and `iat` are validated when present.

#### secure_random

```gx
api_key = secure_random(32)          // 64 hex chars — e.g. an API key or session token
csrf_token = secure_random(16)
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

`shell()` runs a string through `sh -c` (or the platform shell). It's the
right tool exactly when you need real shell semantics: pipes (`|`),
redirects (`>`), glob expansion, or `&&`/`||` chaining. It is **not**
recommended for running a single external program with arguments — use the
[process runtime](#process-runtime-recommended) below for that instead,
which never invokes a shell and so has no quoting/injection surface at all.
`shell()` is not being removed and remains fully supported.

### Process Runtime (recommended)

| Function | Description |
|---|---|
| `process_run(spec)` | Run a program to completion. Blocks until exit (or `timeout`). Returns a result object, never throws for the program's own failure. |
| `process_spawn(spec)` | Start a program in the background. Returns an opaque handle string immediately. |
| `process_wait(handle, [timeout])` | Block until the handle's process exits (or `timeout` elapses, which also kills it). Collects final output and releases the handle. |
| `process_kill(handle)` | Terminate it now. Returns `true` if a running process was signaled, `false` if the handle is unknown or already finished. |
| `process_exists(handle)` | `true` if the handle is known and still running. |
| `process_status(handle)` | Non-blocking snapshot (full field list below), or `null` for an unknown handle. |
| `process_read(handle)` | Pull new stdout/stderr produced since the last read: `{stdout, stderr, done}`, or `null` for an unknown handle. |

Requires `--allow-process` (a separate flag from `--allow-shell` — an
application can enable one without the other). `spec` is always an object:

```gx
spec = {
  command: "git",           // required
  args: ["status", "--short"], // optional, default []
  cwd: "/path/to/repo",      // optional; sandboxed like file I/O when active
  env: { GIT_PAGER: "" },    // optional; merged into the inherited environment
  stdin: "input text\n",     // optional
  timeout: 10                // optional, seconds — kills the process if exceeded
}
```

**No shell is ever invoked.** `args` is a real argument array passed directly
to the OS (`execvp`-style on Unix, `CreateProcess` on Windows) — there is no
quoting dialect to get right or wrong, because nothing re-parses a command
string. This is what makes it immune to shell injection and consistent across
Linux, macOS, and Windows: the same `{command, args}` shape behaves the same
way on every platform, since none of them involve a shell at all.

```gx
result = process_run({ command: "git", args: ["log", "--oneline", "-5"] })
if result.ok {
  say result.stdout
} else {
  say "git failed: {result.error_kind}"   // e.g. "not_found", "timeout", or null for a plain non-zero exit
}
```

`process_run`/`process_wait` never throw for the *program's own* failure
(not found, non-zero exit, timeout) — same convention as `http_get` never
throwing for a failed request. They only throw for programmer error: a
malformed spec, `--allow-process` not granted, or a `cwd` outside the
sandbox.

#### Result object (`process_run` / `process_wait`)

| Field | Meaning |
|---|---|
| `ok` | `true` only for a normal, non-timed-out, non-killed exit with code `0`. |
| `stdout` / `stderr` | Captured output, as strings (capped — see [Output truncation](#output-truncation) below). |
| `exit_code` | The real exit code, or `null` if the process never exited normally (killed, timed out, or still unconfirmed). |
| `timed_out` / `killed` | Whether the spec's `timeout` fired, or `process_kill`/this call's own `timeout` terminated it. |
| `error_kind` | `null` for a plain non-zero exit; otherwise one of `"not_found"`, `"permission_denied"`, `"spawn_failed"`, `"timeout"`, `"killed"`, `"unresponsive"`, `"input_too_large"`-style spec errors are thrown instead, not returned here. |
| `truncated` / `stdout_truncated` / `stderr_truncated` | Whether output was cut off — see below. |
| `stdout_bytes` / `stderr_bytes` | **Total** bytes the process actually produced on each stream, not just how much was retained. |

`error_kind: "unresponsive"` is a distinct, rare outcome from `"timeout"`/`"killed"`:
it means the runtime tried to terminate the process and, after a bounded
grace period, still couldn't confirm it actually exited (see
[Uninterruptible-sleep processes](#uninterruptible-sleep-d-state-processes)
below). Unlike every other outcome, the process handle is **not** released
in this case — call `process_wait`/`process_status` again later to check on
it.

#### Output truncation

Each stream is capped at 32 MiB of *retained* output — a runaway or chatty
child can't grow GX's memory without bound. This is never silent: `truncated`
(and the per-stream `stdout_truncated`/`stderr_truncated`) tells you when it
happened, and `stdout_bytes`/`stderr_bytes` report how much the process
*actually* produced, even though only 32 MiB of it is in `stdout`/`stderr`.

```gx
result = process_run({ command: "some-noisy-tool" })
if result.truncated {
  say "warning: only {len(result.stdout)} of {result.stdout_bytes} stdout bytes were kept"
}
```

If you need to process output larger than the cap, use `process_spawn` +
`process_read` to consume it incrementally instead of `process_run`/
`process_wait`'s all-at-once capture.

#### Streaming and long-running processes

```gx
h = process_spawn({ command: "ping", args: ["-c", "20", "example.com"] })
while process_exists(h) {
  chunk = process_read(h)
  if len(chunk.stdout) > 0 { say chunk.stdout }
  sleep(0.5)
}
result = process_wait(h)
```

#### Cancellation

```gx
h = process_spawn({ command: "ffmpeg", args: [...] })
// ...user cancels the job...
process_kill(h)
result = process_wait(h)   // result.killed == true
```

#### Observability (`process_status`)

A non-blocking snapshot for logging, dashboards, and diagnostics — never
includes `env` or `stdin` content (only how many bytes of stdin were sent),
so it's safe to log without worrying about leaking secrets passed that way.

| Field | Meaning |
|---|---|
| `pid`, `command`, `args`, `cwd` | What was launched and where (`cwd` is `null` if none was given). |
| `running` | `true` until the process is confirmed finished. |
| `exit_code` | `null` while running, killed, or timed out. |
| `exit_reason` | One-glance summary: `"running"`, `"exited"`, `"exited_error"`, `"timeout"`, `"killed"`, or `"unknown"`. |
| `started_at`, `finished_at` | Unix milliseconds (`finished_at` is `null` while running). |
| `duration_ms` | Elapsed time so far (running) or total (finished). |
| `stdin_bytes` | Bytes of `stdin` handed to the child (`0` if none). |
| `stdout_bytes`, `stderr_bytes`, `truncated`, `stdout_truncated`, `stderr_truncated` | Same meaning as on the result object — see [Output truncation](#output-truncation). |
| `timed_out`, `killed` | Same meaning as on the result object. |

```gx
h = process_spawn({ command: "long-running-job" })
status = process_status(h)
say "{status.command} (pid {status.pid}): {status.exit_reason}, running for {status.duration_ms}ms"
```

#### Uninterruptible-sleep (D-state) processes

On Linux, a process blocked on hung kernel-level I/O (e.g. an unresponsive
NFS mount) can enter *uninterruptible sleep* — it will not respond to
`SIGKILL`, or any other signal, until it leaves that state, which the kernel
alone controls. No signal, timeout, or userspace trick can force it to
terminate sooner. This is a real operating-system limitation, not a GX gap —
GX bounds its own side of the wait instead of pretending the problem doesn't
exist: after a kill is attempted, the runtime waits up to ~2 extra seconds to
*confirm* the process actually exited. If it can't confirm this, `process_wait`
(and `process_run`, which calls the same logic internally) return
`{ok: false, error_kind: "unresponsive", ...}` rather than hanging forever —
and, uniquely among outcomes, the handle stays registered so you can check on
it again later:

```gx
result = process_wait(h)
if result.error_kind == "unresponsive" {
  log_warn("process may still be running (D-state?), checking again later")
  // handle h is still valid — process_status(h)/process_wait(h) again later
}
```

#### Capability and sandbox integration

- `--allow-process` gates `process_run`/`process_spawn`, independently of
  `--allow-shell` — an application can allow structured process execution
  while fully disabling shell string execution.
- `gx.json`'s `dependencies.process` array restricts *which* executables may
  be launched, the same allowlist mechanism already used for `dependencies.js`
  and `dependencies.py`:
  ```json
  { "dependencies": { "process": ["git", "docker", "ffmpeg"] } }
  ```
  If declared, only listed executables run; if the manifest doesn't declare
  `dependencies.process` at all, any executable is allowed (same behavior as
  the existing js/py allowlists).
- `cwd`, if given, resolves through the same sandbox that governs file I/O —
  a `cwd` outside the sandbox directory is rejected. The executable itself is
  never sandbox-restricted (sandboxing governs data, not which system
  binaries exist).
- **The allowlist bounds which executable *starts* — not what that
  executable can be made to do.** Allowlisting `git` doesn't prevent git's
  own escape hatches (e.g. `-c core.pager=...`, hooks, aliases) from running
  something else. This is a structural limitation of any executable
  allowlist, not specific to GX — don't treat `dependencies.process` as a
  full sandbox around what an allowed program itself can do with its own
  arguments or configuration.

#### Input size limits

Every field that can carry attacker-reachable data is bounded, checked
*before* it's copied out of the argument value (so an oversized value costs
an O(1) length check to reject, not a full allocation):

| Field | Limit |
|---|---|
| `command` | 4 KiB |
| `cwd` | 4 KiB |
| each `args` element | 1 MiB |
| each `env` key | 256 bytes |
| each `env` value | 1 MiB |
| `stdin` | 10 MiB |

Arguments are for parameters, not payloads — if you need to hand a process a
large blob of data, use `stdin` (10 MiB budget) rather than a giant `args`
element.

#### Resource management

Every process the runtime starts is owned for its full lifetime: a
background thread reaps it the instant it exits (preventing zombie processes
on Unix regardless of whether the script ever calls `process_wait`), and any
process still running when the GX program exits is killed automatically —
nothing is ever orphaned. `process_wait` releases its handle once it
returns; a handle that's spawned and never waited on is still cleaned up at
program exit.

#### Cross-platform behavior

The `{command, args}` shape behaves identically on Linux, macOS, and
Windows because no shell is ever involved — argument passing is handled
entirely by the OS process-creation API (`execvp` on Unix, `CreateProcess`
on Windows), which Rust's standard library implements correctly for each
platform. A few differences are inherent to the platforms themselves, not
something GX layers behavior on top of:

- **Killing is always forceful.** `process_kill` sends `SIGKILL` on Unix or
  calls `TerminateProcess` on Windows — there is no portable "ask nicely"
  signal (like `SIGTERM`) exposed, since Windows has no direct equivalent.
  If you need graceful shutdown, send the process its own IPC/protocol
  signal (e.g. write a message to its `stdin`) before falling back to
  `process_kill`.
- **A killed process's `exit_code` differs by platform.** On Unix, a
  signal-terminated process always reports `exit_code: null`. On Windows,
  `TerminateProcess` can leave a non-null exit code. Always check the
  `killed`/`timed_out` flags to detect termination — never infer it from
  `exit_code` alone; the result object's `ok` field already accounts for
  this correctly.
- **Executable resolution follows each platform's own convention** — PATH
  search with automatic `.exe`/`.cmd`/`.bat` suffix resolution on Windows
  (via `PATHEXT`), exact-name PATH search on Unix. `command: "git"` finds
  `git.exe` on Windows and `git` on Unix without any GX-level translation.

#### Migrating from shell()

Most `shell()` calls that build a command string purely to add arguments —
not to use real shell features — map directly onto `process_run` and
typically get *simpler*, not more complex:

```gx
// Before: string-built, vulnerable if `container`/`command` contain
// shell metacharacters (quotes, `;`, `$()`, backticks, ...).
shell("docker exec " + container + " " + command)

// After: container/command are discrete argv entries — there is nothing
// to escape and nothing to inject into.
process_run({ command: "docker", args: ["exec", container, command] })
```

```gx
// Before: shell '...' quoting only protects against spaces, not against
// an embedded single quote in `text`.
shell("say -o " + out_path + " '" + text + "'")

// After: no quoting needed at all.
process_run({ command: "say", args: ["-o", out_path, text] })
```

```gx
// Before: shell "cd X && Y" chaining used purely to set a working directory.
shell("cd " + work_dir + " && docker-compose -f " + compose_file + " up -d")

// After: cwd is a first-class field — no shell chaining needed.
process_run({ command: "docker-compose", args: ["-f", compose_file, "up", "-d"], cwd: work_dir })
```

Keep `shell()` for cases that genuinely need shell syntax — pipes,
redirects, glob expansion. Even a `||` fallback chain often reads more
clearly (and more in line with GX's "every decision explicit" philosophy) as
explicit GX control flow over a few `process_run` calls than as one shell
one-liner:

```gx
r = process_run({ command: "say", args: ["-o", out_path, text] })
if !r.ok {
  r = process_run({ command: "espeak", args: ["-w", out_path, text] })
}
```

#### Production best practices

- **Default to `process_run`/`process_spawn`, not `shell()`.** Reach for
  `shell()` only when you specifically need pipes, redirects, glob
  expansion, or multi-command chaining — not as a general-purpose "run a
  program" tool.
- **Always set a `timeout`** on anything that isn't a known-fast, trusted
  command. Without one, `process_run`/`process_wait` block until the process
  exits naturally, with no artificial bound — appropriate for a controlled
  local tool, risky for anything network-facing or user-triggered (a hung
  `ffmpeg` job, a webhook-triggered conversion tool that never returns).
- **Declare `gx.json`'s `dependencies.process`** in any application that
  handles untrusted input (LLM tool calls, webhook payloads, user-submitted
  jobs) — an explicit allowlist of executables is a real, enforced boundary;
  relying on `--allow-process` alone means *any* executable on PATH can run.
- **Check `result.truncated` before trusting `result.stdout`/`stderr`** for
  any command whose output size you don't control (logs, `find`, `grep -r`,
  LLM-generated shell equivalents). Use `process_spawn` + `process_read` to
  stream incrementally instead of buffering it all if the output could
  plausibly exceed 32 MiB.
- **Treat `stdin` as the payload channel, `args` as parameters.** Passing a
  large blob as an argument both hits the 1 MiB per-argument cap sooner and
  is unusual for the programs you're likely to be calling (git, docker,
  ffmpeg, ssh) — real payload data (file contents, request bodies) belongs
  on `stdin`.
- **Never build `args` by splitting a string on spaces.** If you're tempted
  to do that, you've reintroduced the exact shell-quoting ambiguity
  (`"a b"` vs. `"a"`, `"b"`) that `process_run` exists to eliminate — keep
  each logical argument as its own array element from the source that
  produced it.

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
| *(default)* | File I/O sandboxed to script dir; shell, process, and internal HTTP blocked |
| `--allow-shell` | Enable `shell()` builtin |
| `--allow-process` | Enable `process_run`/`process_spawn` (independent of `--allow-shell`) |
| `--allow-internal-http` | Allow HTTP to private/localhost IPs |
| `--no-sandbox` | Disable file-path sandboxing |
| `--no-limit` | Remove while-loop iteration cap |

`gx.json`'s `dependencies.process: [...]` further restricts *which*
executables `process_run`/`process_spawn` may launch, the same allowlist
mechanism as `dependencies.js`/`dependencies.py` for bridge modules.

---

**© 2026 DEVJSX LIMITED** — Ahmed Elgarhy
