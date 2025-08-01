# 📊 GX Data Types

Complete reference for all data types supported in the GX language.

## 🎯 Overview

GX supports a rich set of data types designed for cognitive programming, from simple primitives to complex mental structures.

## 🔤 Primitive Types

### String

Text data enclosed in quotes.

```gx
memory {
  name = "Alice"
  message = "Hello, Cognitive World!"
  empty_string = ""
}
```

**Operations**:
- Concatenation: `"Hello" + " " + "World"`
- Length: `memory.name.length`
- Contains: `memory.message.contains("Cognitive")`

### Number

Numeric values (integers and floating-point).

```gx
memory {
  age = 25
  temperature = 98.6
  count = 0
  negative = -42
}
```

**Operations**:
- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Comparison: `>`, `<`, `>=`, `<=`, `==`, `!=`
- Math functions: `sum()`, `average()`, `max()`, `min()`

### Boolean

True/false values.

```gx
memory {
  is_active = true
  is_complete = false
  has_permission = true
}
```

**Operations**:
- Logical: `and`, `or`, `not`
- Comparison: `==`, `!=`

## 📦 Collection Types

### Array

Ordered collections of values.

```gx
memory {
  numbers = [1, 2, 3, 4, 5]
  names = ["Alice", "Bob", "Charlie"]
  mixed = [1, "hello", true, 3.14]
  empty_array = []
}
```

**Operations**:
- Access: `memory.numbers[0]`
- Length: `memory.numbers.length`
- Push: `memory.numbers.push(6)`
- Pop: `memory.numbers.pop()`
- Iteration: `for each item in memory.numbers`

### Object

Key-value pairs for structured data.

```gx
memory {
  user = {
    name: "Alice",
    age: 30,
    email: "alice@example.com",
    preferences: ["dark_theme", "notifications"]
  }
  
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

**Operations**:
- Access: `memory.user.name`
- Nested access: `memory.settings.notifications.email`
- Set: `memory.user.age = 31`
- Keys: `memory.user.keys()`
- Values: `memory.user.values()`

## 🧠 Mental Types

### Plan

The output of the think phase, containing action and data.

```gx
think {
  plan = {
    action: "process_data",
    data: memory.input_data,
    priority: "high",
    timestamp: get_timestamp()
  }
}
```

**Structure**:
- `action`: String describing what to do
- `data`: Any data needed for the action
- `priority`: String indicating urgency
- Additional properties as needed

### Analysis

Evaluation results from the think phase.

```gx
think {
  analysis = {
    input_valid: validate(memory.user_input),
    required_fields: ["name", "email"],
    missing_fields: get_missing_fields(memory.user_input),
    confidence: 0.95
  }
}
```

**Common Properties**:
- Validation results
- Required/missing fields
- Confidence scores
- Error conditions

### Action

Structured action definitions.

```gx
act {
  if plan.action == "send_notification" {
    action = {
      type: "email",
      recipient: plan.data.recipient,
      subject: plan.data.subject,
      body: plan.data.body
    }
  }
}
```

**Structure**:
- `type`: String indicating action type
- `target`: Who/what to act on
- `parameters`: Action-specific data

## 📡 Communication Types

### Signal

Messages sent between agents.

```gx
reflect {
  emit "data_ready" with {
    data: memory.processed_data,
    timestamp: get_timestamp(),
    source: "data_processor"
  }
}
```

**Structure**:
- `name`: String signal name
- `data`: Optional data payload
- `timestamp`: When signal was sent
- `source`: Sending agent name

### Goal

Conditional logic definitions.

```gx
goal "high_temperature_alert" {
  when memory.temperature > memory.threshold
  then {
    action: "send_alert",
    message: "Temperature too high: " + memory.temperature,
    priority: "high"
  }
}
```

**Structure**:
- `when`: Boolean condition
- `then`: Action to execute
- `action`: String action name
- `data`: Action parameters

## 🎨 UI Types

### Render Component

UI component definitions.

```gx
render "dashboard" {
  type: "chart"
  library: "gxchart"
  style: {
    theme: "dark",
    layout: "grid",
    columns: 3,
    padding: "lg"
  }
  data: memory.analytics_data
}
```

**Properties**:
- `type`: Component type (chart, form, table, etc.)
- `library`: UI library to use
- `style`: Visual styling properties
- `data`: Component data

### View Layout

Layout and composition definitions.

```gx
view "main_page" {
  layout: "column"
  components: ["header", "content", "footer"]
  bind {
    content.data ↔ memory.page_data
    header.title ↔ memory.page_title
  }
  style: {
    theme: "light",
    responsive: true
  }
}
```

**Properties**:
- `layout`: Layout type (column, row, grid)
- `components`: Array of component names
- `bind`: Data binding definitions
- `style`: Layout styling

## 🔧 System Types

### Memory Block

Complete agent memory structure.

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

### Capability Array

Agent capability definitions.

```gx
capabilities: ["data_processing", "api_calls", "file_handling", "render"]
```

## 🔄 Type Operations

### Type Checking

```gx
mental {
  think {
    if typeof(memory.data) == "string" {
      plan = { action: "process_string" }
    } else if typeof(memory.data) == "array" {
      plan = { action: "process_array" }
    } else if typeof(memory.data) == "object" {
      plan = { action: "process_object" }
    }
  }
}
```

### Type Conversion

```gx
act {
  // String to number
  memory.number = parseInt(memory.string_number)
  
  // Number to string
  memory.string = toString(memory.number)
  
  // Array to string
  memory.array_string = memory.array.join(", ")
  
  // String to array
  memory.string_array = memory.string.split(", ")
}
```

### Type Validation

```gx
function "validate_data" {
  input: data
  output: is_valid
  
  mental {
    think {
      plan = {
        action: "validate",
        data: data
      }
    }
    
    act {
      if plan.action == "validate" {
        is_valid = validate_type(plan.data, "object") and
                   validate_property(plan.data, "name", "string") and
                   validate_property(plan.data, "age", "number")
      }
    }
  }
}
```

## 📊 Data Type Examples

### User Profile

```gx
memory {
  user_profile = {
    id: 12345,
    name: "Alice Johnson",
    email: "alice@example.com",
    age: 30,
    is_active: true,
    preferences: {
      theme: "dark",
      language: "en",
      notifications: {
        email: true,
        push: false,
        sms: true
      }
    },
    tags: ["developer", "admin", "premium"],
    metadata: {
      created_at: "2024-01-15T10:30:00Z",
      last_login: "2024-01-20T14:45:00Z",
      login_count: 42
    }
  }
}
```

### API Response

```gx
memory {
  api_response = {
    status: "success",
    data: {
      users: [
        {
          id: 1,
          name: "Alice",
          email: "alice@example.com"
        },
        {
          id: 2,
          name: "Bob",
          email: "bob@example.com"
        }
      ],
      pagination: {
        page: 1,
        per_page: 10,
        total: 25
      }
    },
    timestamp: "2024-01-20T15:30:00Z"
  }
}
```

### Form Data

```gx
memory {
  form_data = {
    fields: {
      name: {
        value: "Alice Johnson",
        valid: true,
        error: null
      },
      email: {
        value: "alice@example.com",
        valid: true,
        error: null
      },
      age: {
        value: 30,
        valid: true,
        error: null
      }
    },
    is_valid: true,
    is_submitted: false
  }
}
```

## 🎯 Best Practices

### Type Safety

1. **Validate Input**: Check data types before processing
2. **Use Descriptive Names**: Make variable types clear from names
3. **Document Types**: Comment complex data structures
4. **Handle Errors**: Provide fallbacks for type mismatches

### Memory Organization

1. **Group Related Data**: Use objects for related variables
2. **Use Arrays for Lists**: Store collections in arrays
3. **Nest Appropriately**: Use nested objects for complex data
4. **Keep It Simple**: Avoid overly complex nested structures

### Performance

1. **Choose Efficient Types**: Use appropriate types for data
2. **Minimize Conversions**: Avoid unnecessary type conversions
3. **Cache Results**: Store computed values in memory
4. **Use Arrays Wisely**: Arrays are good for iteration

---

**GX Data Types**  
*Version: 0.1.0*  
*Last Updated: 2024* 