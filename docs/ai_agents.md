# Building AI Agents with GX

AI is a first-class citizen in GX — no SDK, no boilerplate. Every AI call is automatically logged.

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
  }

  brain { plan {} execute {} remember {} communicate {} }
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
result = ask gpt { prompt: "Hello" }        // alias

// Anthropic
result = ask anthropic { prompt: "Hello" }
result = ask claude { prompt: "Hello" }     // alias

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
|-------|------|-------------|
| `result.text` | String | The model's response |
| `result.confidence` | Number | 0.0–1.0 (lower when model hedges) |
| `result.tokens_used` | Number | For cost tracking |
| `result.model` | String | Model name |
| `result.provider` | String | `openai`, `anthropic`, `ollama` |
| `result.ok` | Bool | `false` if request failed |

---

## Confidence Checking (Anti-Hallucination)

GX automatically reduces confidence when the model uses hedging language ("I think", "maybe", "I'm not sure"). Use this to prevent bad answers from reaching users.

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

  brain { plan {} execute {} remember {} communicate {} }
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

  brain { plan {} execute {} remember {} communicate {} }
}
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

  brain { plan {} execute {} remember {} communicate {} }
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

## AI Trace — Full Audit Log

Every `ask` call is appended to `memory.ai_trace` automatically. Zero extra code needed.

```gx
agent "auditable" {
  remember {
    ai_trace = []
  }

  when started {
    result = ask openai { prompt: "What is 2 + 2?" }
    say result.text
    say "AI calls made: {len(memory.ai_trace)}"
  }

  brain { plan {} execute {} remember {} communicate {} }
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
- [Examples](examples/ai_assistant.gx) — runnable example
