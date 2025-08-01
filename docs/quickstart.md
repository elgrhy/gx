# 🚀 GX Quick Start Guide

Get up and running with GX in minutes! This guide will teach you the basics of cognitive-first programming.

## 🎯 What You'll Learn

- How to create your first GX agent
- Understanding the mental cycle (Think → Act → Save → Reflect)
- Working with variables and memory
- Basic agent communication

## 📝 Your First GX Agent

Let's start with a simple "Hello World" agent:

```gx
agent "hello_world" {
  capabilities: ["render", "communication"]
  
  memory {
    message = "Hello, Cognitive World!"
    counter = 0
  }

  mental {
    think {
      plan = {
        action: "display_message",
        message: memory.message
      }
    }
    
    act {
      if plan.action == "display_message" {
        render.text(plan.message)
      }
    }
    
    save {
      memory.counter += 1
      memory.last_action = plan.action
    }
    
    reflect {
      log("Message displayed successfully")
      emit "message_sent"
    }
  }
}
```

## 🧠 Understanding the Mental Cycle

Every GX agent follows this cognitive cycle:

### 1. **Think** - Analyze and Plan
```gx
think {
  plan = {
    action: "display_message",
    message: memory.message
  }
}
```
- Analyze the current situation
- Create a plan for what to do next
- Store the plan for the next step

### 2. **Act** - Execute Actions
```gx
act {
  if plan.action == "display_message" {
    render.text(plan.message)
  }
}
```
- Execute the plan from the think phase
- Perform actual work (API calls, rendering, etc.)
- Interact with other agents or systems

### 3. **Save** - Persist State
```gx
save {
  memory.counter += 1
  memory.last_action = plan.action
}
```
- Update the agent's memory
- Store results and state changes
- Prepare for the next cycle

### 4. **Reflect** - Evaluate and Signal
```gx
reflect {
  log("Message displayed successfully")
  emit "message_sent"
}
```
- Evaluate what just happened
- Emit signals to other agents
- Log important information

## 📦 Working with Variables

In GX, all variables are stored in the `memory` block:

```gx
agent "calculator" {
  memory {
    // Simple variables
    result = 0
    numbers = [10, 20, 30]
    
    // Complex objects
    user_data = {
      name: "Alice",
      preferences: ["dark_theme", "notifications"]
    }
  }

  mental {
    think {
      plan = {
        action: "calculate_sum",
        numbers: memory.numbers
      }
    }
    
    act {
      if plan.action == "calculate_sum" {
        memory.result = sum(plan.numbers)
      }
    }
    
    save {
      memory.last_calculation = memory.result
    }
    
    reflect {
      log("Calculated: " + memory.result)
    }
  }
}
```

## 🤝 Agent Communication

Agents communicate through **assignments** and **signals**:

### Assigning Work to Other Agents

```gx
agent "orchestrator" {
  mental {
    think {
      plan = {
        action: "validate_data",
        data: memory.input_data
      }
    }
    
    act {
      if plan.action == "validate_data" {
        assign.agent("validator").validate(plan.data)
      }
    }
    
    save {
      memory.last_assignment = plan.action
    }
    
    reflect {
      log("Work assigned to validator")
    }
  }
}
```

### Emitting Signals

```gx
agent "data_processor" {
  mental {
    think {
      plan = {
        action: "process_data"
      }
    }
    
    act {
      if plan.action == "process_data" {
        // Process the data
        memory.processed = true
      }
    }
    
    save {
      memory.last_processed = get_timestamp()
    }
    
    reflect {
      if memory.processed {
        emit "data_ready"
      } else {
        emit "processing_error"
      }
    }
  }
}
```

## 🎯 Goals and Conditions

Agents can have **goals** that trigger based on conditions:

```gx
agent "monitor" {
  memory {
    temperature = 25
    threshold = 30
  }

  mental {
    think {
      plan = {
        action: "check_temperature"
      }
    }
    
    act {
      if plan.action == "check_temperature" {
        memory.temperature = get_current_temperature()
      }
    }
    
    save {
      memory.last_check = get_timestamp()
    }
    
    reflect {
      log("Temperature checked: " + memory.temperature)
    }
  }

  goal "high_temperature_alert" {
    when memory.temperature > memory.threshold
    then {
      action: "send_alert",
      message: "Temperature too high: " + memory.temperature
    }
  }
}
```

## 🔄 Running Your First GX Application

1. **Save your code** to a `.gx` file (e.g., `my_app.gx`)
2. **Run the application**:
   ```bash
   gx run my_app.gx
   ```
3. **Watch the mental cycles** execute in real-time!

## 🚀 Next Steps

Now that you understand the basics:

1. **Read [Core Concepts](core_concepts.md)** - Deep dive into GX fundamentals
2. **Explore [Keywords Reference](keywords.md)** - Complete language reference
3. **Try [Examples](examples.md)** - See more complex applications
4. **Build something!** - Create your own cognitive application

## 💡 Pro Tips

- **Start simple**: Begin with one agent, then add complexity
- **Use meaningful names**: Make your agents and variables descriptive
- **Log everything**: Use `log()` in reflect blocks for debugging
- **Think in cycles**: Always consider the Think → Act → Save → Reflect flow
- **Embrace delegation**: Use `assign.agent()` to break complex tasks into smaller agents

---

**Ready to build your first cognitive application?** Let's dive deeper into the [Core Concepts](core_concepts.md)! 