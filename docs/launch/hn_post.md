# Hacker News Launch Post

**Title:** Show HN: GX – a brain-first language for building auditable AI agents (Rust, v0.1.0)

---

Hi HN,

I've been building GX, a programming language designed specifically for AI agents.

The core problem I wanted to solve: every AI assistant today is a black box. You don't know what it decided, why, or how confident it was. GX makes that visible by design.

**The language has three syntax levels — all compile to the same runtime:**

Level 1 (zero friction):
```
Agent greeter
name = "World"
"Hello {name}"
```

Level 2 (named behaviors):
```
Agent assistant
Greet:
  "Hello {name}"
On start:
  Greet
```

Level 3 (explicit cognitive cycle):
```
Agent counter
Plan:
  action = "increment"
Execute:
  count += 1
Communicate:
  "Count: {count}"
```

**AI is built into the language — every call is auto-logged:**
```
result = ask openai { prompt: "Summarize: {memory.text}" }
if result.confidence > 0.8 {
  say result.text
} else {
  escalate to human
}
```

**No ecosystem lock-in:**
```
use js.axios
use py.pandas
data = js.axios.get("https://api.example.com")
```

**What's working in v0.1.0:**
- Full Rust tree-walking interpreter
- Both brace syntax and indentation-based progressive syntax
- `ask openai/anthropic/ollama`, `embed`, `infer classifier`
- User-defined functions with recursion
- Multi-file support via `import "file.gx"`
- Package interop: `use js.X`, `use py.X`
- Toolchain: `gx init/run/build/test/fmt/make`
- curl installer, npm package (`npm install -g gxlang`), Homebrew

**Install:**
```bash
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh
# or
npm install -g gxlang
```

GitHub: https://github.com/elgrhy/gx

I'd love feedback on the progressive syntax model — specifically whether Level 1 is too implicit, and whether the brain cycle abstraction (Plan/Execute/Remember/Communicate) feels natural to you.

---

# Reddit r/programming Post

**Title:** I built a programming language where every AI decision is visible and auditable — GX v0.1.0

GX is a language I built because I was frustrated with AI assistants being black boxes. The language has three progressive syntax levels:

**Simple (no boilerplate):**
```
Agent greeter
name = "World"
"Hello {name}"
```

**Structured (named behaviors):**
```
Agent bot
Greet:
  say "Hello!"
On start:
  Greet
```

**Explicit brain cycle:**
```
Agent analyzer
Plan:
  action = "analyze"
Execute:
  result = ask anthropic { prompt: memory.input }
Remember:
  memory.last_result = result.text
Communicate:
  result.text
```

Every `ask` call automatically logs confidence scores and model info. When confidence drops below your threshold, `escalate to human` stops the agent and surfaces it for review.

Built in Rust. v0.1.0 is out now.

GitHub: https://github.com/elgrhy/gx
Install: `npm install -g gxlang`

---

# Twitter/X Launch Thread

**Tweet 1:**
Shipping GX v0.1.0 🧠

A programming language for building auditable AI agents.

Every AI decision: visible. Every call: logged. Confidence scores: built in.

Three ways to write it — pick your level. 🧵

**Tweet 2:**
Level 1 — pure intent (no syntax overhead):
```
Agent greeter
name = "World"
"Hello {name}"
```
Variables auto-become memory. Strings auto-print. GX handles the rest.

**Tweet 3:**
Level 2 — named behaviors:
```
Agent assistant
Greet:
  "Hello {name}"
On start:
  Greet
```
Behaviors are reusable, composable blocks. No functions needed.

**Tweet 4:**
Level 3 — explicit brain cycle:
```
Agent counter
Plan:
  action = "increment"
Execute:
  count += 1
Communicate:
  "Count: {count}"
```
When you want full control over Plan → Execute → Remember → Communicate.

**Tweet 5:**
AI primitives are first-class syntax:
```
result = ask openai {
  prompt: "Analyze: {memory.text}"
}
if result.confidence > 0.8 {
  say result.text
} else {
  escalate to human
}
```
`confidence` score built in. Auto-logged to `memory.ai_trace`.

**Tweet 6:**
Use any npm or Python package directly:
```
use js.axios
use py.pandas
data = js.axios.get("https://api.example.com")
```
No ecosystem lock-in.

**Tweet 7:**
Built in Rust. Runs on macOS/Linux/Windows.

Install:
```
npm install -g gxlang
# or
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh
```

GitHub: github.com/elgrhy/gx

RT if you've ever wanted to actually see what your AI agent is thinking 🧠
