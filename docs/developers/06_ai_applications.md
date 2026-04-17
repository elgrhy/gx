# Building AI Agents with GX

GX has AI built in as first-class language features — no SDK wrappers, no boilerplate.

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

Set your API key:
```bash
export OPENAI_API_KEY=sk-...
gx run assistant.gx
```

---

## Supported Providers

```gx
// OpenAI
result = ask openai { prompt: "Hello" }
result = ask gpt { prompt: "Hello" }  // alias

// Anthropic
result = ask anthropic { prompt: "Hello" }
result = ask claude { prompt: "Hello" }  // alias

// Ollama (local — no API key, no cloud)
result = ask ollama { prompt: "Hello" }
result = ask ollama:mistral { prompt: "Hello" }
```

For Ollama:
```bash
brew install ollama
ollama serve
ollama pull llama3
```

---

## Confidence Checking

Every AI response includes a `confidence` score (0.0–1.0). Use it to prevent hallucinations.

```gx
agent "fact_checker" {
  when started {
    result = ask openai {
      prompt: "What is the population of Mars?",
      max_tokens: 100
    }

    if result.confidence < 0.7 {
      say "I'm not confident about this answer."
      escalate to human
    } else {
      say result.text
    }
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

Confidence is reduced automatically when the response contains hedging language ("I think", "maybe", "I'm not sure", etc.).

---

## System Prompts

```gx
result = ask anthropic {
  prompt: memory.user_question,
  system: "You are a concise assistant. Answer in one sentence only.",
  max_tokens: 50
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

    say "Classified as: {label}"
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

---

## Embed Text

```gx
// Returns an array of floats (embedding vector)
// Requires OPENAI_API_KEY
vector = embed "semantic search query"
```

---

## AI Trace (Anti-Hallucination)

Every `ask` call is automatically appended to `memory.ai_trace`. This gives you a full audit log of all AI interactions without writing any logging code.

```gx
agent "auditable" {
  remember {
    ai_trace = []
  }

  when started {
    result = ask openai { prompt: "Explain gravity" }
    say result.text

    // memory.ai_trace now contains the full call record
    say "Total AI calls: {len(memory.ai_trace)}"
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

---

## Full Example: AI Assistant with Fallback

```gx
agent "smart_assistant" {
  remember {
    question = "Summarize quantum entanglement in 2 sentences."
    retries = 0
    max_retries = 2
  }

  when started {
    result = ask openai {
      prompt: memory.question,
      max_tokens: 150
    }

    if result.confidence > 0.75 {
      say result.text
    } else if memory.retries < memory.max_retries {
      memory.retries += 1
      say "Low confidence ({result.confidence}), trying again..."
      re-run
    } else {
      say "Could not get a confident answer after {memory.retries} attempts."
      escalate to human
    }
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

---

## Next Steps

- [API Reference](../API_REFERENCE.md) — complete AI primitive docs
- [Getting Started](01_getting_started.md) — basics
- [Examples](../examples/ai_assistant.gx) — runnable example
