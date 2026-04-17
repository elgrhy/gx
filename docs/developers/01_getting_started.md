# Getting Started with GX

## Install

```bash
# macOS / Linux
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm
npm install -g gxlang

# From source
git clone https://github.com/elgrhy/gx.git && cd gx && cargo build --release
```

Verify:
```bash
gx version
```

---

## Your First Agent

Create `hello.gx`:

```gx
agent "greeter" {
  remember {
    name = "World"
  }

  when started {
    say "Hello, {memory.name}!"
  }

  brain {
    plan {}
    execute {}
    remember {}
    communicate {}
  }
}
```

Run it:
```bash
gx run hello.gx
# Hello, World!
```

---

## The Brain Cycle

Every GX helper runs a four-phase cognitive cycle: **Plan → Execute → Remember → Communicate**.

```gx
helper "counter" {
  remember {
    count = 0
  }

  brain {
    plan {
      plan = { action: "increment" }
    }
    execute {
      if plan.action == "increment" {
        memory.count += 1
        log("Count is now: " + to_string(memory.count))
      }
    }
    remember {
      memory.last_updated = get_timestamp()
    }
    communicate {
      emit "count_changed" { value: memory.count }
    }
  }
}
```

---

## Memory

Memory is the agent's persistent state. Declared in `remember {}`, accessible everywhere as `memory.key`.

```gx
agent "memo" {
  remember {
    runs = 0
    history = []
    config = { debug: false }
  }

  when started {
    memory.runs += 1
    say "Run number {memory.runs}"

    if memory.config.debug {
      log("Debug mode is on")
    }
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

---

## Control Flow

```gx
// if / else if / else
if memory.count > 10 {
  say "high"
} else if memory.count > 5 {
  say "medium"
} else {
  say "low"
}

// for loop
for item in memory.items {
  log(item)
}

// try / catch
try {
  result = risky_operation()
} catch e {
  log("Failed: " + e)
}
```

---

## Scaffold a Project

```bash
gx init my-project
cd my-project
gx run main.gx
gx test
```

---

## Next Steps

- [AI Agents](06_ai_applications.md) — connect to OpenAI, Anthropic, Ollama
- [API Reference](../API_REFERENCE.md) — complete language reference
- [Examples](../examples/) — runnable `.gx` programs
