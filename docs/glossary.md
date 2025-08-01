# 📚 GX Language Glossary

Complete definitions of all GX terms, concepts, and terminology.

## 🧠 Core Concepts

### Agent
A cognitive entity in GX that has its own memory, mental processes, and capabilities. Agents are the fundamental building blocks of GX applications, each following the Think → Act → Save → Reflect cycle.

**Example**: `agent "calculator" { /* agent definition */ }`

### Mental Cycle
The four-phase cognitive process that every GX agent follows: **Think** → **Act** → **Save** → **Reflect**. This cycle mirrors human cognitive processes and is the core of GX's cognitive-first programming model.

### Memory
The persistent state storage for an agent. All variables, data, and state are stored in the agent's memory block and accessed via `memory.variable_name`.

**Example**: `memory { status = "idle", counter = 0 }`

### Capabilities
Declarations of what an agent can do. Capabilities help other agents understand what services a particular agent can provide.

**Example**: `capabilities: ["data_processing", "api_calls"]`

## 🧠 Mental Process Keywords

### Think
The first phase of the mental cycle where the agent analyzes the current situation and creates a plan for what to do next.

**Example**:
```gx
think {
  plan = {
    action: "process_data",
    data: memory.input_data
  }
}
```

### Act
The second phase of the mental cycle where the agent executes the plan created in the think phase.

**Example**:
```gx
act {
  if plan.action == "process_data" {
    memory.result = process(plan.data)
  }
}
```

### Save
The third phase of the mental cycle where the agent persists state changes and updates memory.

**Example**:
```gx
save {
  memory.last_action = plan.action
  memory.timestamp = get_timestamp()
}
```

### Reflect
The fourth phase of the mental cycle where the agent evaluates what happened and emits signals or logs information.

**Example**:
```gx
reflect {
  if memory.success {
    emit "operation_complete"
  }
  log("Operation completed")
}
```

## 🎯 Goals & Conditions

### Goal
A conditional logic block that triggers based on memory state. Goals allow agents to respond to specific conditions automatically.

**Example**:
```gx
goal "high_temperature_alert" {
  when memory.temperature > memory.threshold
  then {
    action: "send_alert",
    message: "Temperature too high"
  }
}
```

### When
The condition that triggers a goal. Can be any boolean expression based on memory state.

**Example**: `when memory.status == "ready" and memory.data.length > 0`

### Then
The action to execute when a goal condition is met.

**Example**:
```gx
then {
  action: "notify_user",
  data: memory.processed_data
}
```

## 📡 Communication

### Signal
A message sent between agents or to the system. Signals are the primary mechanism for inter-agent communication.

**Example**:
```gx
signal "data_ready" {
  do {
    assign.agent("processor").process(memory.data)
  }
}
```

### Emit
The action of sending a signal to other agents or the system.

**Example**: `emit "operation_complete"`

### Assign
Delegating work to another agent. The `assign.agent()` syntax allows one agent to request work from another.

**Example**: `assign.agent("validator").validate(plan.data)`

## 📥 Input & Data

### Input Channel
A way for an agent to receive data from external sources like user interfaces, APIs, or other agents.

**Example**:
```gx
input {
  channel "user_input" {
    source: "user_interface"
    bind: memory.user_data
  }
}
```

### Bind
Connecting input data to memory variables. The `bind` keyword links external data to an agent's memory.

**Example**: `bind: memory.form_data`

### Source
The origin of input data (e.g., "user_interface", "api", "file_system").

**Example**: `source: "external_api"`

## 🎨 UI & Rendering

### Render
Declaring UI components to be displayed. The `render` block defines what visual elements an agent can create.

**Example**:
```gx
render "dashboard" {
  type: "chart"
  library: "gxchart"
  style: { theme: "dark" }
}
```

### View
Defining the layout and composition of UI components. Views organize how multiple render components are arranged.

**Example**:
```gx
view "main_page" {
  layout: "column"
  components: ["header", "content", "footer"]
}
```

### Data Binding
Connecting UI components to agent memory for real-time updates. Uses the `↔` operator for two-way binding.

**Example**:
```gx
bind {
  chart.data ↔ memory.analytics_data
  form.fields ↔ memory.user_input
}
```

## 🔧 System Terms

### Function
A reusable block of mental logic within an agent. Functions can have inputs and outputs and follow the same mental cycle.

**Example**:
```gx
function "calculate_average" {
  input: numbers_array
  output: average_value
  
  mental {
    think { plan = { action: "calculate" } }
    act { average_value = sum(numbers_array) / numbers_array.length }
    save { memory.last_calculation = average_value }
    reflect { log("Average calculated") }
  }
}
```

### Capability
A declared ability of an agent. Capabilities help with agent discovery and orchestration.

**Example**: `capabilities: ["data_processing", "api_calls", "file_handling"]`

## 🔄 Control Flow

### If/Else
Conditional execution based on boolean expressions.

**Example**:
```gx
if plan.action == "validate" {
  memory.valid = validate(plan.data)
} else {
  memory.error = "Unknown action"
}
```

### For Each
Iterating over arrays or objects.

**Example**:
```gx
for each item in memory.items {
  memory.processed_items.push(process(item))
}
```

## 📊 Data Types

### Primitive Types
- **String**: Text data (`"Hello, World!"`)
- **Number**: Numeric values (`42`, `3.14`)
- **Boolean**: True/false values (`true`, `false`)

### Complex Types
- **Array**: Ordered collections (`[1, 2, 3, 4]`)
- **Object**: Key-value pairs (`{ name: "Alice", age: 30 }`)
- **Mental**: Plans, actions, reflections (`{ action: "process", data: value }`)

## 🔗 Operators

### Assignment Operators
- `=` - Basic assignment
- `+=` - Add and assign
- `-=` - Subtract and assign
- `*=` - Multiply and assign
- `/=` - Divide and assign

### Comparison Operators
- `==` - Equal to
- `!=` - Not equal to
- `>` - Greater than
- `<` - Less than
- `>=` - Greater than or equal to
- `<=` - Less than or equal to

### Logical Operators
- `and` - Logical AND
- `or` - Logical OR
- `not` - Logical NOT

### Arithmetic Operators
- `+` - Addition
- `-` - Subtraction
- `*` - Multiplication
- `/` - Division
- `%` - Modulo

## 🧠 Cognitive Terms

### Plan
The output of the think phase, containing the action to be executed and any necessary data.

**Example**:
```gx
plan = {
  action: "process_data",
  data: memory.input_data,
  priority: "high"
}
```

### Analysis
The evaluation of current conditions and data, often created in the think phase.

**Example**:
```gx
analysis = {
  input_valid: validate(memory.user_input),
  required_fields: ["name", "email"],
  missing_fields: get_missing_fields(memory.user_input)
}
```

### Mental State
The current cognitive state of an agent, including its plan, analysis, and mental cycle phase.

## 🔧 Runtime Terms

### AST (Abstract Syntax Tree)
The parsed representation of GX code that the runtime engine executes.

### Opcode
Low-level instructions that the GX mental engine executes (e.g., `spawn`, `mental_loop`, `analyze`, `execute`, `persist`, `review`).

### Signal Queue
The system-wide queue of pending signals waiting to be processed by target agents.

### Agent Mesh
The network of interconnected agents that can communicate and collaborate.

## 📚 Documentation Terms

### Cognitive-First Programming
The programming paradigm where code is structured around mental processes rather than traditional imperative logic.

### Self-Hosting
The ability of a language to compile and run itself, as GX does with its kernel written in GX.

### Zero-Dependency
A system that requires no external runtime dependencies, as GX is designed to be completely self-contained.

### OS-Grade
Capable of running as a complete operating system, as GX is designed to be.

## 🎯 Advanced Concepts

### DNKN (Distributed Neural Knowledge Network)
The theoretical framework for distributed agent communication and knowledge sharing in GX.

### UCP (Universal Cognitive Protocol)
The protocol for agent communication, goal management, and cognitive orchestration.

### CBX Grammar
The cognitive grammar that defines the mental processes and structures in GX.

### Mental Engine
The runtime component that executes the Think → Act → Save → Reflect cycles for all agents.

---

*GX Language Glossary*  
*Version: 0.1.0*  
*Last Updated: 2024* 