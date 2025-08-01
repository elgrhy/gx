# 📖 GX Keywords Reference

Complete reference for all GX language keywords, operators, and syntax elements.

## 🧠 Core Keywords

### `agent`
**Definition**: Declares a cognitive agent with its own memory, mental processes, and capabilities.

**Syntax**:
```gx
agent "agent_name" {
  capabilities: ["capability1", "capability2"]
  memory { /* variables */ }
  mental { /* cognitive cycle */ }
  goal "goal_name" { /* conditional logic */ }
  signal "signal_name" { /* signal handlers */ }
}
```

**Example**:
```gx
agent "calculator" {
  capabilities: ["math", "computation"]
  
  memory {
    result = 0
    numbers = [1, 2, 3, 4, 5]
  }
  
  mental {
    think { plan = { action: "calculate_sum" } }
    act { memory.result = sum(memory.numbers) }
    save { memory.last_calculation = memory.result }
    reflect { log("Sum calculated: " + memory.result) }
  }
}
```

### `mental`
**Definition**: Defines the cognitive cycle of an agent (Think → Act → Save → Reflect).

**Syntax**:
```gx
mental {
  think { /* analysis and planning */ }
  act { /* execution */ }
  save { /* state persistence */ }
  reflect { /* evaluation and signaling */ }
}
```

**Example**:
```gx
mental {
  think {
    plan = {
      action: "process_data",
      data: memory.input_data
    }
  }
  
  act {
    if plan.action == "process_data" {
      memory.processed = process(plan.data)
    }
  }
  
  save {
    memory.last_processed = get_timestamp()
  }
  
  reflect {
    if memory.processed {
      emit "data_ready"
    }
  }
}
```

### `think`
**Definition**: The analysis and planning phase of the mental cycle.

**Syntax**:
```gx
think {
  plan = { /* plan object */ }
  analysis = { /* analysis object */ }
}
```

**Example**:
```gx
think {
  plan = {
    action: "validate_input",
    data: memory.user_input,
    priority: "high"
  }
  
  analysis = {
    input_valid: validate(memory.user_input),
    required_fields: ["name", "email"]
  }
}
```

### `act`
**Definition**: The execution phase of the mental cycle.

**Syntax**:
```gx
act {
  if condition { /* action */ }
  else { /* alternative action */ }
}
```

**Example**:
```gx
act {
  if plan.action == "validate_input" {
    if plan.analysis.input_valid {
      assign.agent("processor").process(plan.data)
    } else {
      assign.agent("error_handler").handle_invalid_input(plan.data)
    }
  }
}
```

### `save`
**Definition**: The state persistence phase of the mental cycle.

**Syntax**:
```gx
save {
  memory.variable = value
  memory.object.property = new_value
}
```

**Example**:
```gx
save {
  memory.last_action = plan.action
  memory.attempts += 1
  memory.results.push(plan.result)
  memory.timestamp = get_timestamp()
}
```

### `reflect`
**Definition**: The evaluation and signaling phase of the mental cycle.

**Syntax**:
```gx
reflect {
  if condition { emit "signal_name" }
  log("message")
}
```

**Example**:
```gx
reflect {
  if memory.attempts > 3 {
    emit "too_many_attempts"
  }
  
  if memory.success {
    emit "operation_complete"
  }
  
  log("Operation completed with " + memory.attempts + " attempts")
}
```

## 🧠 Memory & Variables

### `memory`
**Definition**: Declares the persistent state storage for an agent.

**Syntax**:
```gx
memory {
  variable_name = value
  object_name = { property: value }
  array_name = [item1, item2, item3]
}
```

**Example**:
```gx
memory {
  status = "idle"
  counter = 0
  user_data = {
    name: "Alice",
    email: "alice@example.com",
    preferences: ["dark_theme", "notifications"]
  }
  history = []
  settings = {
    theme: "dark",
    language: "en",
    notifications: true
  }
}
```

## 🔄 Control Flow

### `if` / `else`
**Definition**: Conditional execution based on boolean expressions.

**Syntax**:
```gx
if condition {
  // code to execute if condition is true
} else {
  // code to execute if condition is false
}
```

**Example**:
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

### `for each`
**Definition**: Iterate over arrays or objects.

**Syntax**:
```gx
for each item in array {
  // code to execute for each item
}
```

**Example**:
```gx
act {
  for each number in memory.numbers {
    memory.sum += number
  }
  
  for each user in memory.users {
    assign.agent("notifier").notify_user(user)
  }
}
```

## 🎯 Goals & Conditions

### `goal`
**Definition**: Declares conditional logic that triggers based on memory state.

**Syntax**:
```gx
goal "goal_name" {
  when condition
  then { action: "action_name", data: value }
}
```

**Example**:
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

### `when`
**Definition**: Condition that triggers a goal.

**Syntax**:
```gx
when memory.variable == value
when memory.temperature > memory.threshold
when memory.status == "ready" and memory.data.length > 0
```

**Example**:
```gx
goal "process_complete" {
  when memory.status == "processing" and memory.result
  then {
    action: "save_result",
    data: memory.result
  }
}
```

### `then`
**Definition**: Action to execute when a goal condition is met.

**Syntax**:
```gx
then {
  action: "action_name",
  data: value,
  target: "target_agent"
}
```

**Example**:
```gx
goal "send_notification" {
  when memory.new_message
  then {
    action: "notify_user",
    message: memory.new_message,
    priority: "high"
  }
}
```

## 📡 Signals & Communication

### `signal`
**Definition**: Declares a signal handler for inter-agent communication.

**Syntax**:
```gx
signal "signal_name" {
  do { /* actions to execute when signal is received */ }
}
```

**Example**:
```gx
signal "data_ready" {
  do {
    assign.agent("processor").process(memory.data)
    log("Data ready signal received")
  }
}

signal "error_occurred" {
  do {
    assign.agent("error_handler").handle_error(memory.error)
    emit "system_error"
  }
}
```

### `emit`
**Definition**: Sends a signal to other agents or the system.

**Syntax**:
```gx
emit "signal_name"
emit "signal_name" with data
```

**Example**:
```gx
reflect {
  if memory.success {
    emit "operation_complete"
  } else {
    emit "operation_failed" with { error: memory.error }
  }
}
```

### `assign.agent`
**Definition**: Delegates work to another agent.

**Syntax**:
```gx
assign.agent("agent_name").function_name(parameters)
assign.agent("agent_name").method_name(data)
```

**Example**:
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

## 📥 Input & Channels

### `input`
**Definition**: Declares input channels for receiving data.

**Syntax**:
```gx
input {
  channel "channel_name" {
    source: "source_type"
    type: "data_type"
    bind: memory.variable
    on_receive: mental_function
  }
}
```

**Example**:
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

### `channel`
**Definition**: Defines a specific input channel.

**Syntax**:
```gx
channel "channel_name" {
  source: "source_type"
  type: "data_type"
  bind: memory.variable
  on_receive: mental_function
}
```

**Example**:
```gx
channel "file_upload" {
  source: "file_system"
  type: "file_data"
  bind: memory.uploaded_file
  on_receive: mental.process_file
}
```

### `bind`
**Definition**: Connects input data to memory variables.

**Syntax**:
```gx
bind: memory.variable_name
bind: memory.object.property
```

**Example**:
```gx
channel "form_data" {
  source: "web_form"
  bind: memory.form_input
  on_receive: mental.process_form
}
```

## 🎨 UI & Rendering

### `render`
**Definition**: Declares UI components to be rendered.

**Syntax**:
```gx
render "component_name" {
  type: "component_type"
  library: "ui_library"
  style: { /* styling properties */ }
}
```

**Example**:
```gx
render "user_dashboard" {
  type: "dashboard"
  library: "gxchart"
  style: {
    theme: "dark",
    layout: "grid",
    columns: 3
  }
}
```

### `view`
**Definition**: Declares layout and composition of UI components.

**Syntax**:
```gx
view "view_name" {
  layout: "layout_type"
  components: ["component1", "component2"]
  bind { /* data binding */ }
}
```

**Example**:
```gx
view "main_page" {
  layout: "column"
  components: ["header", "content", "footer"]
  bind {
    content.data ↔ memory.page_data
    header.title ↔ memory.page_title
  }
}
```

## 🔧 System Keywords

### `capabilities`
**Definition**: Declares what an agent can do.

**Syntax**:
```gx
capabilities: ["capability1", "capability2", "capability3"]
```

**Example**:
```gx
agent "data_processor" {
  capabilities: ["data_processing", "api_calls", "file_handling"]
  // ... rest of agent definition
}
```

### `function`
**Definition**: Declares a reusable function within an agent.

**Syntax**:
```gx
function "function_name" {
  input: parameter_name
  output: return_value
  
  mental {
    think { /* function logic */ }
    act { /* function execution */ }
    save { /* function state */ }
    reflect { /* function cleanup */ }
  }
}
```

**Example**:
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

## 📝 Comments

### `//` (Single-line comments)
**Definition**: Single-line comments for code documentation.

**Syntax**:
```gx
// This is a single-line comment
memory {
  status = "idle"  // Initialize status
  counter = 0      // Reset counter
}
```

### `/* */` (Multi-line comments)
**Definition**: Multi-line comments for longer explanations.

**Syntax**:
```gx
/*
  This is a multi-line comment
  that can span multiple lines
  for longer explanations
*/
```

## 🔗 Operators

### Assignment Operators
- `=` - Assign value to variable
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

## 📚 Best Practices

1. **Use descriptive names** for agents, variables, and functions
2. **Keep mental cycles focused** on a single responsibility
3. **Log important events** in reflect blocks
4. **Use meaningful signals** for inter-agent communication
5. **Structure complex logic** into multiple agents
6. **Document your code** with comments
7. **Test your mental cycles** thoroughly

---

*GX Keywords Reference*  
*Version: 0.1.0*  
*Last Updated: 2024* 