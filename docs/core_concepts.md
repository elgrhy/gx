# 🧠 GX Core Concepts

Understanding the fundamental building blocks of cognitive-first programming in GX.

## 🎯 What is GX?

GX is a **cognitive-first programming language** that structures code around mental processes rather than traditional imperative logic. Instead of writing step-by-step instructions, you create **agents** that think, act, save, and reflect - mirroring human cognitive processes.

## 🧠 The Mental Model

### Cognitive Programming Paradigm

GX is built around the principle that **code should think like humans think**. This means:

1. **Analysis**: Understand the current situation
2. **Planning**: Decide what to do next
3. **Execution**: Perform the planned actions
4. **Evaluation**: Assess the results
5. **Learning**: Adapt based on experience

### The Mental Cycle

Every GX agent follows this cognitive cycle:

```
INPUT → THINK → ACT → SAVE → REFLECT → GOAL → SIGNAL → RESOLVE
```

## 🤖 Agents: The Building Blocks

### What is an Agent?

An **agent** is a cognitive entity in GX that:

- Has its own **memory** (persistent state)
- Possesses **capabilities** (what it can do)
- Follows **mental cycles** (think → act → save → reflect)
- Communicates with other agents
- Can have **goals** (conditional behavior)

### Agent Structure

```gx
agent "agent_name" {
  capabilities: ["capability1", "capability2"]
  
  memory {
    // Persistent state storage
    variable1 = "value"
    variable2 = 42
  }
  
  mental {
    think { /* analysis and planning */ }
    act { /* execution */ }
    save { /* state persistence */ }
    reflect { /* evaluation and signaling */ }
  }
  
  goal "goal_name" {
    when condition
    then { action: "action_name" }
  }
  
  signal "signal_name" {
    do { /* signal handler */ }
  }
}
```

### Agent Lifecycle

1. **Creation**: Agent is spawned with initial memory
2. **Initialization**: Agent sets up its capabilities and state
3. **Mental Cycles**: Agent continuously runs think → act → save → reflect
4. **Communication**: Agent sends/receives signals and assignments
5. **Termination**: Agent completes its work or is stopped

## 🧠 Mental Processes

### Think Phase

The **think** phase is where agents analyze and plan:

```gx
think {
  plan = {
    action: "process_data",
    data: memory.input_data,
    priority: "high"
  }
  
  analysis = {
    input_valid: validate(memory.user_input),
    required_fields: ["name", "email"],
    missing_fields: get_missing_fields(memory.user_input)
  }
}
```

**Purpose**:
- Analyze current situation
- Create a plan for what to do next
- Evaluate conditions and constraints
- Prepare for action

### Act Phase

The **act** phase is where agents execute their plans:

```gx
act {
  if plan.action == "process_data" {
    memory.result = process(plan.data)
  } else if plan.action == "validate_input" {
    if plan.analysis.input_valid {
      assign.agent("processor").process(plan.data)
    } else {
      assign.agent("error_handler").handle_invalid_input(plan.data)
    }
  }
}
```

**Purpose**:
- Execute the plan from think phase
- Perform actual work (API calls, data processing, UI updates)
- Interact with other agents or systems
- Handle errors and exceptions

### Save Phase

The **save** phase is where agents persist their state:

```gx
save {
  memory.last_action = plan.action
  memory.timestamp = get_timestamp()
  memory.attempts += 1
  memory.results.push(plan.result)
}
```

**Purpose**:
- Update agent memory with new state
- Store results and outcomes
- Prepare for next mental cycle
- Maintain persistent state

### Reflect Phase

The **reflect** phase is where agents evaluate and signal:

```gx
reflect {
  if memory.success {
    emit "operation_complete"
  } else {
    emit "operation_failed"
  }
  
  log("Operation completed with " + memory.attempts + " attempts")
}
```

**Purpose**:
- Evaluate what just happened
- Emit signals to other agents
- Log important information
- Trigger further actions based on results

## 🧠 Memory System

### Memory as State Storage

In GX, all variables and state are stored in the agent's **memory**:

```gx
memory {
  // Simple variables
  status = "idle"
  counter = 0
  
  // Complex objects
  user_data = {
    name: "Alice",
    email: "alice@example.com",
    preferences: ["dark_theme", "notifications"]
  }
  
  // Arrays
  history = []
  
  // Nested objects
  settings = {
    theme: "dark",
    language: "en",
    notifications: {
      email: true,
      push: false
    }
  }
}
```

### Memory Access

Access memory variables using `memory.variable_name`:

```gx
mental {
  think {
    plan = {
      action: "process",
      data: memory.user_data,
      settings: memory.settings
    }
  }
  
  act {
    if plan.action == "process" {
      memory.result = process(plan.data)
      memory.history.push({
        timestamp: get_timestamp(),
        action: plan.action,
        result: memory.result
      })
    }
  }
}
```

### Memory Characteristics

- **Persistent**: Memory survives across mental cycles
- **Local**: Each agent has its own memory
- **Structured**: Can store any data type
- **Accessible**: Available in all mental phases

## 🎯 Goals and Conditions

### Goal-Based Programming

**Goals** are conditional logic that triggers based on memory state:

```gx
goal "high_temperature_alert" {
  when memory.temperature > memory.threshold
  then {
    action: "send_alert",
    message: "Temperature too high: " + memory.temperature
  }
}

goal "data_ready" {
  when memory.processed_data and memory.processed_data.length > 0
  then {
    action: "notify_user",
    data: memory.processed_data
  }
}
```

### Goal Structure

- **`when`**: The condition that triggers the goal
- **`then`**: The action to execute when triggered

### Goal Characteristics

- **Reactive**: Automatically trigger when conditions are met
- **Declarative**: Describe what should happen, not how
- **Autonomous**: Agents can have multiple goals running simultaneously
- **Persistent**: Goals remain active until conditions change

## 📡 Signals and Communication

### Inter-Agent Communication

Agents communicate through **signals**:

```gx
// Emitting signals
reflect {
  if memory.success {
    emit "operation_complete"
  } else {
    emit "operation_failed" with { error: memory.error }
  }
}

// Handling signals
signal "operation_complete" {
  do {
    assign.agent("notifier").notify_user("Operation completed successfully")
    log("Operation complete signal received")
  }
}

signal "operation_failed" {
  do {
    assign.agent("error_handler").handle_error(memory.error)
    emit "system_error"
  }
}
```

### Signal Types

1. **Simple Signals**: `emit "signal_name"`
2. **Signals with Data**: `emit "signal_name" with { data: value }`
3. **Signal Handlers**: `signal "signal_name" { do { ... } }`

### Assignment Communication

Direct delegation to other agents:

```gx
act {
  if plan.action == "validate_data" {
    assign.agent("validator").validate(plan.data)
  }
  
  if plan.action == "send_notification" {
    assign.agent("notifier").send_email(plan.email, plan.message)
  }
}
```

## 🎨 Capabilities

### Agent Capabilities

**Capabilities** declare what an agent can do:

```gx
agent "data_processor" {
  capabilities: ["data_processing", "api_calls", "file_handling"]
  
  // ... rest of agent definition
}
```

### Capability Types

- **Core Capabilities**: Built into the language
- **Standard Library**: Provided by GX stdlib
- **Custom Capabilities**: Defined by developers
- **External Capabilities**: From external libraries

### Common Capabilities

- `render`: UI rendering capabilities
- `api_calls`: HTTP/API communication
- `file_handling`: File system operations
- `data_processing`: Data transformation
- `communication`: Inter-agent messaging
- `validation`: Input validation
- `notification`: User notifications

## 🔄 Control Flow

### Conditional Logic

GX uses `if/else` for conditional execution:

```gx
act {
  if plan.action == "validate" {
    memory.valid = validate(plan.data)
  } else if plan.action == "process" {
    memory.result = process(plan.data)
  } else {
    memory.error = "Unknown action: " + plan.action
  }
}
```

### Iteration

Use `for each` to iterate over collections:

```gx
act {
  for each item in memory.items {
    memory.processed_items.push(process(item))
  }
  
  for each user in memory.users {
    assign.agent("notifier").notify_user(user)
  }
}
```

### Goal-Based Control Flow

Instead of traditional loops, use goals for reactive programming:

```gx
goal "process_remaining_items" {
  when memory.items.length > 0 and memory.status == "ready"
  then {
    action: "process_next_item",
    item: memory.items.shift()
  }
}
```

## 🎨 Input and Output

### Input Channels

Agents receive data through **input channels**:

```gx
input {
  channel "user_input" {
    source: "user_interface"
    type: "form_data"
    bind: memory.user_data
    on_receive: mental.process_user_input
  }
  
  channel "api_data" {
    source: "external_api"
    type: "json_data"
    bind: memory.api_response
    on_receive: mental.process_api_data
  }
}
```

### Data Binding

Connect input data to memory:

```gx
channel "form_data" {
  source: "web_form"
  bind: memory.form_input
  on_receive: mental.process_form
}
```

### Output and Rendering

Agents can render UI components:

```gx
render "dashboard" {
  type: "chart"
  library: "gxchart"
  style: {
    theme: "dark",
    layout: "grid"
  }
}

view "main_page" {
  layout: "column"
  components: ["dashboard"]
  bind {
    dashboard.data ↔ memory.analytics_data
  }
}
```

## 🔧 Functions and Reusability

### Function Definition

Create reusable functions within agents:

```gx
function "calculate_average" {
  input: numbers_array
  output: average_value
  
  mental {
    think {
      plan = {
        action: "calculate",
        numbers: numbers_array
      }
    }
    
    act {
      if plan.action == "calculate" {
        average_value = sum(plan.numbers) / plan.numbers.length
      }
    }
    
    save {
      memory.last_calculation = average_value
    }
    
    reflect {
      log("Average calculated: " + average_value)
    }
  }
}
```

### Function Characteristics

- **Input/Output**: Functions can have parameters and return values
- **Mental Cycles**: Functions follow the same think → act → save → reflect pattern
- **Reusable**: Can be called multiple times
- **Encapsulated**: Have their own mental state during execution

## 🏗️ Multi-Agent Systems

### Agent Composition

Complex systems are built by composing multiple agents:

```gx
// Orchestrator agent
agent "main_orchestrator" {
  mental {
    think {
      plan = {
        action: "coordinate_workflow",
        steps: ["validate", "process", "notify"]
      }
    }
    
    act {
      if plan.action == "coordinate_workflow" {
        assign.agent("validator").validate(memory.input_data)
      }
    }
  }
  
  signal "validation_complete" {
    do {
      assign.agent("processor").process(memory.validated_data)
    }
  }
  
  signal "processing_complete" {
    do {
      assign.agent("notifier").notify_user(memory.processed_data)
    }
  }
}

// Specialized agents
agent "validator" {
  capabilities: ["validation"]
  
  mental {
    think { plan = { action: "validate_input" } }
    act { memory.valid = validate(plan.data) }
    save { memory.validation_result = memory.valid }
    reflect { if memory.valid { emit "validation_complete" } }
  }
}
```

### Agent Communication Patterns

1. **Request-Response**: Direct assignment with return values
2. **Event-Driven**: Signal emission and handling
3. **Goal-Based**: Conditional triggers based on state
4. **Pipeline**: Sequential processing through multiple agents

## 🎯 Best Practices

### Agent Design

1. **Single Responsibility**: Each agent should have one clear purpose
2. **Meaningful Names**: Use descriptive agent and variable names
3. **Focused Mental Cycles**: Keep think/act/save/reflect focused
4. **Clear Communication**: Use meaningful signals and goals

### Memory Management

1. **Structured Data**: Use objects and arrays for complex data
2. **Descriptive Names**: Make memory variables self-documenting
3. **Minimal Updates**: Only update memory when necessary
4. **Consistent Patterns**: Use consistent naming and structure

### Error Handling

1. **Graceful Degradation**: Handle errors in act blocks
2. **Error Signals**: Emit error signals in reflect blocks
3. **Error Agents**: Create specialized error handling agents
4. **Logging**: Log important events in reflect blocks

### Performance

1. **Efficient Mental Cycles**: Minimize work in think/reflect phases
2. **Smart Caching**: Cache frequently accessed data in memory
3. **Lazy Loading**: Only load data when needed
4. **Parallel Processing**: Use multiple agents for independent work

---

**GX Core Concepts**  
*Version: 0.1.0*  
*Last Updated: 2024* 