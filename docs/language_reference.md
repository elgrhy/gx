# GX Language — Reference

Complete reference for GX v0.6.1 syntax, built-in functions, and AI primitives.

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

### Progressive syntax: known limitations

Every statement and expression form is shared between the two front-ends (both
compile to the same AST), and the agent-header fields `goal:`, `retry:`,
`on_error:`, and the `on <expr> changes:` / `on cron "...":` trigger forms are
all supported in progressive syntax. Three brace-syntax constructs currently
have **no** progressive-syntax equivalent — using them requires classic brace
syntax:

- `receive { ... }` (channel definitions)
- `recipe "name" { ... }`
- `objective "name" { ... }`

Attempting any of these in a progressive-syntax file is a parse error at the
line in question, not a silent no-op — the rest of the file is unaffected if
you switch just that one agent to brace syntax. (The brace-syntax `can_do:`/
`capabilities:` field and `timeout:` on an agent header are parsed but never
read by the interpreter in *either* syntax — they are reserved for future use,
not a progressive-syntax gap.)

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
`remember.key` also works as an alias for `memory.key` in value position
(e.g. inside `"{remember.key}"` interpolation) — `remember` is the
*declaration* keyword above, `memory` is the conventional accessor, and the
two are easy to reach for interchangeably. Prefer `memory.key` in new code;
the alias exists so the natural mistake doesn't silently evaluate to `null`.

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

`obj.field = val` / `obj[key] = val` / `arr[i] = val` on a plain local
variable mutate in place — O(1) amortized for an array index, O(1) for an
object field/key, not a clone of the whole container. Building up an
object or array incrementally in a loop (`obj[key] = val` once per
iteration) is an ordinary O(n) loop, not O(n²).

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

Catches anything that *throws* — `Signal::Error`/`assert` inside `try_body`.
The caught value `e` is `{ message, kind, code }`, where `kind` is one of
`JsonParseError`/`NetworkError`/`PermissionError`/`NotFoundError`/
`AssertionError`/`RuntimeError`, guessed from the error message's text.
`catch <Kind> e { }` only matches that inferred kind; a bare `catch e { }`
matches anything. Not every builtin throws on failure, though — see
[Error Handling](#error-handling) for the other convention (`{ ok: false,
... }`) several runtime subsystems use instead, and `unwrap()` for
bridging the two.

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

// GX has no ternary operator — `cond ? a : b` is a parse error. `??`
// only covers the null/default-value case; for a general conditional
// expression, use `if`/`else` as a statement instead:
//   if cond { result = a } else { result = b }

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

A number of builtins accumulated more than one name across releases —
all forms in a group call the exact same implementation, so pick whichever
reads best; nothing here is deprecated. The first name in each group is
the one used elsewhere in this reference.

| Canonical | Also accepted |
|---|---|
| `now()` | `now_ms()` is a *different* function (milliseconds, not seconds) — `get_timestamp()`, `timestamp()` are the same as `now()` |
| `to_upper(s)` | `upper(s)`, `uppercase(s)` |
| `to_lower(s)` | `lower(s)`, `lowercase(s)` |
| `trim_start(s)` | `ltrim(s)` |
| `trim_end(s)` | `rtrim(s)` |
| `json_stringify(v)` | `to_json(v)`, `json(v)` |
| `json_parse(s)` | `parse_json(s)` |
| `substring(s, a, b)` | `substr(s, a, b)` |
| `regex_test`/`regex_find`/`regex_find_all`/`regex_replace`/`regex_split`/`regex_captures`/`regex_named_captures` | `re_test`/`re_find`/`re_find_all` (also `regex_findall`)/`re_replace`/`re_split`/`re_captures`/`re_named` |
| `vector_store_new`/`_add`/`_search`/`_delete`/`_size` | `vs_new`/`_add`/`_search`/`_delete`/`_size` |
| `assert_eq(a, b, msg?)` | `assert_equal(a, b, msg?)` |
| `assert_true(cond, msg?)` | `assert_that(cond, msg?)` |
| `xml_stringify(v)` | `xml_encode(v)` |
| `.unique()` | `.distinct()` |
| `.skip(n)` | `.drop(n)` |
| `.flatten()` | `.flat()` |
| `.contains(v)` | `.includes(v)` |
| `.push(v)` | `.append(v)` |
| `.remove(k)` | `.delete(k)` (object method) |

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

if verify_webhook(request.body, request.headers["x-signature"], env("WEBHOOK_SECRET")) {
  say "webhook verified"
} else {
  say "rejected: bad signature"
}
```

`request.headers` keys are always lowercased (HTTP header names are
case-insensitive per RFC 7230, but GX object keys aren't) — use
`request.headers["x-signature"]`, not `request.headers["X-Signature"]`. See
[HTTP Server](#http-server) for the full `request` object shape.

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
| `extname(path)` | Last extension, with the leading dot (`""` if there isn't one) |
| `path_join(a, b, ...)` | Join path segments (cross-platform) |
| `glob(pattern)` | Return array of paths matching shell glob pattern |

```gx
use std.fs   // optional

dirname("/home/user/file.txt")    // "/home/user"
basename("/home/user/file.txt")   // "file.txt"
extname("archive.tar.gz")         // ".gz" — the last extension only
extname("README")                 // ""
extname(".gitignore")             // "" — a dotfile has no extension
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
| `index_of(str, substr)` / `find(str, substr)` | Index of the first occurrence of `substr` in `str`, or `-1` |

```gx
truncate("hello world", 8)          // "hello w…"
truncate("hello world", 8, "...")   // "hello..."
truncate("hi", 20)                  // "hi" (no change)
```

`index_of`/`find` are overloaded by type, not just by name: as **free
functions** (above) they search for a *substring* in a *string*. As
**array methods** (`.index_of(v)`/`.find(v)`, see [Array
Methods](#array-methods)) they search for an exact *value* in an
*array*. `arr.index_of(v)` and `index_of(str, sub)` are unrelated
operations that happen to share a name — there's no `index_of(arr, v)`
free-function form for the array case.

**String methods** (no free-function form beyond the ones above — call
these as `s.method(...)`):

| Method | Description |
|---|---|
| `.trim()` / `.trim_start()` (`.ltrim()`) / `.trim_end()` (`.rtrim()`) | Strip whitespace |
| `.to_upper()` (`.upper()`) / `.to_lower()` (`.lower()`) | Case conversion |
| `.to_upper_first()` | Capitalize just the first character |
| `.split(sep)` | Split into an array |
| `.split_lines()` / `.lines()` | Split on newlines |
| `.replace(from, to)` | Replace every occurrence |
| `.replace_first(from, to)` | Replace only the first occurrence |
| `.starts_with(prefix)` / `.ends_with(suffix)` | Prefix / suffix check |
| `.contains(substr)` | Substring check |
| `.repeat(n)` | Repeat the string `n` times |
| `.pad_start(len, char?)` (`.pad_left(...)`) / `.pad_end(len, char?)` (`.pad_right(...)`) | Pad to `len` characters (space by default) |
| `.char_at(i)` | The character at index `i` |
| `.substring(a, b)` (`.substr(a, b)`) | Substring from `a` up to `b` |
| `.length()` / `.len()` | Character count |

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

### HTTP Client

| Function | Description |
|---|---|
| `http_get(url, headers?)` | GET request |
| `http_post(url, body, headers?)` | POST request (JSON body) |
| `http_put(url, body, headers?)` | PUT request (JSON body) |
| `http_delete(url, headers?)` | DELETE request |
| `http_request({url, method, body?, headers?})` | Unified form — any method |
| `http_stream({url, method?, body?})` | Line-buffered streaming response |
| `http_upload({url, fields?, files?})` | `multipart/form-data` upload |

**Result object** (every function above): `ok`, `status`, `body`, `body_bytes`,
`truncated`, `data` (auto-parsed JSON, only set when the body wasn't
truncated), and on failure `error`/`error_kind`.

```gx
r = http_get("https://api.example.com/users", { "Authorization": "Bearer " + token })
if r.ok {
  say "got {len(r.data)} users"
} else {
  say "request failed: {r.error_kind} — {r.error}"
}
```

`error_kind` is one of `http_status` (non-2xx response — `status`/`body`
are still populated), `timeout`, `dns_error`, `connection_failed`,
`too_many_redirects`, `blocked` (SSRF protection — see below),
`io_error`, or `transport_error` — check this instead of matching on the
free-text `error` string, which can vary by platform and dependency
version.

**Timeouts.** Every request has a 10s connect / 30s read default. Override
per call with a reserved `timeout` key (seconds) inside the `headers`/opts
object — it's popped out before the remaining entries are sent as real
headers:

```gx
r = http_get(url, { timeout: 5 })                         // GET
r = http_post(url, body, { timeout: 5 })                  // POST
r = http_request({ url: url, method: "PATCH", timeout: 5 }) // unified form
```

**Response size.** Bodies are capped at 32 MiB retained in memory; a larger
response still completes (`ok` reflects the real HTTP status) but sets
`truncated: true` and `body_bytes` to the real total, so truncation is
never silent. `data` (auto-parsed JSON) is only populated when the body
wasn't truncated, since a cut-off JSON document can't parse anyway.

**SSRF protection.** Requests to private/loopback/link-local addresses are
blocked by default — this is not just a check on the URL string. Every
connection ureq actually makes (including each redirect hop) is validated
against the *real resolved IP address* through a custom resolver, which is
what closes the classic SSRF bypasses a URL-string check alone misses:
- A URL naming an allowed external host that later redirects to an
  internal address (`169.254.169.254`, `localhost`, ...).
- A hostname that simply *resolves* to a private address.
- An IP address written in a non-dotted-decimal form
  (`http://2130706433/` is `127.0.0.1`).

`--allow-internal-http` allows internal/private addresses; `gx.json`'s
`capabilities.external_network: false` can restrict outbound requests to
public addresses too. See [Capability Runtime](#capability-runtime).

### HTTP Server

```gx
serve on port 8080 {
  route GET "/health" {
    respond json { ok: true }
  }

  route GET "/users/:id" {
    respond json { id: request.params.id }
  }

  route POST "/webhook" {
    sig = request.headers["x-signature"]
    if !secure_compare(hmac_sha256(env("WEBHOOK_SECRET"), request.body), sig) {
      respond text 401 "invalid signature"
    } else {
      respond json { received: true }
    }
  }
}
```

**The `request` object**, available in every route body:

| Field | Description |
|---|---|
| `request.method` | `"GET"`, `"POST"`, ... |
| `request.path` | Request path, no query string |
| `request.body` | Raw request body as a string |
| `request.json` | Auto-parsed body, only set when it's valid JSON |
| `request.query` | Raw query string (e.g. `"a=1&b=2"`) |
| `request.query_params` | Parsed query string as an object (percent-decoded) |
| `request.params` | Named path segments (`:id` → `request.params.id`) |
| `request.headers` | All request headers, **keys always lowercased** |
| `request.remote_addr` | Client's `"ip:port"`, or `null` if unavailable |

**Routes and path parameters.** `route METHOD "path" { ... }` — `METHOD` is
`GET`/`POST`/`PUT`/`DELETE`/`ANY` (matches any method). A path segment
starting with `:` captures into `request.params`: `"/users/:id/posts/:post_id"`
matches `/users/42/posts/7` with `request.params == {id: "42", post_id: "7"}`.
Routes are checked in declaration order; the first match wins.

**Responses.** `respond json { ... }`, `respond html "..."`, `respond text
"..."` (default format), each optionally taking a status code:
`respond json 201 { id: new_id }`. A route that finishes without calling
`respond` returns `200 OK` with an empty body.

**Errors are never leaked to the client.** If a route throws (a GX error,
not an explicit `respond`), the server returns a generic `500 Internal
Server Error` to the caller and logs the full error — including the
route's file/line — to stderr. The client never sees internal details
(file paths, capability-denial reasons, stack-adjacent context) that a
raw error message could otherwise expose.

**Concurrency.** The server runs a fixed pool of 8 worker threads (each
with its own private `Interpreter` sharing the program's definitions and
capabilities — inheriting the same way `spawn agent`/`parallel {}` do),
calling `recv()` on the shared listener. A slow route (waiting on an AI
provider, an outbound HTTP call, a subprocess) no longer blocks every
other route, including unrelated webhooks — this was a single-threaded,
one-request-at-a-time loop before this milestone.

**Request size.** Bodies are capped at 32 MiB; a larger request is
rejected with `413 Payload Too Large` before the route ever runs, whether
or not the client's `Content-Length` was honest about the size.

#### Server-Sent Events (streaming responses)

`respond stream { ... }` keeps the connection open and lets the route send
one frame at a time with `sse_send(event?, data)`, instead of producing a
single buffered response:

```gx
route GET "/progress" {
  respond stream {
    i = 0
    while i < 100 {
      sse_send("progress", { percent: i })
      sleep(0.1)
      i += 100 / 10
    }
    sse_send("done", { percent: 100 })
  }
}
```

`sse_send(data)` sends an unnamed event; `sse_send(event, data)` sets the
SSE `event:` field. `data` is JSON-encoded unless it's already a string.
`sse_send` blocks if the client is reading slowly (a bounded channel
provides real backpressure rather than buffering an unbounded amount of
unsent data in memory) and returns an error if the client has
disconnected — check for that if a long-running stream should stop
producing data once nobody's listening.

`sse_send` is only valid inside a `respond stream { ... }` block; calling
it from a normal route (or outside `serve` entirely) is an error.

WebSocket upgrade isn't implemented, but the underlying server library
(`tiny_http`) supports connection takeover, so the worker-pool
architecture here doesn't preclude adding it later — it's a scoped future
extension, not a redesign.

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
| `config_load(options)` | Layered config: defaults < file < env overrides < explicit overrides, with optional schema validation — see [Configuration Runtime](#configuration-runtime) |

### Script arguments

| Function | Description |
|---|---|
| `argv()` | Array of every positional argument after a literal `--` in `gx run file.gx -- arg1 arg2` |
| `script_args()` | Alias for `argv()` |

`gx run file.gx -- arg1 arg2` (or the `gx file.gx -- arg1 arg2` shorthand)
passes `arg1`/`arg2` through to the script unchanged — including a value
that happens to look like one of `gx`'s own flags (`gx run file.gx --
--allow-shell` passes the literal string `"--allow-shell"` to the
script; it does not grant that capability). Without a `--`, `argv()`
returns `[]`. `gx run file.gx foo bar` (no `--`) does **not** make
`foo`/`bar` reachable — `gx`'s own flag parsing still owns everything
before `--`.

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
| `xml_parse(str)` | Parse XML into `{ tag, attrs, children, text }` |
| `xml_stringify(v)` (`xml_encode(v)`) | Serialize a `{ tag, attrs?, children?, text? }` element back to XML text |

```gx
doc = xml_parse("<book id=\"1\"><title>GX</title><author>Ada</author></book>")
doc.tag              // "book"
doc.attrs.id         // "1"
doc.children[0].tag  // "title"
doc.children[0].text // "GX"

xml_stringify({ tag: "note", attrs: { priority: "high" }, text: "Ship it" })
// <note priority="high">Ship it</note>
```

`xml_parse` is intentionally narrow, not a full XML implementation:
mixed content (text interleaved with child elements, e.g. `<p>Hello
<b>world</b>!</p>`) collapses into separate `text` and `children` fields
rather than preserving their relative order — enough for config files and
simple data/API documents, not a document markup format. More
importantly, **no DTD or entity definitions are ever processed** — only
the five predefined XML entities (`&amp;` `&lt;` `&gt;` `&quot;`
`&apos;`) and numeric character references (`&#65;` / `&#x41;`) are
recognized; anything else, including a custom entity a document tries to
define via `<!DOCTYPE>`, is a parse error. This is deliberate, not a
missing feature — it's the standard defense against XXE (XML External
Entity) injection and "billion laughs" entity-expansion attacks, both of
which are built entirely out of entity definitions this parser never
resolves in the first place.

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
| `random()` / `random(lo, hi)` | Random float; `[0.0, 1.0)` by default, `[lo, hi)` with bounds |
| `random_int(min, max)` | Random **integer**, inclusive of both `min` and `max` |
| `random_choice(arr)` | A random element of `arr`; `null` if `arr` is empty |
| `shuffle(arr)` | A new array with `arr`'s elements in random order (does not mutate `arr`) |
| `set_random_seed(n)` | Makes every `random`/`random_int`/`random_choice`/`shuffle` call deterministic for the rest of the run — see [Testing Framework](#testing-framework) |
| `pi` | 3.14159... |
| `e` | 2.71828... |

`random_int`/`random(lo, hi)` are easy to mix up: `random(0, 10)` is a
**float** in `[0, 10)` (10 excluded); `random_int(0, 10)` is an **integer**
in `[0, 10]` (10 included). Reach for `random_int` whenever you want a
whole number — `floor(random() * (max - min + 1)) + min` is the
hand-rolled equivalent, and it's exactly the kind of expression that's
easy to get off-by-one wrong (forgetting the `+ 1`, or not flooring).

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
| `date_now()` | Current time as an ISO-8601 string |
| `date_timestamp()` | Current Unix timestamp, in **seconds** |
| `date_parse(str, format?)` | Parse a date string → Unix timestamp (number). Auto-detects ISO-8601/RFC 2822 and several common formats with no `format`; pass a strftime pattern (e.g. `"%Y-%m-%d"`) for anything else |
| `date_format(date, fmt?)` | Format a timestamp or date string with a strftime pattern (default `"%Y-%m-%d"`) |
| `date_diff(a, b, unit)` | Difference in `"seconds"`, `"minutes"`, `"hours"`, `"days"`, `"weeks"`, `"months"`, `"years"` |
| `date_add(date, n, unit)` | Add `n` `unit`s to a date — **always returns a Unix timestamp (number)**, even when `date` was an ISO string. See the callout below. |
| `date_add_iso(date, n, unit)` | Same arithmetic as `date_add`, but always returns an ISO-8601 string — the recommended function when the result will be stored or compared as a string (e.g. alongside `date_now()` in a `next_action_at` column) |
| `date_parts(date)` | Object: `{ year, month, day, hour, minute, second, weekday, weekday_name, timestamp, iso }` |
| `date_from_parts(year, month, day, hour?, minute?, second?)` | Build a Unix timestamp from individual parts |

Every function above **accepts** either a Unix timestamp or an ISO-8601
string as a date argument, interchangeably. On the **output** side,
though, only `date_now()` (and the `.iso`/`.weekday_name` fields of
`date_parts()`) return a string — `date_add`, `date_parse`, and
`date_from_parts` all return a plain number, matching this module's
internal canonical representation rather than whatever shape you gave
it. This is easy to trip over: `date_add(date_now(), 4, "days")` takes
a string in and silently hands back a number, and storing that number
into a column that otherwise holds ISO strings — then comparing it
against another `date_now()` value — produces a **string comparison**
between a numeric-looking string and an ISO string, which is silently
always true (`"1784369596"` sorts before any `"2026-..."` string
lexicographically) regardless of the real date. Use `date_add_iso`
instead of `date_add` (or wrap it: `date_parts(date_add(...)).iso`)
whenever the result needs to stay string-typed.

### Array Methods

**Functional — also work as free functions** (`arr.map(fn)` and
`map(arr, fn)` call the exact same code; use whichever reads better at
the call site):

| Method | Free function | Description |
|---|---|---|
| `.map(fn)` | `map(arr, fn)` | New array of `fn(item)` for each item |
| `.filter(fn)` | `filter(arr, fn)` | New array of items where `fn(item)` is truthy |
| `.reduce(fn, initial)` | `reduce(arr, fn, initial)` | Folds left: `acc = fn(acc, item)` for each item, starting from `initial` (required — an empty array just returns `initial` unchanged) |
| `.some(fn)` | `some(arr, fn)` | `true` if `fn(item)` is truthy for *any* item (`false` on an empty array) |
| `.every(fn)` | `every(arr, fn)` | `true` if `fn(item)` is truthy for *every* item (vacuously `true` on an empty array) |
| `.find_index(fn)` | `find_index(arr, fn)` | Index of the first item where `fn(item)` is truthy, or `-1` — **predicate**-based, distinct from `.index_of(v)`/`.find(v)` below, which look for an exact value match |

`map`/`filter` also work on an object (iterating its keys as strings) and
a string (iterating one character at a time) — not array-only, since the
free-function forms already worked that way before methods existed for
them.

**Method-only** (no free-function form — call these as `arr.method(...)`,
not `method(arr, ...)`):

| Method | Description |
|---|---|
| `.push(v)` / `.append(v)` | Append element. As a bare statement (`arr.push(v)` **or** the common agent-memory shape `memory.field.push(v)`, one level of nesting), mutates the underlying array in place, in O(1) amortized time. As a captured expression (`x = arr.push(v)`), functional: returns a new array, the original unchanged — the idiomatic accumulator pattern `results = results.push(v)` relies on this. **Known limitation**: two or more levels of nesting as a bare statement (`a.b.c.push(v)`) isn't recognized by the in-place fast path and silently doesn't mutate anything (the computed new array is a bare statement's discarded expression value) — reassign explicitly instead: `tmp = a.b.c; tmp.push(v); a.b.c = tmp`. |
| `.pop()` | Removes the last element **and mutates the array in every context**, including when the result is captured (`x = arr.pop()` shrinks `arr` and gives `x` the removed value) and one level of nesting (`x = memory.items.pop()`) — unlike `.push()` above, there's no useful "functional pop" reading where the array staying unchanged would make sense, so this isn't scoped to bare statements the way `.push()`'s in-place fast path is. |
| `.shift()` | First element (does not remove it) |
| `.unshift(v)` | Prepend element (returns new array) |
| `.sort()` | Sort ascending. As a bare statement, mutates in place; as a captured expression, functional (same distinction as `.push()`). |
| `.reverse()` | Reverse order. Same bare-statement-mutates / captured-expression-functional distinction as `.push()`/`.sort()`. |
| `.unique()` / `.distinct()` | Deduplicate |
| `.flatten()` / `.flat()` | Flatten one level |
| `.concat(...)` | Append more arrays/values (spreading arrays, pushing anything else as-is) |
| `.slice(start, end?)` | Sub-array from `start` up to (excluding) `end`; `end` defaults to the array's length |
| `.sum()` | Sum of numeric elements |
| `.min()` / `.max()` | Min / max value |
| `.average()` | Mean value |
| `.take(n)` | First `n` elements |
| `.skip(n)` / `.drop(n)` | Skip the first `n` elements |
| `.join(sep)` | Join to string |
| `.filter_by(key, value)` | Filter an array of objects by a field's value |
| `.map_field(key)` | Extract one field from each object |
| `.contains(v)` / `.includes(v)` | Membership check (exact value equality) |
| `.index_of(v)` | Index of the first element equal to `v`, or `-1` — value-*equality*, not predicate-based (see `.find_index(fn)` above for that) |
| `.find(v)` | The first element equal to `v`, or `null` |
| `.first()` / `.last()` | First / last element |
| `.length()` / `.len()` / `.count()` | Number of elements |

### Object

| Function | Description |
|---|---|
| `keys(obj)` | Array of keys |
| `values(obj)` | Array of values |
| `entries(obj)` | Array of `[key, value]` pairs |
| `merge(obj, ...)` | Shallow-merge objects left to right (later wins) |
| `has(obj, key)` | Key existence check |
| `pick(obj, keys)` | New object containing only the named keys that are actually present |
| `omit(obj, keys)` | New object excluding the named keys |
| `group_by(arr, key)` | Group array of objects by field |

```gx
user = { id: 1, name: "Ada", email: "ada@example.com", password_hash: "..." }

public = pick(user, ["id", "name"])   // { id: 1, name: "Ada" }
safe   = omit(user, ["password_hash"]) // everything except password_hash
```

`pick`/`omit` never error on a key that isn't present — a name in `keys`
that doesn't exist on `obj` is silently skipped, the same "missing means
absent, not an error" convention `has`/`keys`/`values` already use for a
non-object input.

Also available as **methods** on an object value — `obj.has_key(k)` /
`obj.get(k, default?)` (returns `default`, or `null`, if the key is
missing) / `obj.remove(k)` (alias `delete`) / `obj.pick(keys)` /
`obj.omit(keys)` / `obj.is_empty()` / `obj.len()` (aliases `length`/
`count`) / `obj.to_json()`.

### Reliability & Introspection

See [Error Handling](#error-handling) for `retry`/`unwrap` and
[Capability Runtime](#capability-runtime) for `has_capability`.

| Function | Description |
|---|---|
| `retry(fn, max?, opts?)` | Calls `fn()` until it succeeds — a *thrown* error or a returned `{ ok: false, ... }` both count as a failure worth retrying. `opts`: `delay` (ms, default 1000), `backoff` (`"exponential"`\|`"linear"`\|`"fixed"`, default `"exponential"`). Returns the final attempt's outcome unchanged once `max` (default 3) is exhausted. |
| `unwrap(result)` | If `result` is `{ ok: false, error, error_kind, ... }`, throws (catchable via `try/catch`, same as `db_query`'s own failures). Anything else passes through unchanged. |
| `has_capability(resource, name?)` | `true`/`false` — would this resource/name currently be authorized? Never throws, never has a side effect. |

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

| Function | Description |
|---|---|
| `db_query(path, sql, params)` | Run a SELECT, returns an array of row objects |
| `db_exec(path, sql, params)` | Run an INSERT/UPDATE/DELETE/DDL statement, returns rows affected |
| `db_transaction(path) { ... }` | Run a block atomically — see below |
| `db_migrate(path, [sql, ...])` | Apply pending schema migrations — see below |
| `db_integrity_check(path)` | `{ok, errors}` — SQLite's own `PRAGMA integrity_check` |
| `db_vacuum(path)` | Rebuild the file, reclaiming space from deleted rows |
| `db_backup(path, dest_path)` | Online backup, safe against a concurrently-written source |

```gx
rows  = db_query("app.db", "SELECT * FROM users WHERE active = ?", [true])
count = db_exec("app.db", "INSERT INTO events (name) VALUES (?)", ["login"])
```

**Always use `?` placeholders and a `params` array for values — never build
SQL by concatenating or interpolating untrusted input.** This is the only
thing standing between a script and SQL injection; GX doesn't and can't
sanitize a string that's already been spliced into a SQL statement before
`db_query`/`db_exec` ever sees it.

```gx
// Right — the driver escapes `name` correctly no matter what it contains:
db_exec(db, "INSERT INTO users(name) VALUES (?)", [name])

// Wrong — do not do this, regardless of any manual escaping applied first:
db_exec(db, "INSERT INTO users(name) VALUES ('" + name + "')")
```

**Connections are pooled per path**, not opened fresh on every call — the
first `db_query`/`db_exec`/`db_transaction` call for a given path opens the
connection and configures it (`PRAGMA journal_mode=WAL`, `busy_timeout=5000ms`,
`foreign_keys=ON`); every later call to that same path within the same `gx`
process reuses it, with prepared statements cached automatically. WAL mode
in particular is what lets multiple `serve` routes read and write the same
database concurrently without "database is locked" errors — see
[HTTP Server](#http-server)'s concurrency notes; the two features are
designed to work together. One connection is kept open per distinct path
for the life of the process — fine for the common case of a small, fixed
set of database files; a script that generates many dynamically-named
database paths at runtime will accumulate one pooled connection per
distinct path with no eviction today. `:memory:` is a valid path and gets
pooled the same way, which means (unlike raw SQLite, where every
`:memory:` connection is normally independent) every `:memory:` call
within one `gx` process shares the *same* in-memory database — a
convenient way to get a scratch database for a script's lifetime without
managing a handle yourself.

#### Transactions and nested transactions

```gx
db_transaction(db_path) {
  db_exec(db, "INSERT INTO accounts(name, balance) VALUES (?, ?)", [name, 0])
  db_exec(db, "INSERT INTO audit_log(action) VALUES (?)", ["account_created"])
}
```

The block commits if it finishes normally, and rolls back if it throws —
`db` is a variable bound inside the block to the same path passed to
`db_transaction`, for `db_exec(db, ...)`/`db_query(db, ...)` calls inside it.
Any read-then-write sequence that needs to stay consistent under concurrent
access (check a row exists, then update it; delete related rows across two
tables) belongs in a transaction — without one, another connection can
observe or modify the same rows in between your read and your write.

Also available in progressive syntax, as a block header (same for
`span("name"):`, below):

```
db_transaction(db_path):
  db_exec(db_path, "INSERT INTO accounts(name, balance) VALUES (?, ?)", [name, 0])
  db_exec(db_path, "INSERT INTO audit_log(action) VALUES (?)", ["account_created"])
```

`db_transaction` blocks **nest correctly** using real SQLite savepoints — a
transaction started while another is already active *on the same path*
becomes a savepoint instead of a fresh `BEGIN`, so a reusable "does its own
transaction" helper works whether it's called standalone or from inside a
larger transactional workflow:

```gx
function record_purchase(db_path, user_id, amount) {
  db_transaction(db_path) {
    db_exec(db, "UPDATE accounts SET balance = balance - ? WHERE id = ?", [amount, user_id])
    db_exec(db, "INSERT INTO purchases(user_id, amount) VALUES (?, ?)", [user_id, amount])
  }
}

// record_purchase's own transaction becomes a savepoint here — if the
// audit insert below fails, only record_purchase's own work rolls back,
// not the whole outer transaction:
db_transaction(db_path) {
  record_purchase(db_path, user_id, 42)
  db_exec(db, "INSERT INTO audit_log(action) VALUES (?)", ["purchase_flow"])
}
```

A `db_query`/`db_exec` call always operates on the database file it's
actually given, regardless of what transaction (if any) is active
elsewhere — calling `db_exec("other.db", ...)` from inside
`db_transaction("main.db") { ... }` runs against `other.db` in its own
auto-commit connection, not against `main.db`'s open transaction.

#### Migrations

```gx
db_migrate(db_path, [
  "CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT)",
  "ALTER TABLE users ADD COLUMN email TEXT",
  "CREATE INDEX idx_users_email ON users(email)"
])
```

Version tracking uses SQLite's own `PRAGMA user_version` — no separate
tracking table to create or get out of sync. `migrations[i]` runs when the
database's current version is `<= i`; each migration commits in its own
transaction, so a failure partway through leaves the database at the last
successfully applied version, not partially migrated. Calling `db_migrate`
again with the same list (or a prefix of it, e.g. after adding new entries
to the end) only applies what's new — safe to call on every application
startup.

#### Maintenance

```gx
check = db_integrity_check(db_path)
if !check.ok { log("corruption detected: " + json_stringify(check.errors)) }

db_vacuum(db_path)                    // reclaim space from deleted rows
db_backup(db_path, "backup.db")       // safe even while db_path is being written concurrently
```

`db_vacuum`/`db_migrate` refuse to run while a transaction is active on the
same database (matching SQLite's own restriction on `VACUUM` mid-transaction).
`db_backup` uses SQLite's online backup API, not a file copy — a plain
`read_file`/`write_file` copy of a live database can capture a torn,
inconsistent snapshot if something else is writing to it at the same
moment; the backup API is specifically designed to be safe against that.

#### Binary data

GX has no native binary value type. A BLOB column reads back as a base64
string automatically; to store binary data, base64-encode it into a TEXT
column rather than trying to write a BLOB directly:

```gx
db_exec(db, "CREATE TABLE files(id INTEGER PRIMARY KEY, content_b64 TEXT)", [])
db_exec(db, "INSERT INTO files(content_b64) VALUES (?)", [base64_encode(file_bytes)])
// ... later:
bytes = base64_decode(row.content_b64)
```

### Persistent Memory

```gx
persist_memory()   // save memory to ~/.gx/state/<agent>.db
load_memory()      // restore from SQLite
```

### Observability

```gx
trace_log("event.name", { key: value })   // free-form named event, tagged with the current trace/span
log_debug(msg, data?)  log_info(msg, data?)  log_warn(msg, data?)  log_error(msg, data?)
span("name") { ... }   trace_id()   span_id()
```

Full reference, automatic instrumentation, and production guidance:
[Diagnostics & Observability](#diagnostics--observability).

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

These are single-shot: each call builds its own prompt with no memory of
previous calls. For multi-turn conversations, token budgeting, automatic
trimming, and tool-call round-trips, see
[AI Context Runtime](#ai-context-runtime).

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

// Structured output (openai only — a direct pass-through to OpenAI's own
// `response_format`; Anthropic has no native equivalent to pass through to)
result = ask openai {
  prompt: "Extract the name and age as JSON.",
  response_format: { type: "json_object" }
}

// Per-call timeout override (seconds) — every provider honors this,
// including ollama, routed through the same pooled connection as
// openai/anthropic (just with `internal_network` pre-authorized, since
// ollama is inherently a loopback endpoint).
result = ask ollama {
  prompt: "...",
  timeout: 10
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
| `result.error_kind` | String or null | On failure: `rate_limited`/`auth_error`/`invalid_request`/`server_error`/`timeout`/`network_error`/`http_error`/`unknown`. `null` on success. |
| `result.retry_after_ms` | Number or null | The provider's `Retry-After` header, in milliseconds, when it sent one. |

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

Two agent shapes exist, and they stay genuinely distinct — not two
spellings of the same thing:

- **`brain { }`** — a synchronous, callable agent. `spawn agent` calls it
  and gets back whatever `communicate` produces.
- **`when message "event" { }`** — an asynchronous event handler. It runs
  in response to `send`/`spawn "event" to "agent"` and never returns a
  value to any caller, regardless of how it's invoked.

```gx
agent "summarizer" {
  brain {
    plan { }
    execute { summary = input.text }
    remember { }
    communicate { summary }
  }
}

agent "classifier" {
  when message "classify" {
    log("classifying: {message.text}")
  }
}
```

A handler's contract comes from its own declaration, not from whichever
call form happens to invoke it — the same reason Erlang/OTP keeps
`handle_call` (synchronous, expects a reply) and `handle_cast`
(asynchronous, no reply) as separate callbacks rather than letting one
opportunistically satisfy the other. `when message` is GX's `handle_cast`;
`brain { }` is GX's `handle_call`.

### `spawn agent` — Call a brain{} Agent and Get a Value Back

```gx
result = spawn agent "summarizer" with { text: "hello world" }
log(result)
```

`spawn agent` only ever calls a `brain { }`. Targeting an agent with no
`brain { }` — including a `when message`-only agent, even when the
`action` given names a real handler — fails with a clear error naming the
agent and what it actually exposes, instead of silently returning `null`:

```
Agent "classifier" cannot be called synchronously.

This agent only exposes asynchronous `when message` handlers (classify) —
no `brain { }` block. A `when message` handler never returns a value to a
`spawn agent` caller, regardless of the `action` passed — it only runs in
response to `send`/`spawn "action" to "agent"`.

Use:
  spawn "action" to "classifier" with { ... }
or convert the agent to a `brain { }` implementation.
```

### `|>` Pipeline

```gx
result = { value: 5 } |> spawn agent "doubler" |> spawn agent "formatter"
```

Non-object values are auto-wrapped as `{ value: X }`.

### `spawn "event" to "agent"` — Fire-and-Forget

```gx
spawn "task" to "worker" with { task: "process data" }
```

No return value — the target's matching `when message "task"` handler runs,
but the caller doesn't wait for or receive anything back. Use this instead
of `spawn agent` when the target is meant to be asynchronous by design (a
notification, a background job) rather than because it happens to lack a
synchronous return path.

**The target agent must exist.** `spawn "task" to "worker"` fails
immediately with a clear error if no agent named `"worker"` is declared
anywhere in the project — a typo'd or removed agent name is caught rather
than silently queuing a message nothing can ever deliver. This is
narrower than "no matching handler": an agent that exists but doesn't
(yet) declare a `when message "task"` handler is unaffected and still
queues the message for deferred delivery, exactly as before — only a
genuinely undeclared *agent* is an error.

---

## Package Interop

### Bare-specifier form — an installed package

```gx
use js.path
result = js.path.join("/home", "user")     // 3-part: namespace.module.method(...)
result = path.join("/home", "user")        // 2-part: module.method(...) — same call

use py.os
cwd = py.os.getcwd()

use ts.analytics
report = ts.analytics.generate(data)
```

`path`/`os`/`analytics` are resolved as bare specifiers — `require('path')`
(Node's own module resolution, so `node_modules`/`NODE_PATH` apply) or
Python's `importlib.import_module`. The 2-part form (`path.join(...)`, no
`js.` prefix) and the 3-part form (`js.path.join(...)`) are exactly
equivalent — both resolve to the same call. Use whichever reads better at
the call site.

### Quoted-path form — a local project file or executable

```gx
use js "./scripts/playwright_bridge.js" as playwright_bridge
playwright_bridge.navigate({ url: "https://example.com" })

use py "./scripts/ocr.py" as ocr
ocr.extract_text("scan.png")

use binary "./bin/my_processor" as processor
output = processor.transform(payload)

use go "./services/search" as search
results = search.query("hello")
```

Use this form to point a bridge at a file that's part of your own project
instead of an installed package — `require()`/`importlib` never has to find
it via `node_modules`/`PYTHONPATH`, and relative paths resolve against the
directory `gx run` was invoked from, not wherever the bridge's internal
runner process happens to live. `as <alias>` is optional; without it, the
alias defaults to the path's file stem (`"./scripts/playwright_bridge.js"`
→ `playwright_bridge`).

**How it works:**
- JS/Python/TypeScript calls: a persistent child process per bridge, speaking
  a newline-delimited JSON IPC protocol — not a new subprocess per call
- Go/Binary: compiled binary with the same JSON stdin/stdout protocol

See **Writing a Bridge Script** below for the shim's actual calling
convention — the thing every bridge script author needs and no prior
example showed.

### Writing a Bridge Script

A bridge script is a **plain, importable module** — it does not open its own
stdin/stdout loop, does not parse JSON itself, and does not know it's being
called from GX at all. GX's own shim process owns the entire IPC loop; your
script just exposes ordinary top-level functions, called positionally
(`fn_ref(*args)`). Writing a bridge script as a standalone program with its
own dispatch loop is the single most common mistake — on Python specifically,
it doesn't just fail, it **hangs** for the full call timeout, since the
script's own blocking read races the shim's.

A complete, correct JS bridge script:

```js
// scripts/greeter.js
function greet(name) {
  return "hello " + name;
}

module.exports = { greet };
```

```gx
use js "./scripts/greeter.js" as greeter
log(greeter.greet("world"))   // "hello world"
```

A complete, correct Python bridge script:

```python
# scripts/greeter.py
def greet(name):
    return "hello " + name
```

```gx
use py "./scripts/greeter.py" as greeter
log(greeter.greet("world"))   // "hello world"
```

That's the entire contract: define functions, export/expose them at module
level (`module.exports` in JS; top-level `def` in Python — no `if __name__ ==
"__main__":` dispatch), and let GX's shim do the rest. Nested method access
(`obj.method.deeper`) and promise/async results are handled transparently on
the JS side; a Python function can return any JSON-serializable value.

---

## Capability Runtime

Every GX subsystem that touches something outside the interpreter's own
memory — the filesystem, a subprocess, a network socket, a database file, an
AI provider, a bridge module — authorizes through one place:
`crate::capability::Capabilities`. No subsystem implements its own
allow/deny logic; it asks the Capability Runtime, which decides, and the
subsystem executes (or doesn't).

This is deliberately framed as *capability*, not *permission*. A permission
check answers "can I execute this operation right now?" at the call site
where it happens. A capability answers "what resources is this program
allowed to access?" — a property of the whole running program, decided once,
in one place, and consulted consistently everywhere. That's what lets new
GX subsystems (a future package manager, native plugins, a distributed
runtime) integrate into the same model without inventing their own.

### What's gated

| Resource | Default | Grantable via |
|---|---|---|
| `shell` (`shell()`/`exec()`) | denied | `--allow-shell` only |
| `process` (`process_run`/`process_spawn`) | denied | `--allow-process` only |
| `internal_network` (HTTP to private/loopback addresses) | denied | `--allow-internal-http` only |
| `external_network` (HTTP to public addresses) | **open** | restrict via `gx.json` |
| `http_server` (`serve on port ...`) | **open** | restrict via `gx.json` |
| `database` (`db_query`/`db_exec`/`db_transaction`/`db_migrate`/`db_vacuum`/`db_backup`/`db_integrity_check`) | **open** | restrict via `gx.json` |
| `environment` (`env`/`get_env`/`set_env`) | **open** | restrict via `gx.json` (denylist) |
| `ai` (`ask`/`embed`/`infer classifier`) | **open** | restrict via `gx.json` (allowlist) |
| `js`/`ts`/`py`/`binary`/`go`/`rust_bin` (bridge modules/executables) | **open** | restrict via `gx.json` (allowlist) |
| `filesystem` | sandboxed to script dir | `--no-sandbox` (unrestricted) |

**Why the split.** `shell`, `process`, and `internal_network` are the three
resources that can execute arbitrary code or reach otherwise-unreachable
network addresses — they stay deny-by-default and only a CLI flag can grant
them; `gx.json` can narrow what they're allowed to do (e.g. the process
executable allowlist) but can never turn them on. `gx.json` ships next to
the script it describes, so a manifest is only as trustworthy as the script
itself — an explicit CLI flag is a decision made by whoever *invokes* `gx`,
a stronger, out-of-band signal. Everything else in the table above was
already unconditionally available in earlier GX versions (this is what
"backward compatible" requires), so it stays open by default; what's new is
that it's now *possible* to restrict, through the same manifest mechanism
GX already used for `dependencies.js`/`dependencies.py`.

### CLI flags

| Flag | Effect |
|---|---|
| *(default)* | File I/O sandboxed to script dir; shell, process, internal-network blocked |
| `--allow-shell` | Enable `shell()`/`exec()` |
| `--allow-process` | Enable `process_run`/`process_spawn` (independent of `--allow-shell`) |
| `--allow-internal-http` | Allow HTTP to private/localhost IPs |
| `--no-sandbox` | Disable file-path sandboxing |
| `--project-sandbox` | Sandbox to the nearest ancestor directory with a `gx.json`, instead of just this script's own directory (falls back to the default if no ancestor has one — never widens access beyond what was asked for) |
| `--deny <resource>` | Force-deny a resource, overriding everything else (repeatable) |
| `--no-limit` | Remove while-loop iteration cap |

`--project-sandbox` exists for a project laid out in subdirectories
(`agents/`, `lib/`, a shared `data/`) where an agent needs to reach a
sibling directory it wouldn't otherwise be able to under the default
per-script sandboxing — a level of access between "sandboxed to this
exact script's own directory" and `--no-sandbox`'s "no path restriction
at all". `gx.json`'s own capability/dependency declarations are loaded
from that same discovered root when the flag is used, so a manifest
that lives at the project root (rather than beside every individual
script) is now actually reachable too.

`--deny` always wins — not even `--allow-shell` can override a matching
`--deny shell`. It exists for the operator invoking `gx` (a deployment
script, a CI job) to enforce a stricter posture than the script or its
manifest asks for, without having to edit either.

### gx.json schema

```json
{
  "dependencies": {
    "js": ["axios"],
    "ts": ["some-module"],
    "py": ["requests"],
    "binary": ["./my_service"],
    "go": ["./my_go_service"],
    "rust_bin": ["./my_rust_service"],
    "process": ["git", "docker"],
    "ai": ["anthropic", "openai"]
  },
  "capabilities": {
    "http_server": false,
    "database": false,
    "external_network": false,
    "env_deny": ["AWS_SECRET_ACCESS_KEY", "DATABASE_PASSWORD"]
  }
}
```

`dependencies.*` allowlists apply the same rule for every namespace:
**declaring a list restricts that namespace to exactly those names; an
undeclared key stays open.** This is the same convention GX already used
for `dependencies.js`/`dependencies.py`/`dependencies.process`, just
extended uniformly to `ts`/`binary`/`go`/`rust_bin`/`ai` — those previously
had *no* allowlist mechanism at all.

Each `dependencies.*` array element must be a plain string
(`"axios"`, `"./scripts/foo.js"`) — not an object. `gx run`/`gx check` now
reject the whole manifest with a clear error if an element isn't a string,
rather than silently dropping it: an array of `{"name": ..., "path": ...}`
objects used to filter down to an *empty* allowlist, which denies
everything in that namespace with no signal that the shape, not the intent,
was wrong.

`capabilities.*` restricts the resources that aren't a name list: set to
`false` to deny, omit to leave at the default (open). `env_deny` is an exact
list of environment variable names to block — not a glob pattern — reading
or writing any other variable is unaffected.

Both sections are loaded from the directory containing the script being run
(or the current directory for `gx -e`/`gx eval`) — independent of whether
`--no-sandbox` was passed. File-path sandboxing and the dependency/capability
allowlists are different concerns; disabling one no longer silently disables
the other.

### Precedence

1. `--deny <resource>` — operator override, always wins.
2. `--allow-shell` / `--allow-process` / `--allow-internal-http` — the only
   way to grant these three; `gx.json` cannot.
3. `gx.json`'s `dependencies.*` / `capabilities.*` — narrows what's open by
   default, or narrows the process/bridge allowlist once a CLI flag has
   granted the underlying resource.
4. Built-in default (the table above).

### Static Diagnostics (`gx check`)

`gx check <file.gx>` parses the file plus everything it transitively
`import`s (the whole project, not just that one file) and runs a set of
static checks — no statement is ever executed. Findings print as
`file:line: error|warning: message`; any `error`-severity finding makes
`gx check` exit non-zero.

| Check | Severity | Catches |
|---|---|---|
| Spawn target has no `brain{}` | error | `spawn agent "x" with {...}` targeting an agent with no `brain{}` — including a `when message`-only agent, regardless of whether `action` names a real handler. `brain{}` and `when message` stay distinct concepts; this always fails at runtime either way. |
| Fire-and-forget target doesn't exist | error | `spawn "event" to "x"` where `"x"` isn't declared anywhere in the project — the message can never be delivered. Only the *agent* not existing is flagged; an existing agent with no matching `when message` handler yet is unaffected. |
| Agent declared but never spawned | warning | An agent whose name never appears as a literal `spawn agent`/`spawn ... to` target anywhere in the project — dead code, or a half-wired refactor. Agents that auto-run standalone (`when started`, `when cron`, or a `brain{}` that never references `input`) are correctly excluded — they're not meant to be spawned. |
| Cross-file function/agent name collision | warning | Two different imported files independently defining the same function or agent name (`import`'s "last one wins" behavior) — previously only visible as a runtime log line. |
| SQL built by concatenation/interpolation | warning | `db_exec`/`db_query`'s SQL argument built with `+` or `"...{var}..."` instead of a `?` placeholder — the textbook SQL-injection shape. |

Every check is intentionally conservative: a dynamically-constructed spawn
target (not a string literal) is skipped rather than guessed at, to keep
the false-positive rate low enough to trust in CI.

### Spawned agents and `parallel {}`

`spawn agent "name" with { ... } timeout N` and `parallel { key: spawn agent
... }` each run on their own OS thread with their own `Interpreter`. That
child interpreter inherits the parent's full capability grants — it does
not start from a fresh, all-denied default. A multi-agent program that was
granted `--allow-process` at the top level can use `process_run` from
inside a spawned agent exactly as if it were called directly.

`parallel` has two forms: `results = parallel { a: expr, b: expr }` (an
*expression* — each key becomes a concurrent branch, the whole thing
evaluates to `{ a: ..., b: ... }`) and a bare `parallel { stmt1 stmt2 }`
(a *statement* — each statement is its own concurrent branch, none of
them named). The named expression form already worked in progressive
syntax (it's parsed the same way any other `{ ... }` expression on an
assignment's right-hand side is — progressive syntax uses indentation for
*statement* blocks, not for expressions). The bare statement form now
does too, as a block header:

```
parallel:
  memory.a = compute_a()
  memory.b = compute_b()
```

### `gx build`

`--allow-shell`/`--allow-process`/`--allow-internal-http`/`--deny` are also
accepted by `gx build`, baked into the generated launcher's own `gx run`
invocation — a distributed binary's end user generally has no way to know
which flags the program needs, so the developer decides at build time
instead. If a `gx.json` sits next to the source file, `gx build` copies it
into `dist/` alongside the launcher, which `cd`s into its own directory
before running — so the manifest's allowlists still apply to the built
binary no matter where it's invoked from.

### Migrating from the old flags

Nothing to change for existing scripts: `--allow-shell`, `--allow-process`,
`--allow-internal-http`, `--no-sandbox`, and `dependencies.js`/
`dependencies.py`/`dependencies.process` behave exactly as before — they
now route through `Capabilities` internally, but the CLI surface and
`gx.json` schema are unchanged and fully backward compatible. To adopt the
new, stricter posture: add a `capabilities` section to `gx.json` for the
resources you want to restrict, and extend `dependencies.*` to the bridge
namespaces (`ts`/`binary`/`go`/`rust_bin`) or `ai` providers you want
restricted — both were previously ungated entirely.

### Production best practices

- Treat `--deny` as the operator's tool, not the developer's — use it in
  deployment configs to enforce a floor the script/manifest can't raise.
- If a script only needs specific AI providers, declare `dependencies.ai` —
  don't rely on the default-open behavior in a compliance-sensitive deployment.
- If a script never needs a listening server, set `capabilities.http_server:
  false` — closes an attack surface with a `serve` call the script never
  legitimately reaches.
- `env_deny` names exact variables, not patterns — list the specific secrets
  a script shouldn't be able to read (cloud credentials, database passwords)
  rather than trying to guess a pattern that covers all of them.

---

## Diagnostics & Observability

One runtime every subsystem — HTTP client/server, Database, Process,
Capability, and future ones — reports through, instead of each inventing
its own logging. Output is JSON Lines (JSONL) on stderr, deliberately
backend-agnostic: pipe it into `jq`, ship it to a log aggregator, or read it
raw during local development.

```json
{"ts_ms":1783588658960,"kind":"span","name":"db.exec","trace_id":"4fd0...","span_id":"1f8b...","parent_span_id":null,"duration_ms":3.1,"outcome":"ok","data":{"db":"orders.db"}}
```

### Two tiers

**Tier 1 — structured logging, always on.** `log_debug`/`log_info`/
`log_warn`/`log_error(message, data?)`, plus automatic `capability_denied`
audit events emitted whenever `Capabilities` denies something. Filtered by
`--log-level` (`debug`/`info`/`warn`/`error`, default `info`) or the
`GX_LOG_LEVEL` environment variable if the flag isn't passed. Audit events
are always emitted at `warn` regardless of the configured minimum, since a
denied capability should stay visible in production by default.

```gx
log_debug("cache miss", { key: cache_key })
log_info("order processed", { order_id: id, total: total })
log_warn("retrying after transient failure", { attempt: n })
log_error("payment failed", { order_id: id, reason: err })
```

**Tier 2 — spans and correlation IDs, opt-in via `--trace`.** A `trace_id`
identifies one logical operation (one top-level script run, or one incoming
HTTP request); nested `span_id`s time named sub-operations within it, with
a `parent_span_id` linking them back together. With `--trace` off, spans
degrade to a single boolean check and just run their body — no UUID
generation, no allocation, no stack push. This is deliberate: a script
that never opts in pays nothing for tracing beyond that one branch per span.

```bash
gx run app.gx --trace --log-level debug
```

### Manual instrumentation

```gx
span("checkout") {
  log_info("processing order", { order_id: id })
  result = charge_card(id)
}

id = trace_id()   // current trace id, or null if none is active
s  = span_id()    // current (innermost) span id, or null if none is active
```

`span(name) { ... }` behaves like `db_transaction(path) { ... }`: its body
shares the enclosing scope (variables it sets are visible after the block),
and it propagates whatever the body does — a thrown error still propagates
after the span is ended with an `error` outcome; a Rust panic inside the
body still ends the span (also as `error`) before the panic continues to
unwind, the same `catch_unwind`-based cleanup guarantee already used for
`db_transaction` rollback and SSE stream cleanup. Spans nest correctly to
any depth — `end_span` searches for its own id rather than assuming it's
always the top of the stack, so a leaked/never-ended inner span (a bug
elsewhere) can't corrupt an outer span's bookkeeping.

Progressive syntax: `span("checkout"):` as a block header, same as
`db_transaction(path):` above.

### Automatic spans

No extra code needed — these are wired into the runtime directly:

| Subsystem | Span(s) |
|---|---|
| HTTP client | `http.client.http_get` / `http_post` / `http_put` / `http_delete` (also covers `fetch`/`http_request`) |
| HTTP server | `http.server.request` — one root span per incoming request, on a fresh `trace_id` |
| Database | `db.query`, `db.exec`, `db.transaction` (including nested savepoints) |
| Process | `process.run`, `process.spawn`, `process.shell` |
| Multi-agent | `agent.spawn` (`spawn agent ... timeout`), `agent.parallel` (`parallel { ... }`) |

Each incoming HTTP request gets its own fresh `trace_id` — independent from
whatever trace the server process itself started under — since a request is
its own logical operation; this holds correctly across the HTTP server's
worker pool, where several requests run concurrently on different workers.
`spawn agent`/`parallel { ... }` do the opposite: the spawned agent
**inherits the parent's `trace_id`** (via `Diagnostics::for_child()`) so a
delegated sub-task still correlates back to the operation that triggered
it, and starts its own `agent.spawn`/`agent.parallel` span nested under
whatever span was active in the parent at spawn time.

### CLI flags

| Flag | Effect |
|---|---|
| *(default)* | Tier 1 logging active at `info`; tier 2 tracing off |
| `--trace` | Turn on tier 2 (spans, correlation IDs) |
| `--log-level <level>` | Set the tier 1 minimum level (`debug`/`info`/`warn`/`error`) |
| `GX_LOG_LEVEL` env var | Same as `--log-level`, used when the flag isn't passed |

### Production best practices

- Leave `--trace` off in normal production traffic unless you're actively
  debugging — tier 1 logging plus capability-denial audit events is usually
  enough, and tier 2 adds per-span UUID generation and JSON serialization
  overhead you don't need until you're chasing a specific problem.
- Correlate logs across a distributed deployment by forwarding `trace_id`
  in a request header and having the receiving service continue the same
  logical trace (not yet automatic — this runtime is deliberately designed
  so that a future distributed-tracing integration has a `trace_id`/
  `span_id`/`parent_span_id` shape ready to extend, without a breaking
  change to today's output format).
- Prefer `data` (the structured second argument to `log_*`/the attributes
  a span records) over string-interpolating values into the message — it's
  what makes the JSONL output queryable with `jq`/a log pipeline instead of
  needing regex.
- A capability-denied audit event is emitted at both the pre-check and
  resolved-address layers for HTTP (SSRF can be blocked by either), so
  don't assume one `capability_denied` event per blocked request — dedupe
  on `trace_id` if counting distinct blocked operations.

---

## Task Runtime

The language-level primitive for safe, observable, cancellable concurrent
work. `spawn agent ... timeout` and `parallel { ... }` are built on top of
it internally — this is what any GX program (agents, background workers,
web servers, pipelines) should use for its own concurrency too.

### Model

A task is a GX closure (`fn() { ... }`) run on its own OS thread, against
its own child `Interpreter` — the same "one Interpreter per unit of
concurrent work" shape already used by `spawn agent`, the HTTP server's
worker pool, and `parallel { ... }`. There is no async runtime underneath;
GX's tree-walking interpreter has no cooperative yield points inside
expression evaluation, so this is the execution model — no `async`/`await`
syntax to learn.

```gx
h = task_spawn(fn() {
  return process_run({ command: "some-job", args: [] })
})
result = task_wait(h)
if result.ok { say result.value.stdout }
```

### Primitives

| Builtin | Behavior |
|---|---|
| `task_spawn(fn, opts?)` | Runs `fn` (a zero-arg closure) on a new task. Returns a handle string immediately — non-blocking. |
| `task_wait(handle, timeout_ms?)` | Blocks until the task finishes, or `timeout_ms` elapses (default: waits forever). Returns a result object (below). |
| `task_wait_all([handles], timeout_ms?)` | Waits for every handle; returns an array of result objects in the same order. |
| `task_wait_any([handles], timeout_ms?)` | Returns the result of whichever task finishes first, plus `index` identifying which handle won. `null` if the timeout elapses first. |
| `task_cancel(handle, reason?)` | Flags the task cancelled. Returns `true` if it found a still-running task, `false` for an unknown or already-finished handle. |
| `task_status(handle)` | `{ status, done, cancelled, label, parent_id, started_at, duration_ms, task_id }`, or `null` for an unknown handle. `status` is `"running"`/`"done"`/`"cancelled"`/`"failed"`. |
| `task_id()` | The current task's handle, or `null` outside of one. |
| `is_cancelled()` | Whether the current task (or an ancestor of it) has been cancelled. `false` outside of a task. |
| `task_emit(value)` | Called from *inside* a running task to report incremental progress. Errors if not running inside a task. |
| `task_progress(handle)` | Drains every value `task_emit`'d since the last drain (or since the task started) — an empty array, not an error, when there's nothing new. |

Once a `task_wait`/`task_wait_all`/`task_wait_any` call actually observes a
task as finished, that task's handle is reaped — `task_status`/`task_cancel`
on it afterward behave exactly as if the handle had never existed (`null`/
`false`), the same convention `process_wait`/`process_status` already use.
This is what keeps a long-lived script (an HTTP worker spawning a task per
request, say) from accumulating finished tasks forever — read whatever you
need from the result object `task_wait` itself returns, since that's your
only chance to. A wait call that times out *without* the task actually
finishing does **not** reap it — `task_status` stays queryable while it
winds down after cancellation.

`opts` (both optional): `{ timeout: ms }` sets a deadline that cancels the
task automatically if it's still running once it passes. `{ pool: "name",
max_concurrent: N }` caps how many tasks sharing that pool name actually
run at once — see [Bounded parallel execution](#bounded-parallel-execution)
below.

`task_wait`'s result object:

```gx
{
  ok: bool,          // true only on a real successful completion
  value: any,        // the closure's return value, or null
  error: string?,    // the failure/cancellation reason, or null
  cancelled: bool,
  timed_out: bool,   // true only if THIS wait call's own timeout elapsed
  status: string,
  task_id: string,
  progress: array    // any task_emit values not yet drained via task_progress
}
```

### Reporting progress from a still-running task

`task_wait`'s result is only available once a task finishes — there was
previously no way for a still-running task to report incremental progress
back to whatever spawned it (each task runs against a completely separate
`Interpreter`, so there's no memory shared with the caller to poll). A task
calls `task_emit(value)` to push a progress update; the caller drains new
updates with `task_progress(handle)`:

```gx
h = task_spawn(fn() {
  i = 0
  while i < 5 {
    task_emit({ step: i + 1, total: 5 })
    sleep(0.1)
    i = i + 1
  }
  return "done"
})

while task_status(h).done == false {
  updates = task_progress(h)
  // ... show updates ...
  sleep(0.05)
}
// Anything emitted between the last poll and completion is still
// reachable — task_wait's own result carries it, so nothing at the tail
// end of a task's work is ever lost even if you stop polling early.
result = task_wait(h)
say result.progress
```

Capped at 1000 unconsumed entries per task (`task_emit` errors past that,
the same defense-in-depth shape as `MAX_CONCURRENT_TASKS`) — a task that
emits in a tight loop with nobody draining it fails loudly instead of
growing memory without bound.

### Cancellation is cooperative, not preemptive

There is no safe way to forcibly kill a running thread. `task_cancel`
(and a task's own `timeout` deadline, and `task_wait`'s own timeout — see
below) all just set a flag; the task's own execution notices it at the
next checkpoint:

- **Every statement**, automatically — no loop needs to check anything
  itself. A cancelled task raises internally and unwinds like any other
  error, running `db_transaction` rollbacks and `span(...)` cleanup on the
  way out.
- **`sleep()`** wakes up promptly instead of completing its full duration.
- **`process_run`/`process_spawn`** kill the child OS process the moment
  cancellation is noticed, rather than leaving it running.

A single very long `http_get`/`db_query` call with no timeout of its own
will still run to completion before a cancellation takes effect — the
same boundary every mainstream language's cancellation model has (Go's
`context`, Java's `Thread.interrupt`, .NET's `CancellationToken`). Give
it its own `timeout` if that matters for a specific call.

A task's own cancellation check is unreachable from `try`/`catch` — a
`catch { }` block can catch an error, but not silently absorb a
cancellation the task was relying on to stop. If you need to react to
cancellation deliberately (flush partial state, log something), poll
`is_cancelled()`.

### Structured concurrency — no orphans

```gx
parent = task_spawn(fn() {
  child = task_spawn(fn() { ... })
  return task_wait(child)
})
task_cancel(parent)   // cancels the child too
```

A task spawned from inside another task is linked to it: cancelling the
parent cancels every task nested under it. And because a task's body runs
against its own `Interpreter`, any further tasks *it* spawns live in that
Interpreter's own registry — cleaned up the moment that Interpreter's
thread function returns, for any reason (normal completion, error, panic,
or cancellation). A task can never outlive the Interpreter that owns it,
recursively — if a script spawns a task and never calls `task_wait` on it
at all, it's still cancelled and joined (within a bounded grace period)
when the owning Interpreter is dropped, exactly like a `process_spawn`
child process already was.

### Bounded parallel execution

```gx
handles = []
i = 0
while i < 100 {
  handles = handles + [task_spawn(fn() { return fetch_item(i) }, {
    pool: "fetchers", max_concurrent: 8
  })]
  i = i + 1
}
results = task_wait_all(handles)
```

`pool`/`max_concurrent` names a concurrency group, created on first use and
shared by every later `task_spawn` naming the same pool from that
Interpreter. Only `max_concurrent` tasks in the group actually run at
once; the rest wait for a slot (and still respond to cancellation while
waiting).

### Integration with every other runtime

| Runtime | What happens |
|---|---|
| Diagnostics | Every task gets its own span (named by `label`, default `"task"`), nested under whatever span was active when it was spawned, with a `cancelled` outcome distinct from `error`. |
| Capability | A task inherits the spawning script's capabilities — the same `spawn agent`/`parallel {}` already had. |
| Process | Cancelling a task kills any child process it started via `process_run`/`process_spawn`. |
| Database | Each task runs against its own `Interpreter`, so it has its own connection pool and transaction-nesting state — one task's `db_transaction` can never be corrupted by another's. |
| Multi-agent | `spawn agent ... timeout` / `parallel { ... }` are implemented on top of `task_spawn`/`task_wait` — no separate mechanism, and no more orphaned threads on timeout. |

### What this doesn't do (yet)

- No distributed execution — every task runs as a thread in this same OS
  process. The handle/status vocabulary (`task_id`, `parent_id`, `status`)
  is deliberately shaped so a future distributed variant could extend it
  without a breaking change.
- No async/await syntax — GX's execution model doesn't need one for this.
- No preemption of a single long blocking call with no timeout of its own
  (see "Cancellation is cooperative" above).

### Why there's no separate "event runtime"

A dedicated async/event layer was investigated and deliberately not
built, because the capabilities it would provide already exist:

| Capability | Already provided by |
|---|---|
| Timers | `sleep(seconds)` — cancellation-aware when running inside a task |
| Intervals | `while true { ...; sleep(n) }` inside a `task_spawn`'d closure — cancellable via `task_cancel` exactly like any other task (`sleep` polls for cancellation rather than blocking the full duration) |
| Scheduling | `when cron "*/5 * * * *" { ... }` inside an `agent`/`helper` |
| Event emitters/listeners, subscriptions | `emit`/`broadcast`/`send "event" to "agent"` and `when message "event" { }` inside an `agent`/`helper` |
| Signals (reactive triggers) | `when <expr> changes { }` |
| Progress/incremental results from background work | `task_emit`/`task_progress` (above) |

The one genuine gap found — a still-running task having no way to report
incremental progress before finishing — is what `task_emit`/
`task_progress` fill. Everything else was already there, just not
necessarily obvious without reading the interpreter directly.

---

## AI Context Runtime

The runtime responsible for managing AI conversation state — message
history, token budgeting, trimming, and prompt assembly — for every GX
application. It is deliberately *not* an AI framework: it does not decide
when to call a model, which tools to invoke, or how to summarize a
conversation. It provides the primitives every such policy is built from.

### A context is plain data

Unlike `process_spawn`/`task_spawn`, `context_create()` does not return an
opaque handle into a registry — it returns an ordinary GX object. A
context wraps no external resource (no OS process, no background thread)
that needs tracking; it's a system prompt, an array of messages, and some
token-budget configuration, and GX's existing value semantics already give
that everything a handle-based design would need extra machinery for:

- **Isolation**: a context passed into a `task_spawn`/`spawn agent`
  closure is already an independent snapshot — mutating it inside never
  affects the caller's copy.
- **Inheritance**: passing a context as a task/agent input argument already
  hands the child a full copy of the conversation so far — the same
  closure capture the Task Runtime already provides for any value.
- **Cloning**: `ctx2 = ctx` already deep-copies; `context_clone` (below)
  exists as an explicit, discoverable primitive that also stamps a fresh
  id and records the lineage, for branching a conversation into two
  continuations.
- **Serialization/persistence**: a context is already JSON-serializable —
  `context_serialize`/`context_deserialize` add version checking so a
  context saved before a future schema change fails loudly instead of
  loading silently-wrong. Persistence is just `db_exec`/`db_query` on the
  resulting string; there is no separate AI-context database API.

### Primitives

| Builtin | Behavior |
|---|---|
| `context_create(opts?)` | Creates a fresh context. `opts`: `system`, `max_history_tokens` (default 8000), `reserve_tokens` (default 1000, must be less than `max_history_tokens`), `max_messages` (default 200), `trim_strategy` (default `"drop_oldest_pair"`), `tool_output_max_chars` (default 8000). |
| `context_set_system(ctx, text)` | Returns an updated context with the system prompt set/replaced. |
| `context_add_message(ctx, role, content, opts?)` | Appends a message (`role`: `"user"`/`"assistant"`/`"tool"` — not `"system"`, use `context_set_system`). `opts`: `tool_call_id`/`name` (role `"tool"` requires `tool_call_id`), `tool_calls` (role `"assistant"` requesting tool use). Auto-trims afterward if configured. |
| `context_add_tool_result(ctx, tool_call_id, name, content)` | Sugar for `context_add_message(ctx, "tool", content, { tool_call_id, name })`, with `tool_output_max_chars` truncation applied. |
| `context_ask(ctx, provider, opts?)` | Sends the full context to `provider` (`"openai"`/`"anthropic"`). `opts`: `model`, `max_tokens` (response length, default 1024), `temperature`, `timeout`, `tools`. Returns the AI response fields (`ok`/`text`/`tokens_used`/`tool_calls`/`error`/`error_kind`/`retry_after_ms`) plus `.context` — the updated context with the assistant's reply already appended (only on success). |
| `context_trim(ctx, opts?)` | Forces a trim pass now if the context is over budget. `opts.strategy` overrides `ctx`'s own `trim_strategy` for this call. |
| `context_summarize_and_trim(ctx, summary_text, opts?)` | Replaces the oldest messages with one summary message. `opts.keep_last` (default 0) keeps that many of the most recent messages verbatim after the summary. See "Automatic summarization" below. |
| `context_clone(ctx)` | An independent copy with a new context id, recording `forked_from` in stats. |
| `context_reset(ctx, opts?)` | A fresh context (new id, empty history, reset stats) preserving configuration. `opts.keep_system` (default `true`) controls whether the system prompt survives the reset. |
| `context_serialize(ctx)` / `context_deserialize(json)` | Versioned JSON round-trip; `context_deserialize` rejects an incompatible version or invalid JSON. |
| `context_stats(ctx)` | `{ context_id, message_count, estimated_tokens, system_present, max_history_tokens, over_budget, trim_strategy, ask_count, total_tokens_used, total_messages_added, total_messages_trimmed, created_at, last_ask_at }`. |

`ctx` in every builtin above must come from `context_create`/
`context_deserialize`/another context builtin — an arbitrary object is
rejected with a clear error rather than silently misread.

### Conversations

```gx
ctx = context_create({ system: "You are a helpful assistant.", max_history_tokens: 6000 })
ctx = context_add_message(ctx, "user", "What's the capital of France?")

result = context_ask(ctx, "openai", { model: "gpt-4o-mini" })
if result.ok {
  say result.text
  ctx = result.context   // assistant's reply already appended
} else {
  say "failed: {result.error_kind} — {result.error}"
}
```

### Tool calls

```gx
result = context_ask(ctx, "openai", { model: "gpt-4o-mini", tools: my_tool_schemas })
ctx = result.context
if result.tool_calls {
  for each call in result.tool_calls {
    output = run_my_tool(call.name, call.arguments)   // application logic — GX doesn't decide this
    ctx = context_add_tool_result(ctx, call.id, call.name, output)
  }
  result2 = context_ask(ctx, "openai", { model: "gpt-4o-mini", tools: my_tool_schemas })
  ctx = result2.context
}
```

The provider-neutral message list (`role`/`content`/`tool_call_id`/`name`/
`tool_calls`) is translated to each provider's actual wire format inside
the runtime — OpenAI's `tool_calls` field vs. Anthropic's `tool_use`/
`tool_result` content blocks, Anthropic's top-level `system` field vs.
OpenAI's system-role message — so a script never needs to know which
provider it's talking to when assembling a conversation.

### Automatic trimming

Every `context_add_message` call (and `context_ask`'s own append of the
assistant's reply) checks the context against its budget afterward and
trims automatically unless `trim_strategy` is `"none"`:

| Strategy | Behavior |
|---|---|
| `"none"` | Never auto-trims. `context_stats(ctx).over_budget` still reports honestly; call `context_trim` explicitly when you want to act on it. |
| `"drop_oldest"` | Removes the single oldest message, repeated until back under budget. |
| `"drop_oldest_pair"` (default) | Removes the two oldest messages at a time — usually keeps `user`/`assistant` turns paired rather than leaving a dangling reply with no prompt. A heuristic, not a dependency-aware conversation-graph trimmer. |
| `"summarize"` | Behaves exactly like `"drop_oldest_pair"` automatically — see below for why. |

Two independent triggers, either one starts a trim pass: the estimated
token count (`system` + every message's cached per-message estimate, the
same ~4-chars-per-token heuristic as `token_count()`) exceeding
`max_history_tokens - reserve_tokens`, or the message count exceeding
`max_messages` — a hard ceiling that exists specifically because the token
estimate is a heuristic, not an exact tokenizer.

**Automatic summarization.** `trim_strategy: "summarize"` does not call a
model itself — this runtime doesn't decide when to spend tokens/money on
summarization, or which model/prompt to use for it, on an application's
behalf. What it provides is the actual hook: `context_summarize_and_trim`,
the mechanical "replace these old messages with one summary message"
operation. The typical pattern:

```gx
if context_stats(ctx).over_budget {
  old_messages = ctx.messages   // read what's about to be summarized
  summary = context_ask(context_reset(ctx), "openai", {
    model: "gpt-4o-mini",
  }).text   // your own summarization call — your model, your prompt
  ctx = context_summarize_and_trim(ctx, summary, { keep_last: 4 })
}
```

### Persistence

```gx
db_exec(db_path, "CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY, data TEXT)")
db_exec(db_path, "INSERT OR REPLACE INTO conversations(id, data) VALUES (?, ?)", [conv_id, context_serialize(ctx)])

// later:
row = db_query(db_path, "SELECT data FROM conversations WHERE id = ?", [conv_id])[0]
ctx = context_deserialize(row.data)
```

### Integration with every other runtime

| Runtime | What happens |
|---|---|
| Diagnostics | `context_ask` (and the single-shot `ask`/`Think`/`embed`/`infer`) get an automatic `ai.request`/`ai.embed`/`ai.infer` span with `provider`/`model`/`context_id`/`message_count`/`tokens_used` attributes and an `error_kind`-aware outcome. |
| Capability | `context_ask` authorizes through the same `Resource::AiProviders` check (`dependencies.ai` in `gx.json`) as every other AI primitive. |
| HTTP | Every provider — `openai`/`anthropic`/`ollama` alike, single-shot `ask` or `context_ask` — reuses a capability-checked, connection-pooled `ureq::Agent`, with a longer default read timeout (120s vs. 30s) appropriate for a real completion, and honors a per-call `timeout` (seconds) override. `ollama` gets its own dedicated agent with `internal_network` force-granted rather than the general one: its whole purpose is talking to a loopback model server, so requiring `--allow-internal-http` (a flag for arbitrary internal-network HTTP access) just to use a local model would break the single most common Ollama workflow. The SSRF resolver still runs on that agent — it's not the same as the old zero-checks connection this replaced. |
| Task | A context is ordinary data — see "A context is plain data" above; no special propagation code was needed. |
| Database | Persistence is `context_serialize`/`context_deserialize` plus ordinary `db_exec`/`db_query` — no dedicated API. |
| Process | A tool's output (often a `process_run(...).stdout`) flows into `context_add_tool_result` like any other string — no special integration code needed. |

### Retry and rate-limit hooks

Every AI response — `context_ask` and the single-shot primitives alike —
carries a structured `error_kind` (`rate_limited`/`auth_error`/
`invalid_request`/`server_error`/`timeout`/`network_error`/`http_error`)
classified from the provider's HTTP status, and `retry_after_ms` when the
provider sent a `Retry-After` header. This is the actual "retry hook" —
not a new retry engine, since GX already has one:

```gx
result = retry(fn() {
  r = context_ask(ctx, "openai", {})
  if r.ok { return r }
  if r.error_kind == "rate_limited" or r.error_kind == "server_error" {
    if r.retry_after_ms { sleep(r.retry_after_ms / 1000) }
    x = 1 / 0   // force retry() to treat this attempt as failed
  }
  return r   // invalid_request/auth_error: retrying won't help, stop
}, 5, { delay: 1000, backoff: "exponential" })
```

### Provider neutrality

No `openai`-, `anthropic`-, `ollama`-, or application-specific behavior
exists in this runtime — every provider-specific translation (message
shape, tool-call format, system-prompt placement) lives inside the AI
Runtime's connectors, never in the Context Runtime or exposed to scripts.
`ollama`'s multi-turn `/api/chat` endpoint is wired into `context_ask` the
same way `openai`/`anthropic` are — `context_ask(ctx, "ollama", {})` sends
the full conversation and appends the reply, the same as any other
provider.

### What this doesn't do (yet)

- No streaming support in `context_ask` (the single-shot `ask ... {
  stream: true }` is unaffected). Nothing in the context object's design
  precludes adding it later.
- No multimodal (image/audio) message content — `content` is a string.
- No distributed/shared conversation state across processes — persistence
  is explicit (`context_serialize` + your own storage), not automatic
  replication.
- No automatic tool-execution loop — `context_ask` returns `tool_calls`;
  deciding which tool to run and actually running it is application logic,
  same as it already was for the single-shot `ask` primitive.

---

## Module & Package Runtime

Production-grade module resolution and dependency management for
multi-file GX projects: deterministic `import` resolution, package
metadata, semantic versioning, dependency locking (`gx.lock`), package
integrity verification, and a local package cache. `gx.json`'s
`name`/`version`/`entry`/`dependencies.gx` fields existed since `gx init`
first scaffolded them but were previously entirely inert — this runtime is
what reads them.

### File imports

```gx
import "./lib/utils.gx"          // relative to this file's own directory
import "./lib/utils.gx" as utils // namespaced — utils.func_name()
```

- **Resolved relative to the importing file's own directory first**, a
  current-working-directory-relative (or absolute) path only as a
  fallback. This is deterministic: the same script resolves the same way
  regardless of which directory `gx` was invoked from.
- **Transitive**: an imported file's own `import`s are followed too, at
  any depth. `a.gx` importing `b.gx`, which itself imports `c.gx`, loads
  all three.
- **Each file is parsed at most once**, no matter how many places import
  it (a "diamond" — two files that both import a common third file — is
  detected and merged from cache, not re-parsed or falsely flagged as a
  collision).
- **Import cycles are detected**, reported with the full chain
  (`a.gx -> b.gx -> a.gx`) rather than overflowing the stack.
- A flat (unaliased) import whose function/agent name collides with an
  existing definition from a *different* file follows the previous
  last-import-wins behavior for compatibility, but now logs a diagnostics
  warning (`Level::Warn`) instead of silently shadowing it.

### Package imports

```gx
import "leftpad"   // a bare name — no .gx suffix, no path separator, no leading "."
```

A bare name (as opposed to `"./leftpad.gx"` or `"leftpad.gx"`) is resolved
as a **package**: looked up in `gx.lock`, then read from wherever it was
cached at `gx install` time, entering at its own `gx.json`'s `entry` field
(default `main.gx`). Resolving the same package from several files in one
run only reads `gx.lock` and re-verifies its integrity hash once, not once
per importer.

### `gx.json` dependencies

```json
{
  "name": "my-app",
  "version": "0.1.0",
  "entry": "main.gx",
  "dependencies": {
    "js": [],
    "py": [],
    "gx": {
      "leftpad": "^1.2.0",
      "some-lib": { "git": "https://github.com/example/some-lib", "rev": "v2.0.0" },
      "shared": { "path": "../shared" }
    }
  }
}
```

Three dependency source kinds:

| Source | Shape | Resolves via |
|---|---|---|
| Registry (semver range) | `"^1.2.0"` | The local cache only — the highest cached version satisfying the range. Nothing is ever fetched implicitly; `gx install` is what populates the cache. There is no hosted GX package registry (see below), so this errors clearly if nothing cached satisfies the range, naming a `git`/`path` dependency as the alternative. |
| Git | `{ "git": "<url>", "rev": "<ref>"? }` | `gx install` clones it (shells out to `git`, `--quiet`), checks out `rev` if given, strips `.git` metadata before hashing, and caches it under the version its own `gx.json` declares (or `0.0.0+git.<short-sha>` if it declares none). |
| Path | `{ "path": "<relative path>" }` | Used directly at that location, relative to the *dependent's* `gx.json` directory — never cached, never integrity-checked (a path dependency is meant to always reflect its current on-disk state, the entire point of using one during local/monorepo development). |

### `gx install` / `gx publish`

```bash
gx install <js.pkg|py.pkg>        # add/update a single js/py bridge package (unchanged)
gx install                        # no args: resolve gx.json's dependencies.gx, write gx.lock
gx install --offline              # fail clearly instead of touching the network
GX_OFFLINE=1 gx install            # same, via environment variable
gx publish                        # validate + hash this package, write <name>-<version>.gxpkg.json
```

`gx install` (no arguments) resolves every entry in `dependencies.gx`
against its source, computes a SHA-256 integrity hash over the resolved
package's file tree, and writes/updates `gx.lock`. `--offline` (or
`GX_OFFLINE`) makes any dependency that isn't already cached a hard error
instead of reaching the network.

`gx publish` does not upload anything — **there is no hosted GX package
registry**, deliberately: GX has no server infrastructure to run one, and
building a fake one would be exactly the kind of feature added "because
other languages have it." Git-based distribution (clone a tagged
repository) is a real, complete, offline-capable-after-first-fetch source
that needs no new infrastructure — the same bootstrap path early Cargo
used before crates.io existed. `gx publish` validates the package has a
name and a valid semver version, rejects publishing with a `path`
dependency still in place (meaningless outside this checkout), computes
the same integrity hash `gx install` will later verify against, writes a
`<name>-<version>.gxpkg.json` descriptor, and prints the actual git-tag
workflow for distributing it.

### `gx.lock`

```json
{
  "version": 1,
  "packages": {
    "leftpad": {
      "version": "1.2.0",
      "resolved": "git+https://github.com/example/gx-leftpad#a1b2c3d",
      "integrity": "sha256-…"
    }
  }
}
```

Versioned (an incompatible future lockfile format fails loudly rather than
being silently misread) and key-sorted for clean diffs. Every resolution
that finds a `gx.lock` (a `git` or registry dependency; path dependencies
are exempt, see above) re-verifies the cached package's hash against it
before using it — a tampered or corrupted local cache entry is rejected,
not silently used.

### Local package cache

Cached packages live at `~/.gx/packages/<name>/<version>/`, overridable
with `GX_PACKAGE_CACHE_DIR` (mirrors the existing `GX_STATE_DIR`
convention for persistent memory). Package names/versions are sanitized
before being used to build filesystem paths, and a symlink anywhere inside
a package's tree is rejected outright when computing its integrity hash —
an untrusted git/registry dependency could otherwise plant one pointing
outside the package (e.g. a `main.gx` symlinked to `~/.ssh/id_rsa`) that
would later be read as the package's own source the moment something
imports it.

### Capability scope (deliberate boundary)

Imported/dependency code — a plain file import, a namespaced module
import, or a package import — runs with **exactly the same
[Capability Runtime](#capability-runtime) grant as the script that
imported it**. There is no per-package capability sandboxing: a
dependency's functions execute in the same interpreter, sharing its one
capability set, the same way an ordinary function call does. This is a
deliberate scope boundary for this milestone, not an oversight — isolating
each dependency in its own capability sandbox is a real, valuable, but
substantially larger feature (per-module capability declarations, grant
attenuation across call boundaries, enforcement at every call site rather
than once at process start), left as a future consideration.

### Diagnostics

Two spans, active under `--trace` like every other diagnostics span:
`module.import` (wraps the whole import-resolution pass for a program run,
with a `files_resolved` count) and `package.resolve` (one per package
import, with `package`/`version`/`source` attributes — see
[Diagnostics & Observability](#diagnostics--observability)).

---

## Error Handling

GX has two coexisting failure-signaling conventions, because different
runtime subsystems were built at different times against different
precedents. Neither is "correct" — this section exists so the difference
is something you look up once, not something you discover by watching a
`try/catch` silently never fire.

### The two conventions

**Throwing** — the operation raises an error that propagates up until a
`try/catch` (or the top level) catches it. Used by: `db_query`/`db_exec`/
`db_transaction`, file I/O (`read_file`/`write_file`/`delete_file`/...),
`readline`, `assert`, and most builtins that fail only on a genuine
programmer/environment error (a missing argument, an unreadable path).

```gx
try {
  rows = db_query(path, "SELECT * FROM users WHERE id = ?", id)
} catch e {
  log("query failed: " + e.message)
}
```

**Returning a result object** — the operation always returns normally;
success or failure is a field on the returned value, `{ ok: true, ... }`
or `{ ok: false, error, error_kind, ... }`. Used by `http_get`/`http_post`/
`http_put`/`http_delete`/`http_request`, `process_run`/`process_spawn`/
`process_wait`, `task_wait`/`task_wait_all`/`task_wait_any`, `ask`, and
`context_ask` — every one of these treats a timeout, a non-2xx status, a
non-zero exit, or a provider error as an *expected, operational* outcome,
not a programmer error, so it never throws for one.

```gx
result = http_get(url)
if !result.ok {
  log("request failed: " + result.error)
} else {
  data = result.data
}
```

**Why this matters**: code written assuming the wrong convention doesn't
fail loudly — it fails *silently*. A `try/catch` wrapped around `http_get`
never fires, because `http_get` doesn't throw; an `if !result.ok` check
after `db_query` never runs, because a failed query throws before
producing any result to check. Both look reasonable and both pass review;
neither does what its author expected.

### `unwrap()` — bridging the two conventions

```gx
try {
  data = unwrap(http_get(url))     // throws if result.ok is false
  rows = unwrap(db_query(path, sql)) // db_query already throws — unaffected
} catch e {
  log("failed: " + e.message + " (" + e.kind + ")")
}
```

`unwrap(result)` normalizes a `{ ok: false, error, error_kind, ... }`
result into a thrown error (catchable exactly like `db_query`'s own
failures — `e.message`/`e.kind` work the same way). Anything else —
`{ ok: true, ... }`, or a value with no `ok` field at all — passes through
completely unchanged; `unwrap` never guesses which field holds "the real
payload," since that differs per builtin (`.data` for `http_*`, `.value`
for `task_wait`, `.text` for `ask`/`context_ask`, the rows array itself
for `db_query`). Use it when you want one idiom (throw-and-catch)
regardless of which convention the wrapped call actually uses; ignore it
entirely if you're happy checking `.ok` — nothing about the returning
convention changes.

`retry(fn, max?, opts?)` already understands both conventions on its own:
it retries a closure that throws *or* one that returns `{ ok: false, ...
}`, and returns the final attempt's outcome unchanged once attempts are
exhausted (a thrown error stays thrown; a returned `{ ok: false, ... }`
stays a returned value) — so `retry(fn() { return http_get(url) }, 3)`
retries on a failed request, not just on a thrown error.

### `error_kind` vocabulary reference

The two conventions also use different vocabularies for classifying
*what kind* of failure occurred — this is the other half of "don't assume
one convention where the other applies." `try/catch`'s inferred `kind` is
PascalCase and guessed from the error message's text (see the caveat
below); a result object's `error_kind` field is a specific, deliberately
chosen snake_case string with no PascalCase equivalent.

| Convention | Kind values |
|---|---|
| `try/catch`'s `e.kind` (`infer_error_kind`) | `JsonParseError`, `NetworkError`, `PermissionError`, `NotFoundError`, `AssertionError`, `RuntimeError` |
| `http_*`'s `error_kind` | `http_status`, `blocked` (SSRF-denied), `timeout`, `dns_error`, `connection_failed`, `too_many_redirects`, `io_error`, `transport_error` |
| `process_*`'s `error_kind` | `not_found`, `permission_denied`, `spawn_failed`, `timeout`, `killed`, `unresponsive` |
| `ask`/`context_ask`'s `error_kind` | `rate_limited`, `auth_error`, `invalid_request`, `timeout`, `server_error`, `http_error`, `network_error`, `unknown` |

**Caveat**: `try/catch`'s `e.kind` is inferred by checking whether the
error *message* contains certain substrings (`"timeout"` → `NetworkError`,
`"not found"` → `NotFoundError`, ...) — it is not a structured
classification, and it can misfire on a message that happens to contain a
matching word for an unrelated reason (e.g. a request to a hostname
literally named `*.invalid` can surface as `JsonParseError` purely because
the message contains the substring `"invalid"`). Don't rely on `catch
<Kind>` where getting the classification exactly right matters — check
`e.message` directly, or use `unwrap()` on a result-object builtin and
inspect the original `error_kind` string yourself before it gets
re-classified.

### Checking a capability before you use it

```gx
if has_capability("external_network") {
  data = http_get(url)
} else {
  data = read_file("cached_response.json")
}

if has_capability("ai", "openai") {
  result = ask openai { prompt: "..." }
}
```

`has_capability(resource, name?)` answers "would this be allowed?" without
attempting the operation — no side effect, no denial thrown, no audit-log
entry. Before this existed, the only way to learn whether a capability was
granted was to attempt the operation and catch a `capability_denied`
error — workable, but it means a script that wants to *choose* a strategy
up front (use the network if allowed, otherwise fall back to a cache) has
to structure that as error handling instead of a plain `if`. `resource` is
one of the same names used in `gx.json`/`--deny`/error messages: `shell`,
`process`, `filesystem`, `internal_network`, `external_network`,
`http_server`, `database`, `environment`, `ai`, `js`, `ts`, `py`, `binary`,
`go`, `rust_bin` (see [Capability Runtime](#capability-runtime)). `name`
narrows the check against a resource's allowlist (an AI provider, a bridge
module, a process executable) — omit it to check only the resource-level
grant.

---

## Debugger Runtime

Built directly on `run_stmt` — the same per-statement checkpoint the Task
Runtime already uses for cooperative cancellation — so pausing execution
needed no redesign of the tree-walking interpreter: it's one more check
before a statement runs, exactly like the existing cancellation check.

### `breakpoint()` — pause from inside a script

```gx
x = compute_something()
breakpoint()
y = x + 1
```

Call it anywhere, in any execution context (`gx run`, `gx debug`, `gx -e`,
`gx test`, even from inside the REPL) — no flag or prior debug session
required. Drops into an interactive `(gx-debug)` prompt right there.

### `gx debug` / `gx run --break`

```bash
gx debug <file.gx> [--break line1,line2,...]
gx run <file.gx> --break line1,line2,...
```

`gx debug` is `gx run` with the debugger available — same flags, same
execution path, just named for discoverability. `--break` sets external,
source-unmodified line-number breakpoints (works on either command).

### The `(gx-debug)` prompt

| Command | Effect |
|---|---|
| `c`, `continue` | Resume execution |
| `s`, `step` | Run the next statement, then pause again (single-stepping) |
| `l`, `locals` | List every variable in the current scope |
| `bt`, `stack` | Show the current call stack |
| `p`, `print <expr>` | Evaluate and print a real GX expression (method calls, field access, interpolation — anything) |
| `w`, `watch <expr>` | Re-evaluate `<expr>` and print it at every future pause |
| `q`, `quit` | Stop execution |
| `h`, `help` | List these commands |

```
$ gx debug order.gx --break 12
[breakpoint] paused at line 12
(gx-debug) locals
  item = { name: "widget", price: 9.99, qty: 3 }
  total = 0
(gx-debug) watch total
watch added: total
(gx-debug) step
[breakpoint] paused at line 13
watch: total = 29.97
(gx-debug) continue
```

Every pause also emits a `debugger.pause` diagnostics event (visible with
`--trace`), so a paused breakpoint shows up in the same structured output
as every other subsystem's tracing.

**Concurrency note**: `debug_pause` blocks on stdin exactly like the
pre-existing `readline()` builtin — a breakpoint hit inside a spawned
task or a `parallel {}` branch pauses only that thread (each has its own
`Interpreter`/call stack), never the whole process.

---

## Testing Framework

`gx test` already ran whole files and reported pass/fail per *file*. This
adds named, isolated test cases within a file, setup/teardown, golden-file
comparison, deterministic randomness, and a sandbox-safe scratch
directory — composing existing primitives (the same `assert`, the same
capability-gated file I/O) rather than introducing a parallel system.

### `test(name, fn)`

Registers a named test case — deferred, not run immediately. `gx test`
runs every registered case separately, after the top-level script
finishes, each with its own fresh assertion count and failure list, so
one test's failure is reported under its own name and doesn't abort the
others the way a bare top-level `assert` would:

```gx
test("addition works", fn() {
  assert 1 + 1 == 2 "math is broken"
})

test("handles the empty case", fn() {
  assert len([]) == 0
})
```

```
$ gx test
  PASS   tests/test_math.gx
  PASS   tests/test_math.gx :: addition works (1 assertions)
  PASS   tests/test_math.gx :: handles the empty case (1 assertions)

Results: 3 passed, 0 failed, 0 errors | 2 total assertions
```

### `before_each(fn)` / `after_each(fn)`

Setup/teardown run around every `test()` case in the file — a single
active hook each (a later call replaces an earlier one). GX closures
capture their outer scope **by value**, so a plain variable
`before_each` mutates is invisible to the test body's own, separately
captured snapshot of it. `memory.*` is this language's existing channel
for state that needs to survive across separate closure calls (every
agent already relies on exactly this) — `gx test` runs `before_each`, the
test body, and `after_each` for one test case against the *same*
underlying scope, so `memory.*` is how setup hands state to the test:

```gx
before_each(fn() {
  memory.db = test_temp_dir() + "/test.db"
})

after_each(fn() {
  log("cleaning up {memory.db}")
})

test("insert then query", fn() {
  db_exec(memory.db, "CREATE TABLE t (v INTEGER)", [])
  db_exec(memory.db, "INSERT INTO t VALUES (?)", [1])
  rows = db_query(memory.db, "SELECT * FROM t", [])
  assert len(rows) == 1
})
```

Teardown always gets a chance to run, even if `before_each` or the test
body itself failed — the same "cleanup runs regardless" shape a `finally`
block has.

### `set_random_seed(n)`

Makes `random`/`random_int`/`random_choice`/`shuffle` fully deterministic
for the rest of the run — the same seed always produces the same sequence
of draws:

```gx
set_random_seed(42)
a = random_int(1, 100)
set_random_seed(42)
b = random_int(1, 100)
assert a == b   // always true
```

Scripts that never call `set_random_seed` see exactly the prior,
clock-seeded behavior — this is purely additive.

### `test_temp_dir()`

Returns a fresh, writable scratch directory on every call — resolved
through the same capability-gated path resolution as every other file
builtin (sandboxed under the script's own directory when `gx run`
sandboxing is active; relative to cwd under `gx test`'s unrestricted
default, matching a script's own `write_file("foo.txt", ...)` calls
either way):

```gx
test("round-trips through a file", fn() {
  dir = test_temp_dir()
  write_file(dir + "/out.txt", "hello")
  assert read_file(dir + "/out.txt") == "hello"
})
```

### `assert_golden(actual, path)`

Byte-for-byte comparison against a saved "golden" file:

```gx
test("renders the expected summary", fn() {
  summary = { status: "ok", count: 3 }
  assert_golden(summary, "tests/golden/summary.json")
})
```

A `Value::Str` is compared as-is (the natural shape for golden *text*
output — a rendered template, an HTTP body); anything else is serialized
as pretty-printed JSON with sorted keys (deterministic regardless of GX
object's own unordered internal representation). No golden file yet, or
`GX_UPDATE_GOLDEN=1` set: writes `actual` as the new golden and passes —
so a fresh golden test doesn't need two separate runs (one to create the
file, one to verify it) before it can go green:

```bash
GX_UPDATE_GOLDEN=1 gx test   # (re)write every golden file to match current output
```

---

## Configuration Runtime

A GX app previously had to manually chain `json_parse`/`yaml_parse`/
`toml_parse` + `env()` + `schema_validate` with no single ergonomic entry
point. `config_load` organizes those existing primitives — it doesn't
duplicate them.

### `config_load(options)`

Layered merge, later layers win:

```
defaults  <  config file  <  environment overrides  <  explicit overrides
```

```gx
config = config_load({
  defaults: { port: 3000, host: "localhost", debug: false },
  file: "config.json",       // auto-detected: .json/.yaml/.yml/.toml
  env_prefix: "APP_",        // APP_PORT overrides port, type-coerced
  overrides: { debug: true }, // highest precedence
  schema: { port: "number", host: "string", debug: "boolean" },
})
```

Every layer is independently optional — `config_load({ defaults: {...} })`
is a valid, if trivial, call.

- **`defaults`** — the base layer.
- **`file`** — path to a config file. A *missing* file is not an error
  (defaults carry the app); a file that exists but fails to parse, or
  whose extension isn't `.json`/`.yaml`/`.yml`/`.toml`, is.
- **`env_prefix`** — enables the environment-override layer. For each key
  already present after `defaults`+`file`, checks
  `{env_prefix}{KEY_UPPERCASED}` and, if set, overrides that key —
  coerced to match the *existing* value's type (env vars are always
  strings; a numeric/boolean default makes `"8080"`/`"true"` usable
  without a separate parse step). **Security property**: this can only
  ever override a key the app already declared via `defaults`/`file` — it
  can never inject a brand-new config key purely from an environment
  variable. A key denied by `gx.json`'s `capabilities.env_deny` throws
  (the same as a direct `env()` call would), audited the same way every
  other capability denial is.
- **`overrides`** — highest precedence, applied last.
- **`schema`** — if given, the *final* merged config is run through the
  existing `schema_validate`, and `config_load` throws on failure (fail
  fast on bad config) rather than returning an invalid object the caller
  might forget to check.

### Secrets stay separate

`config_load` is for *non-secret* settings. Secrets belong in
`.env`/`load_env()` + `env()` — already capability-gated, and never part
of what `config_load` returns:

```gx
config = config_load({ defaults: { port: 3000 }, file: "config.json" })
load_env(".env")
api_key = env("API_KEY", "")
```

Before logging or returning a config object, shape it with the existing
`pick()`/`omit()` builtins the same way you would any other API response
— `omit(config, ["internal_debug_token"])` — rather than assuming nothing
sensitive ever ends up in a config file.

Workspace/monorepo configuration (linking multiple packages' configs
together) is intentionally out of scope: GX has no monorepo/workspace
concept to hang that on today, and inventing one speculatively isn't a
gap-fill.

---

## Serialization Runtime

JSON, YAML, TOML, CSV, and XML support already existed
(`json_parse`/`json_stringify`, `yaml_parse`/`yaml_stringify`,
`toml_parse`/`toml_stringify`, `csv_parse`/`csv_stringify`,
`xml_parse`/`xml_stringify`) and are already deterministic — every one of
them sorts object keys before serializing, so the same value always
produces byte-identical output regardless of GX's own `Object` type being
internally unordered. This section covers what was genuinely missing:
JSON Lines, versioned serialization, and format-agnostic file I/O.

### JSON Lines (NDJSON)

Distinct from `json_parse`/`json_stringify`, which expect the whole text
to be exactly one JSON value: JSON Lines is one independent JSON value per
line — the standard shape for log streams and data-pipeline exports.

```gx
jsonl_stringify([{ id: 1 }, { id: 2 }])   // '{"id":1}\n{"id":2}\n'
jsonl_parse(text)                          // → array<value>, one per non-empty line
```

A malformed line is a parse error naming which line failed
(`jsonl_parse: line 2: ...`); a blank line is skipped.

### Versioned serialization

Generalizes the AI Context Runtime's own `context_serialize`/
`context_deserialize` pattern — a version tag checked on load, rejecting a
stale/foreign blob loudly rather than silently deserializing it into a
wrong shape — into a primitive any GX app can use for its own persisted
data:

```gx
saved = versioned_stringify({ name: "Ada" }, 2)
// '{"__gx_version":2,"data":{"name":"Ada"}}'

restored = versioned_parse(saved, 2)   // → { name: "Ada" }
versioned_parse(saved, 3)              // throws: unsupported version 2 (expected 3)
versioned_parse(saved)                 // no expected version: returns the data unconditionally
```

### `data_import(path)` / `data_export(path, value, schema?)`

Format-agnostic "read+parse"/"stringify+write", format detected from the
path's extension (`.json`/`.yaml`/`.yml`/`.toml`/`.csv`/`.xml`/`.jsonl`) —
composes every existing parser/stringifier rather than adding new parsing
logic:

```gx
data_export("report.yaml", { status: "ok", count: 3 })
report = data_import("report.yaml")

// Optional schema validation before writing — fails fast, writes nothing
// on a validation failure, using the existing schema_validate.
data_export("config.json", cfg, { port: "number" })
```

A missing extension `data_import` doesn't recognize is an error naming
every extension it does; a *missing file* is a plain I/O error (unlike
`config_load`'s `file` option, which deliberately tolerates a missing
config file — `data_import`/`data_export` are a direct file-access
primitive, not a layered-defaults system).

### What was already solved, and what wasn't built

- **Deterministic serialization** — already true for every existing
  format (see above); no new API needed.
- **Cross-format conversion** — already trivially composable
  (`toml_stringify(yaml_parse(text))` converts YAML to TOML in one line);
  a dedicated `convert_format()` wrapper would just be two existing calls
  with extra ceremony.
- **Binary serialization** (MessagePack, CBOR, Protobuf, ...) — not
  justified: GX has no existing binary wire-format need (no gRPC, no
  custom binary protocol anywhere in the language), and adding one would
  be a new dependency in search of a use case.
- **Custom serializers** — not needed for a dynamically-typed language:
  a script can already transform its own data with a plain function call
  before `json_stringify`/`data_export`, which is what a custom serializer
  would exist to do in a statically-typed language with real `Serialize`
  trait impls.

---

## Template & Code Generation Runtime

GX already has a full programming language for generating text: string
interpolation (`"{expr}"`), `while`/`for each`, and `write_file` let a
script build up and emit arbitrary text or source code. That covers
"generate text from code I'm writing right now." What it couldn't do is
the other common shape: render an *external* template — loaded from a
file, written once, reused many times — against a data object only
available at runtime, because `"{expr}"` interpolation resolves against
variables in scope at the exact point the string literal appears in the
source, not against an arbitrary value passed into a function.

### `render_template(template, data)`

```gx
tmpl = read_file("greeting.template")   // "Hello, {{name}}! You are {{age}}."
render_template(tmpl, { name: "Ada", age: 36 })
// → "Hello, Ada! You are 36."
```

`{{dotted.path}}` substitution against `data` — each segment is looked up
as an object field first, or as an array index if the current value is an
array and the segment parses as a number (`{{items.0.name}}`). A path
that doesn't resolve is a rendering error, not a silently-blanked
placeholder — `class {{name}} {` rendering to `class  {` on a typo is a
worse failure mode than refusing to render at all. Literal `{{`/`}}` in
the output (e.g. generating a file that itself contains template syntax)
are written as `\{{`/`\}}`.

**Important**: write templates to a *file* (or otherwise construct the
string at runtime), not as a GX string literal in source. GX's own
`"{expr}"` interpolation runs at parse time and will consume `{{name}}`
inside a literal before `render_template` ever sees it
(`"{{name}}"` in source becomes the already-mangled string `"{name}"`).
A template loaded via `read_file` is a plain runtime string GX's
interpolation never touches, which is also the realistic shape for a
reusable template anyway.

Deliberately not a web template engine: no HTML auto-escaping (this is
for source files, config files, and docs, not rendering untrusted values
into an HTML response), and no expression evaluation or control-flow
syntax (no `{{#if}}`/`{{#each}}` mini-language) inside `{{ }}` — a
repeated block is just an ordinary GX loop calling `render_template` once
per item:

```gx
names = ["Button", "Header", "Footer"]
tmpl = read_file("component.template")
i = 0
while i < len(names) {
  write_file(names[i] + ".jsx", render_template(tmpl, { name: names[i] }))
  i = i + 1
}
```

This is also the full "project scaffolding" story — `render_template` +
an ordinary loop + `write_file`/`make_dir`. No separate `scaffold()`
primitive was built; it would just be this same composition with extra
ceremony.

---

## Developer Tooling

### `gx repl` — Interactive REPL

```bash
gx repl [--trace] [--log-level <level>]
```

A real interactive development environment, not a line-at-a-time
`run_program` wrapper:

- **State persists across lines.** `x = 42` on one line, `x` visible on
  every line after — backed by one persistent `Env` held for the whole
  session (see `Interpreter::run_repl_stmts`). Declarations
  (`function`/`agent`/`helper`/`tool`/`import`/`use`) still go through the
  ordinary program-execution path, so they behave exactly as they would in
  a file; everything else (assignments, expressions, calls) runs against
  the session's shared scope.
- **Multiline input.** An unclosed `{`/`(`/`[` switches the prompt to
  `... ` and keeps buffering until it balances — detected by tokenizing
  the buffered input and counting real bracket *tokens* (not raw
  characters), so a `{` inside a string literal is never mistaken for an
  unclosed block, and the input is only sent to the parser once it's
  actually complete.
- **Auto-print.** A bare expression's value prints automatically (`5 + 5`
  shows `10`); an assignment or a `say` (which already prints) doesn't
  echo anything extra.
- **Persistent history.** Every accepted line is appended to
  `~/.gx_history` (best-effort — a read-only home directory doesn't
  interrupt the session) and listed with `:history`.
- **`:help` / `:help <name>`** — REPL commands, or documentation for a
  specific builtin (reuses the same table `gx lsp`'s hover uses).
- **`:vars`** — list every variable currently in scope.
- **`:trace on|off`** — toggle diagnostics tracing mid-session, same
  effect as `--trace`.
- **Imports work.** `import "./lib.gx"` resolves relative to the
  directory `gx repl` was launched from, the same convention `gx run`
  already uses.

```
$ gx repl
gx> x = 10
gx> y = 20
gx> x + y
30
gx> function double(n) {
...   return n * 2
... }
gx> double(21)
42
gx> :vars
  x = 10
  y = 20
gx> exit
Goodbye!
```

**Known limitation**: no arrow-key line recall/in-place editing — that
needs raw-terminal handling (a new dependency, e.g. `rustyline`),
deliberately scoped out rather than half-implemented. `:history` still
shows the full session log.

### Diagnostics with source snippets

Parser, lexer, and runtime errors are rendered with the offending source
line and (when available) a caret at the exact column, Rust-compiler
style, for `gx run`, `gx check`, `gx -e`, and `gx repl`:

```
$ gx run script.gx
Error: expected identifier, got Say
  --> script.gx:4:5
   |
 4 |     say "unreachable"
   |     ^
```

This is rendering, not a new error format — GX's parser/lexer/interpreter
errors are still plain strings (`"Line N: message"` or `"Line N, col C:
message"`); the CLI parses that convention back out at the point it prints
the final error (see `src/diagnostics_render.rs`) rather than requiring a
structured error type throughout the whole codebase. A message that
doesn't follow the convention — or whose reported line is out of the
source's range — falls back to printing unchanged, so this can never hide
information a plainer error would have shown.

An uncaught assertion failure now also shows its call stack (`in agent
"..."`), matching every other kind of uncaught error — previously only
`Signal::Error` got that context; `assert`'s failure message deliberately
stays exactly what the script wrote when it's *caught* (`e.message` must
keep matching it verbatim), so the call stack is only ever added at the
top-level, uncaught conversion, never to what a `catch` block sees.

**Known limitation**: this location-recovery approach only works where an
error's message already embeds a line number in one of the two
conventions above. Not every runtime error does (e.g. a caught-then-
rethrown value, or a message built entirely outside those two call sites)
— those still print without a snippet, exactly as before this milestone,
never with a wrong one.

### `gx doc` — API reference generation

```bash
gx doc <file.gx|dir> [--out <file.md>]
```

Generates a Markdown reference: every `function`, `agent`/`helper`, and
`tool` definition, its signature, and any `//`-comment block immediately
preceding its declaration in the source (GX's lexer discards comments
during tokenization, so this reads the doc comment from the raw source
text directly, not the parsed AST). Tools are self-documenting already —
`description` and each parameter's `description` are real AST fields, not
comments — and are used verbatim. Directory targets are scanned
recursively for every `.gx` file, the same discovery `gx test` and `gx
fmt` use. Prints to stdout without `--out`.

### `gx fmt` — directories and `--check`

```bash
gx fmt <file.gx|dir>            # format in place
gx fmt <file.gx|dir> --check    # report only, write nothing; exits non-zero if any file would change
```

`gx fmt` now accepts a directory (every `.gx` file found recursively) in
addition to a single file, and `--check` — the same CI-friendly
convention `cargo fmt --check`/`prettier --check` established — for
verifying formatting without touching anything.

This milestone also fixed two real, pre-existing formatter bugs
uncovered while building `--check` (which depends on `gx fmt` being
idempotent — format, then format again, must produce identical output —
to ever succeed right after `gx fmt` itself runs):

- The brace-syntax formatter's token-to-text conversion had a catch-all
  fallback covering nearly every keyword beyond a small hand-picked set —
  `fn`, `assert`, `while`, `import`, `parallel`, `tool`, `await`, and
  more all silently vanished from the formatted output. `assert x ==
  y` could become `x == y` (assertion silently removed), `fn(n) { ... }`
  could become `(n) { ... }` (invalid syntax). The conversion is now an
  exhaustive match over every token kind — a future keyword that's missed
  is a compile error, not a silently-deleted token.
- String literals containing `\n`/`\t`/`\\`/`\"` were re-emitted with the
  literal decoded characters instead of the escape sequence, silently
  turning a one-line string into a multi-line one in the formatted
  output.

Both were verified fixed against every real `.gx` file in this repository
(formats cleanly, still parses, still runs with identical behavior, and
is now idempotent).

### `gx lsp` — Language Server

```bash
gx lsp
```

A real, intentionally-scoped Language Server over stdio (JSON-RPC 2.0,
hand-rolled framing — no new dependency). Point any LSP-capable editor's
GX language configuration at `gx lsp` as the server command. Implemented:

- **Diagnostics** (`textDocument/didOpen`/`didChange`) — re-parses on
  every edit and publishes the same column-aware errors `gx run`/`gx
  check` show, so an editor's error squiggles land on the exact token.
- **Hover** (`textDocument/hover`) — signatures for a curated set of the
  most commonly-used builtins (`retry`, `unwrap`, `has_capability`,
  `http_get`, `db_query`, `task_spawn`, ...; see `builtin_docs` in
  `src/lsp.rs` for the full list — deliberately not the entire several-
  hundred-entry Built-in Functions reference, though extending it is a
  one-line addition per entry), and for any `function`/`agent`/`tool`
  defined in the same file.
- **Go to definition** (`textDocument/definition`) — jumps to a
  `function`/`agent`/`helper`/`tool` declaration *within the same file*.

**Known, deliberate limitations** — each would be a substantial feature
in its own right, and a half-implemented version (a rename that misses
references, a completion list that's just keyword-matching) would be
worse than being honest it isn't there yet:

- No cross-file go-to-definition/find-references (a call into an
  `import`ed file's function won't jump, even though the Module & Package
  Runtime's import resolution could in principle supply this later).
- No rename, no completion/autocomplete suggestion lists, no semantic
  (token-type-aware) highlighting, no signature help while typing a call.
- Position columns follow the LSP spec's UTF-16 code-unit convention only
  for the common case — text containing astral-plane Unicode (rare
  emoji, some CJK extension characters) inside a line could misalign
  hover/definition by one position; GX source is overwhelmingly ASCII, so
  this is narrow in practice.
- No `initialize`-before-any-other-request enforcement — a well-behaved
  client (every mainstream editor) always sends `initialize` first
  anyway.

### `gx <command> --help`

Every subcommand now accepts `--help`/`-h` and prints its usage —
previously only the bare `gx help`/`gx --help` (no command) worked, and
`gx run --help` fell through to `run`'s own argument parsing, producing a
confusing "file not found: --help" instead of ever showing usage.

---

## Resource Limits

A handful of fixed, generous-but-not-unbounded ceilings exist across the
runtime specifically to turn "an unbounded resource leak eventually crashes
or wedges the process" into a normal, catchable error instead. These are
deliberate safety bounds, not configuration you're expected to tune — none
of them should be reachable by a realistic single workload, only by a bug
or a hostile input.

| Limit | Value | What happens at the limit |
|---|---|---|
| Bridge (`use js/ts/py/binary/go`) call timeout | 300s | The call returns a clear timeout error instead of blocking the calling task/worker forever; the bridge's underlying process is treated as unusable afterward and a fresh one is spawned on the next call. |
| Concurrent `respond stream` (SSE) responders, per server | 256 | A new `respond stream` call returns an error ("too many concurrent streaming connections") instead of accepting an unbounded number of threads blocked writing to clients that never read. |
| Pooled SQLite connections, per Interpreter | 128 | The least-recently-used *idle* connection (no active `db_transaction`) is closed to make room; a connection with an in-flight transaction is never closed to enforce this cap. |
| Concurrently-tracked tasks (`task_spawn`), per Interpreter | 10,000 | `task_spawn` returns a clear error naming the limit; call `task_wait`/`task_wait_all` on finished tasks to free slots, or use a bounded pool (`{ pool: "name", max_concurrent: N }`). |
| HTTP response / process output body size | 32 MiB | The body is truncated; the result reports `truncated: true` and the actual byte count rather than silently returning a partial body with no indication. |

---

**© 2026 DEVJSX LIMITED** — Ahmed Elgarhy
