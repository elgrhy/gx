# Getting Started with GX

## Install

```bash
# macOS / Linux (recommended)
curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

# npm (any platform with Node.js 16+)
npm install -g gxlang

# From source (requires Rust)
git clone https://github.com/elgrhy/gx.git
cd gx && cargo build --release
sudo cp target/release/gx /usr/local/bin/
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

## Create a Project

```bash
gx init my-project
cd my-project
gx run main.gx
```

This creates:
```
my-project/
├── gx.json       # Project config
├── main.gx       # Entry point
├── agents/       # Put your agents here
└── tests/        # Run with: gx test
```

---

## The Brain Cycle

Every agent follows four phases: **Plan → Execute → Remember → Communicate**.

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
        log("Count: " + to_string(memory.count))
      }
    }
    remember {
      memory.last_run = get_timestamp()
    }
    communicate {
      emit "tick" { count: memory.count }
    }
  }
}
```

---

## Memory

Declare variables in `remember {}`. Access them as `memory.key` everywhere.

```gx
agent "memo" {
  remember {
    runs = 0
    items = []
    config = { debug: false }
  }

  when started {
    memory.runs += 1
    say "Run {memory.runs}"
  }

  brain { plan {} execute {} remember {} communicate {} }
}
```

---

## Control Flow

```gx
// if / else if / else
if memory.score > 90 {
  say "excellent"
} else if memory.score > 60 {
  say "ok"
} else {
  say "needs work"
}

// for loop
for each item in memory.items {
  log(item)
}

// try / catch
try {
  result = risky()
} catch e {
  log("error: " + e)
}
```

---

## String Interpolation

```gx
name = "Ahmed"
count = 42
say "Hello {name}, count is {count}"
```

---

## Scaffold, Test, Build

```bash
gx init my-agent        # new project
gx run main.gx          # run
gx check main.gx        # syntax check only
gx test                 # run all tests/
gx build main.gx        # build standalone launcher → dist/main
gx fmt main.gx          # format source
```

---

## Next Steps

- [Language Reference](language_reference.md) — complete syntax and built-ins
- [AI Agents](ai_agents.md) — connect to OpenAI, Anthropic, Ollama
- [Examples](examples/) — runnable `.gx` programs
