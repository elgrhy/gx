# Building AI Agents with GX

AI is a first-class citizen in GX — no SDK, no boilerplate. Every AI call is automatically logged, confidence-scored, and token-tracked.

---

## Ask an AI Model

```gx
agent "assistant" {
  remember {
    question = "What is the capital of France?"
  }

  when started {
    result = ask openai {
      prompt: memory.question,
      max_tokens: 100
    }

    say result.text
    say "Confidence: {result.confidence}"
    say "Tokens used: {result.tokens_used}"
  }
}
```

```bash
export OPENAI_API_KEY=sk-...
gx run assistant.gx
```

---

## Supported Providers

```gx
// OpenAI
result = ask openai { prompt: "Hello" }
result = ask gpt { prompt: "Hello" }         // alias

// Anthropic
result = ask anthropic { prompt: "Hello" }
result = ask claude { prompt: "Hello" }      // alias

// Ollama — runs locally, no API key needed
result = ask ollama { prompt: "Hello" }
result = ask ollama:mistral { prompt: "Hello" }
```

For Ollama (free, local, private):
```bash
brew install ollama && ollama serve
ollama pull llama3
# No API key needed
```

---

## What Every AI Call Returns

| Field | Type | Description |
|---|---|---|
| `result.text` | String | The model's response |
| `result.confidence` | Number | 0.0–1.0 (lower when model hedges) |
| `result.tokens_used` | Number | Tokens for this call |
| `result.model` | String | Model name used |
| `result.provider` | String | `openai`, `anthropic`, `ollama` |
| `result.ok` | Bool | `false` if request failed |
| `result.tool_calls` | Array | Tool calls requested by the model |

---

## Token Tracking (v0.5.0)

GX tracks token usage across the entire run. No extra code needed.

```gx
agent "cost_aware" {
  when started {
    r1 = ask openai { prompt: "Summarize GX language", max_tokens: 100 }
    r2 = ask openai { prompt: "What is a brain-first language?", max_tokens: 100 }

    say r1.text
    say r2.text
    say "Total tokens this run: {tokens_used()}"   // cumulative across all ask calls
  }
}
```

Estimate tokens before calling:
```gx
text = read_file("document.txt")
est = token_count(text)
say "Estimated tokens: {est}"

if est > 4000 {
  text = truncate(text, 16000)   // ~4000 tokens
}
result = ask anthropic { prompt: text }
```

---

## Confidence Checking (Anti-Hallucination)

GX automatically reduces confidence when the model uses hedging language ("I think", "maybe", "I'm not sure").

```gx
agent "safe_assistant" {
  when started {
    result = ask openai {
      prompt: "What is the cure for cancer?",
      max_tokens: 150
    }

    if result.confidence > 0.75 {
      say result.text
    } else {
      say "I'm not confident enough to answer this."
      escalate to human
    }
  }
}
```

---

## System Prompts

```gx
result = ask anthropic {
  prompt: memory.user_question,
  system: "You are a concise assistant. Answer in one sentence.",
  max_tokens: 60
}
```

---

## Streaming AI

For long responses — chunks print in real-time while `result.text` holds the complete assembled text.

```gx
agent "writer" {
  when started {
    result = ask openai {
      prompt: "Write a 500-word essay on AI transparency",
      stream: true
    }
    say "Total tokens used: {result.tokens_used}"
  }
}
```

---

## AI Tool Use — Function Calling

```gx
tool "search_web" {
  description: "Search the web for current information"
  params: {
    query: { type: "string", required: true }
  }
  execute(query) {
    result = http_get("https://api.search.example.com?q={query}")
    return result.data
  }
}

tool "lookup_customer" {
  description: "Look up customer by ID"
  params: {
    customer_id: { type: "number", required: true }
  }
  execute(customer_id) {
    return http_get("https://api.example.com/customers/{customer_id}").data
  }
}

agent "researcher" {
  when started {
    response = ask openai {
      prompt:  "Find info about customer #42 and search for their latest order",
      tools:   [search_web, lookup_customer],
      model:   "gpt-4o"
    }

    if response.tool_calls != null {
      for each call in response.tool_calls {
        log("AI called: {call.name}")
      }
    }

    say response.text
  }
}
```

---

## Retry Pattern

```gx
agent "reliable" {
  remember {
    question = "Explain recursion simply."
    attempts = 0
  }

  when started {
    result = ask openai { prompt: memory.question, max_tokens: 100 }

    if result.confidence > 0.7 {
      say result.text
    } else if memory.attempts < 2 {
      memory.attempts += 1
      re-run
    } else {
      say "Could not get a confident answer."
      escalate to human
    }
  }
}
```

Or use the built-in retry with backoff:

```gx
result = retry(fn() {
  return ask openai { prompt: "classify this text", max_tokens: 50 }
}, 5, { delay: 1000, backoff: "exponential" })
```

---

## Classify Text

```gx
agent "router" {
  remember {
    message = "I want a refund for my order"
  }

  when started {
    label = infer classifier {
      input: memory.message,
      classes: ["support", "sales", "billing", "other"]
    }
    say "Category: {label}"
  }
}
```

---

## Embed Text

```gx
// Returns a float array (embedding vector)
// Requires OPENAI_API_KEY
vector = embed "text to convert to semantic vector"
```

---

## Vector Store — Semantic Search

```gx
agent "semantic" {
  when started {
    store = vector_store_new("knowledge")

    vector_store_add(store, "doc1", embed("The cat sat on the mat"), "cat story")
    vector_store_add(store, "doc2", embed("Dogs are loyal companions"), "dog story")

    hits = vector_store_search(store, embed("feline pets"), 3)
    log(hits[0].label)    // "cat story"
    log(hits[0].score)    // cosine similarity score
  }
}
```

---

## AI Trace — Full Audit Log

Every `ask` call is appended to `memory.ai_trace` automatically. Zero extra code.

```gx
agent "auditable" {
  remember {
    ai_trace = []
  }

  when started {
    result = ask openai { prompt: "What is 2 + 2?" }
    say result.text
    say "AI calls made: {len(memory.ai_trace)}"
    say "Total tokens: {tokens_used()}"

    // Full trace available for inspection / export
    for each entry in memory.ai_trace {
      log("Call: {entry.provider} — {entry.tokens_used} tokens")
    }
  }
}
```

---

## Observability — Structured JSONL Tracing

```gx
trace_log("pipeline.start", { query: memory.query })
result = ask anthropic { prompt: memory.query }
trace_log("ai.done", { tokens: result.tokens_used, confidence: result.confidence })
// Emits: {"ts":1748609381000,"agent":"my_agent","event":"ai.done","data":{...}} to stderr
```

Pipe to a file:
```bash
gx run agent.gx 2>> trace.jsonl
```

---

## Multi-Agent AI Pipeline

```gx
helper "classifier" {
  brain {
    plan { }
    execute {
      label = infer classifier {
        input: input.text,
        classes: ["urgent", "normal", "low"]
      }
    }
    remember { }
    communicate { label }
  }
}

helper "responder" {
  brain {
    plan { }
    execute {
      result = ask openai {
        prompt: "Reply to this {input.priority} request: {input.text}",
        max_tokens: 200
      }
    }
    remember { }
    communicate { result.text }
  }
}

agent "dispatcher" {
  when started {
    priority = spawn agent "classifier" with { text: memory.incoming }
    reply    = spawn agent "responder"  with { text: memory.incoming, priority: priority }
    say reply
    say "Tokens used: {tokens_used()}"
  }
}
```

---

## Environment Variables

```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
# Ollama: no key needed — run: ollama serve
```

---

## Next Steps

- [Language Reference](language_reference.md) — complete syntax
- [Getting Started](getting_started.md) — basics
- [Examples](examples/) — runnable example programs
