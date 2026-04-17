# Recipes and Functions in GX

> Full syntax reference: [../API_REFERENCE.md](../API_REFERENCE.md)

GX uses `recipe` blocks for reusable named operations within a helper.

```gx
helper "calculator" {
  remember {
    result = 0
  }

  recipe "add" {
    needs: a, b
    gives: sum
    brain {
      plan {}
      execute { sum = a + b }
      remember {}
      communicate {}
    }
  }

  brain {
    plan { plan = { action: "compute" } }
    execute {
      if plan.action == "compute" {
        memory.result = add(5, 3)
        log("Result: " + to_string(memory.result))
      }
    }
    remember {}
    communicate {}
  }
}
```

For simple logic, use `if/else` chains directly in the brain cycle — no recipe needed.
