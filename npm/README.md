# GX Language

> A brain-first programming language for building transparent, auditable AI assistants.

[![npm version](https://img.shields.io/npm/v/gxlang)](https://www.npmjs.com/package/gxlang)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://github.com/elgrhy/gx)

Every AI assistant today is a black box. GX makes it a glass box — every decision explicit, every AI call logged, every agent fully auditable. Built in Rust. No cloud lock-in.

## What's New in v0.2.0

- **Opinionated agent style** — `goal`, `think`, `act`, `observe` make every agent self-documenting
- **`think { prompt, model, min_confidence }`** — AI call with automatic confidence gate and escalation
- **`loop until condition`** / **`repeat N times`** — clean loop sugar
- **`parallel { ... }`** — run multiple steps concurrently
- **`retry: N`** / **`on_error: continue|escalate`** — declarative error handling on any agent
- **`http_request { url, method, body, headers }`** — unified HTTP client
- **`send_email { to, subject, body }`** — native SMTP (no library needed)
- **`scrape "url"`** — fetch + strip HTML → clean plain text
- **`notify { channel: "slack", message }`** — webhook notifications
- **`ord(c)`** / **`chr(n)`** / **`is_digit`** / **`is_alpha`** / **`is_whitespace`** — character primitives
- **Multi-agent orchestration** — `spawn agent`, `|>` pipelines, `when message`
- **Production example** — full UAE Legal Assistant in `examples/legal_assistant/`

---

## Install

```bash
npm install -g gxlang
```

Downloads the correct pre-built binary for your platform (macOS arm64/x64, Linux x64/arm64, Windows x64).

Verify:

```bash
gx version
```

---

## The Official GX Style

GX is opinionated. Every agent follows a clear structure — **goal → observe → think → act → remember → communicate**. Anyone can read it without running it.

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
      for each lead in result.leads {
        log("Lead: {lead.name} — {lead.email}")
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

---

## Multi-Agent Orchestration

Agents can call each other, chain through pipelines, and exchange messages.

```gx
// Call an agent and get its result
category = spawn agent "classifier" with { question: input }

// Pipeline: output of one agent becomes input of the next
result = { question: input } |> spawn agent "classifier" |> spawn agent "researcher"

// Send a message to another agent
spawn "task" to "worker" with { payload: data }
```

---

## Three Syntax Levels

All three compile to the same runtime.

### Level 1 — Pure intent
```gx
Agent greeter
name = "World"
"Hello {name}"
```

### Level 2 — Named behaviors
```gx
Agent assistant
user = "Ahmed"

Greet:
  say "Hello {user}!"

On start:
  Greet
```

### Level 3 — Explicit brain cycle
```gx
Agent analyzer
Plan:
  action = "analyze"
Execute:
  result = ask openai { prompt: "Analyze this: {input}" }
Remember:
  memory.last = result.text
Communicate:
  say result.text
```

### Classic brace syntax
```gx
agent "greeter" {
  remember { name = "World" }
  when started { say "Hello, {memory.name}!" }
}
```

---

## AI Primitives

```gx
// Ask any provider — every call auto-logged
result = ask openai    { prompt: "Summarize: {text}", max_tokens: 100 }
result = ask anthropic { prompt: "Translate to Arabic: {text}" }
result = ask ollama    { prompt: "What is recursion?" }  // local, no API key

// result.text        — the response
// result.confidence  — 0.0–1.0, auto-adjusted for hedging language
// result.tokens_used — for cost tracking
// result.ok          — false if request failed

// Embed text
vector = embed "semantic search query"

// Classify
label = infer classifier {
  input: user_message,
  classes: ["support", "billing", "sales", "spam"]
}
```

Set API keys:
```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# ollama: brew install ollama && ollama serve
```

---

## Native Tools

All work out of the box — no npm install required.

```gx
// HTTP
result  = http_get("https://api.example.com/data")
result  = http_request({ url: "https://api.example.com", method: "POST", body: payload })

// Files
content = read_file("data.txt")
write_file("output.txt", content)
lines   = read_file_lines("log.txt")
exists  = file_exists("config.json")
entries = list_dir("./agents")

// Email (uses SMTP_HOST, SMTP_USER, SMTP_PASS env vars)
send_email({ to: "user@example.com", subject: "Alert", body: "Agent completed." })

// Scrape
text = scrape("https://example.com")

// Notify (Slack/webhook)
notify({ channel: "slack", message: "Agent done" })

// JSON
obj  = json_parse(read_file("data.json"))
raw  = json_stringify(obj)

// Character
code = ord("A")    // 65
ch   = chr(65)     // "A"
is_digit("5")      // true
```

---

## Package Interop

```gx
use js.axios
use py.pandas

agent "analyst" {
  when started {
    data = js.axios.get("https://api.example.com/data")
    cwd  = py.os.getcwd()
  }
}
```

Install:
```bash
gx install js.axios
gx install py.requests
```

---

## CLI Reference

| Command | Description |
|---------|-------------|
| `gx run file.gx` | Run a GX file |
| `gx run file.gx --debug` | Run with debug output |
| `gx check file.gx` | Syntax check without running |
| `gx init my-project` | Scaffold a new project |
| `gx build file.gx` | Build a standalone launcher |
| `gx install js.axios` | Install npm package for GX |
| `gx install py.requests` | Install Python package for GX |
| `gx fmt file.gx` | Format GX source |
| `gx test` | Run all files in `tests/` |
| `gx make "a weather bot"` | AI-generate GX code |
| `gx version` | Print version |

---

## Control Flow

```gx
// Standard
if x > 10 { log("big") } else { log("small") }
for each item in list { log(item) }
while running { check() }
try { risky() } catch e { log(e) }

// v0.2.0 sugar
loop until done { process() }
repeat 5 times { tick() }
repeat 10 times as i { log("step {i}") }
parallel { step_a() step_b() step_c() }
```

---

## Memory

```gx
agent "tracker" {
  remember {
    count = 0
    history = []
  }
  when started {
    memory.count += 1
    memory.history = memory.history.push("run {memory.count}")
    log(memory.ai_trace)   // every AI call ever made, auto-logged
  }
}
```

---

## When Blocks

```gx
when started        { /* runs once on startup */ }
when memory.x > 10  { /* runs when condition is true */ }
when memory.status changes { re-run }
when message "task" { log("received: {message.task}") }
```

---

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| macOS | arm64 (Apple Silicon) | ✅ |
| macOS | x86_64 (Intel) | ✅ |
| Linux | x86_64 | ✅ |
| Linux | arm64 | ✅ |
| Windows | x86_64 | ✅ |

---

## More

- **GitHub:** https://github.com/elgrhy/gx
- **Language Reference:** https://github.com/elgrhy/gx/blob/main/docs/language_reference.md
- **Examples:** https://github.com/elgrhy/gx/tree/main/examples
- **Getting Started:** https://github.com/elgrhy/gx/blob/main/docs/getting_started.md
- **Playground:** https://elgrhy.github.io/gx/playground/

---

## License

MIT — © 2026 [DEVJSX LIMITED](https://devjsx.com) · Ahmed Elgarhy
