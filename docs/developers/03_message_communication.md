# Message Communication in GX

> Full syntax reference: [../API_REFERENCE.md](../API_REFERENCE.md)

Helpers communicate by emitting events.

```gx
helper "producer" {
  brain {
    plan {}
    execute {
      memory.result = 42
    }
    remember {}
    communicate {
      emit "result_ready" { value: memory.result }
    }
  }
}
```

Receive events with a `receive` block:

```gx
helper "consumer" {
  receive {
    channel "data" {
      source: "producer"
      type: "result_ready"
      bind: memory.incoming
    }
  }

  brain {
    plan {}
    execute {
      log("Received: " + to_string(memory.incoming))
    }
    remember {}
    communicate {}
  }
}
```

`broadcast` emits to all listening helpers:

```gx
broadcast "shutdown"
```
