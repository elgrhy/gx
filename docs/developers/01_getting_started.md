# 🚀 Getting Started with GX Language

## Welcome to GX!

GX is the world's first brain-first programming language. Instead of traditional programming paradigms, GX models cognitive processes using the **Plan → Execute → Remember → Communicate** cycle.

## Your First GX Program

Let's start with the simplest possible GX program:

```gx
helper "hello_world" {
  can_do: ["greeting"]
  
  remember {
    message = "Hello, GX World!"
  }

  brain {
    plan {
      plan = { action: "display_greeting" }
    }
    
    execute {
      if plan.action == "display_greeting" {
        output(memory.message)
      }
    }
    
    communicate {
      broadcast "greeting_complete"
    }
  }
}
```

## Understanding the Structure

### 1. Helper Declaration
```gx
helper "helper_name" {
  // Helper content goes here
}
```

A **helper** is like a function or class in other languages, but with cognitive capabilities.

### 2. Capabilities
```gx
can_do: ["capability1", "capability2"]
```

This defines what the helper can do. Think of it as a list of skills.

### 3. Memory (Variables)
```gx
remember {
  // Variables go here
  name = "John"
  age = 25
  is_active = true
  scores = [85, 92, 78]
  config = {
    theme: "dark",
    language: "en"
  }
}
```

The `remember` block is where you store variables and data. This is the helper's memory.

### 4. Brain Process
```gx
brain {
  plan { /* Planning phase */ }
  execute { /* Execution phase */ }
  remember { /* Memory update */ }
  communicate { /* Communication phase */ }
}
```

This is the cognitive cycle that every helper follows.

## Working with Variables

### Basic Variables

```gx
helper "variable_examples" {
  can_do: ["variable_demo"]
  
  remember {
    // String variables
    name = "Alice"
    greeting = "Hello, " + memory.name + "!"
    
    // Number variables
    age = 30
    temperature = 98.6
    count = 0
    
    // Boolean variables
    is_logged_in = true
    has_permission = false
    
    // Array variables
    colors = ["red", "green", "blue"]
    numbers = [1, 2, 3, 4, 5]
    
    // Object variables
    user = {
      name: "Bob",
      email: "bob@example.com",
      preferences: {
        theme: "dark",
        notifications: true
      }
    }
  }

  brain {
    plan {
      plan = { action: "demonstrate_variables" }
    }
    
    execute {
      if plan.action == "demonstrate_variables" {
        // Access variables from memory
        output("Name: " + memory.name)
        output("Age: " + memory.age)
        output("Is logged in: " + memory.is_logged_in)
        output("Colors: " + memory.colors)
        output("User email: " + memory.user.email)
      }
    }
    
    communicate {
      broadcast "variables_demonstrated"
    }
  }
}
```

### Variable Operations

```gx
helper "variable_operations" {
  can_do: ["math_operations"]
  
  remember {
    a = 10
    b = 5
    result = 0
  }

  brain {
    plan {
      plan = { action: "perform_operations" }
    }
    
    execute {
      if plan.action == "perform_operations" {
        // Addition
        memory.result = memory.a + memory.b
        output("Addition: " + memory.result)
        
        // Subtraction
        memory.result = memory.a - memory.b
        output("Subtraction: " + memory.result)
        
        // Multiplication
        memory.result = memory.a * memory.b
        output("Multiplication: " + memory.result)
        
        // Division
        memory.result = memory.a / memory.b
        output("Division: " + memory.result)
        
        // String concatenation
        first_name = "John"
        last_name = "Doe"
        full_name = first_name + " " + last_name
        output("Full name: " + full_name)
      }
    }
    
    communicate {
      broadcast "operations_complete"
    }
  }
}
```

## Conditional Logic

```gx
helper "conditional_examples" {
  can_do: ["conditional_demo"]
  
  remember {
    age = 18
    temperature = 75
    is_raining = false
  }

  brain {
    plan {
      plan = { action: "check_conditions" }
    }
    
    execute {
      if plan.action == "check_conditions" {
        // Simple if statement
        if memory.age >= 18 {
          output("You are an adult")
        } else {
          output("You are a minor")
        }
        
        // Multiple conditions
        if memory.temperature > 80 {
          output("It's hot outside")
        } else if memory.temperature > 60 {
          output("It's nice outside")
        } else {
          output("It's cold outside")
        }
        
        // Complex conditions
        if memory.age >= 18 && memory.temperature > 70 && !memory.is_raining {
          output("Perfect day for outdoor activities")
        } else {
          output("Maybe stay inside today")
        }
      }
    }
    
    communicate {
      broadcast "conditions_checked"
    }
  }
}
```

## Loops and Iteration

```gx
helper "loop_examples" {
  can_do: ["loop_demo"]
  
  remember {
    numbers = [1, 2, 3, 4, 5]
    names = ["Alice", "Bob", "Charlie"]
    count = 0
  }

  brain {
    plan {
      plan = { action: "demonstrate_loops" }
    }
    
    execute {
      if plan.action == "demonstrate_loops" {
        // For loop with range
        for i in range(5) {
          output("Count: " + i)
        }
        
        // For each loop
        for each number in memory.numbers {
          output("Number: " + number)
        }
        
        // While loop
        while memory.count < 3 {
          output("While loop iteration: " + memory.count)
          memory.count += 1
        }
        
        // Loop with break
        for each name in memory.names {
          if name == "Bob" {
            output("Found Bob!")
            break
          }
          output("Checking: " + name)
        }
      }
    }
    
    communicate {
      broadcast "loops_demonstrated"
    }
  }
}
```

## Running Your First Program

1. **Save your code** to a file with `.gx` extension:
   ```bash
   # Create your first program
   echo 'helper "hello_world" {
     can_do: ["greeting"]
     remember { message = "Hello, GX World!" }
     brain {
       plan { plan = { action: "display_greeting" } }
       execute { if plan.action == "display_greeting" { output(memory.message) } }
       communicate { broadcast "greeting_complete" }
     }
   }' > hello_world.gx
   ```

2. **Run your program**:
   ```bash
   ./bin/gx hello_world.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: hello_world.gx
     📊 File size: 245 bytes
   
     🚀 Executing GX Runtime: hello_world.gx
     🧠 Initializing cognitive runtime...
     📊 Found 1 helpers with 3 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     Hello, GX World!
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Next Steps

Now that you understand the basics, you're ready to:
- [Learn about Recipes (Functions)](02_recipes_and_functions.md)
- [Explore Message Communication](03_message_communication.md)
- [Build Interactive Applications](04_interactive_applications.md)

## Practice Exercises

1. **Create a calculator helper** that can add, subtract, multiply, and divide two numbers
2. **Build a temperature converter** that converts between Celsius and Fahrenheit
3. **Make a simple counter** that increments and displays the count
4. **Create a list processor** that finds the maximum and minimum values in an array

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 