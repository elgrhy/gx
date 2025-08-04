# 🛠️ GX Language Developer Guide

## 📋 Table of Contents

1. [Getting Started](#getting-started)
2. [Development Environment](#development-environment)
3. [Language Reference](#language-reference)
4. [Architecture Overview](#architecture-overview)
5. [Testing Guide](#testing-guide)
6. [Contributing Guidelines](#contributing-guidelines)
7. [Debugging](#debugging)
8. [Performance Optimization](#performance-optimization)
9. [Deployment](#deployment)

---

## 🚀 Getting Started

### Prerequisites

- **GCC** (GNU Compiler Collection) 7.0 or higher
- **NASM** (Netwide Assembler) for x86 builds
- **GNU Binutils** for ARM64/RISC-V builds
- **Git** for version control
- **Make** for build automation

### Installation

```bash
# Clone the repository
git clone https://github.com/gx-language/gx.git
cd gx

# Build the complete system
./build.sh

# Verify installation
./bin/gx --version
```

### First Steps

1. **Create your first GX program**:
```gx
helper "my_first_helper" {
  can_do: ["greeting"]
  
  remember {
    message = "Hello from GX!"
  }

  brain {
    plan {
      plan = { action: "display_message" }
    }
    
    execute {
      if plan.action == "display_message" {
        output(memory.message)
      }
    }
    
    communicate {
      broadcast "greeting_sent"
    }
  }
}
```

2. **Run your program**:
```bash
./bin/gx my_first_helper.gx
```

---

## 🔧 Development Environment

### Project Structure

```
gx/
├── bin/                    # Compiled binaries
├── build/                  # Build artifacts
├── docs/                   # Documentation
│   ├── reports/           # Implementation reports
│   └── plans/             # Development plans
├── examples/              # Example programs
├── tests/                 # Test suite
├── gx_stdlib/            # Standard library
├── ide/                   # IDE integration
├── *.gx                   # Core GX components
├── build.sh               # Build script
└── README.md             # Project overview
```

### Development Workflow

1. **Make changes** to GX source files
2. **Build** the system: `./build.sh`
3. **Test** your changes: `./tests/run_tests.sh`
4. **Run** examples: `./bin/gx examples/your_example.gx`
5. **Commit** and **push** your changes

### IDE Setup

#### VS Code (Recommended)

1. Install the GX Language extension
2. Configure syntax highlighting
3. Enable IntelliSense for brain processes
4. Set up debugging configuration

#### Vim/Neovim

```vim
" Add to your .vimrc
autocmd BufRead,BufNewFile *.gx set filetype=gx
autocmd FileType gx set syntax=gx
```

#### Emacs

```elisp
;; Add to your .emacs
(add-to-list 'auto-mode-alist '("\\.gx\\'" . gx-mode))
```

---

## 📚 Language Reference

### Core Concepts

#### Helper Definition

```gx
helper "helper_name" {
  can_do: ["capability1", "capability2"]
  
  remember {
    variable1 = "value"
    variable2 = 42
  }

  receive {
    from "source" as "channel_name" {
      type: "data_type"
      bind: memory.variable
      on_receive: brain.handler_function
    }
  }

  brain {
    plan { plan = analyze_input() }
    execute { execute_actions(plan) }
    remember { memory.result = plan.result }
    communicate { broadcast "signal_name" }
  }
}
```

#### Brain Process Cycle

The brain process follows this cycle:

1. **Plan** - Analyze input and create execution plan
2. **Execute** - Perform actions based on plan
3. **Remember** - Persist state and results
4. **Communicate** - Signal completion and share results

#### Memory Management

```gx
remember {
  // Local variables
  local_var = "value"
  
  // Arrays
  data_array = [1, 2, 3, 4]
  
  // Objects
  config = {
    timeout: 5000,
    retries: 3,
    enabled: true
  }
  
  // Nested structures
  complex_data = {
    user: {
      name: "John",
      preferences: {
        theme: "dark",
        language: "en"
      }
    }
  }
}
```

#### Message Communication

```gx
receive {
  from "input_source" as "channel_name" {
    type: "data_type"
    bind: memory.variable
    on_receive: brain.handler_function
  }
}

// Sending messages
communicate {
  broadcast "event_name"
  send_to "target_helper" {
    data: memory.result,
    timestamp: get_timestamp()
  }
}
```

### Advanced Features

#### Recipes (Functions)

```gx
recipe "calculate_sum" {
  needs: numbers
  gives: sum
  
  brain {
    plan {
      plan = { action: "sum_numbers" }
    }
    
    execute {
      if plan.action == "sum_numbers" {
        sum = 0
        for each number in numbers {
          sum += number
        }
      }
    }
  }
}
```

#### Objectives (Conditional Logic)

```gx
objective "check_threshold" {
  when memory.value > 100
  then {
    action: "alert_high_value",
    value: memory.value
  }
}
```

#### Messages (Event Handlers)

```gx
message "data_received" {
  do {
    log("Data received: " + memory.data)
    process_data(memory.data)
  }
}
```

---

## 🏗️ Architecture Overview

### System Components

1. **GX Kernel** (`gx.kernel.gx`) - Self-hosting kernel
2. **GX Parser** (`gx_parser_production.gx`) - Syntax parsing
3. **GX Runtime** (`gx_runtime_production.gx`) - Execution engine
4. **GX Compiler** (`gx_compiler.gx`) - Self-hosting compiler
5. **GX Optimizer** (`gx_optimizer.gx`) - Code optimization
6. **GX Render Engine** (`gx_render_engine.gx`) - UI rendering
7. **DNKN Connector** (`gx_dnkn_connector.gx`) - Distributed mesh
8. **GXOS Kernel** (`gxos_kernel.gx`) - Native OS capabilities

### Data Flow

```
Source Code → Parser → AST → Compiler → Bytecode → Optimizer → Runtime
     ↓
Brain Process → Plan → Execute → Remember → Communicate
     ↓
Distributed Mesh → Knowledge Sharing → Pattern Discovery
```

### Memory Architecture

- **Helper Memory**: Local state for each helper
- **Global Memory**: Shared state across helpers
- **Message Queue**: Inter-helper communication
- **AST Storage**: Parsed program structure

---

## 🧪 Testing Guide

### Running Tests

```bash
# Run all tests
./tests/run_tests.sh

# Run specific test categories
./tests/test_brain_processes.sh
./tests/test_compilation.sh
./tests/test_distributed.sh
./tests/test_optimization.sh
```

### Writing Tests

Create test files in the `tests/` directory:

```gx
// tests/test_basic_helpers.gx
helper "test_helper" {
  can_do: ["testing"]
  
  remember {
    test_results = []
  }

  brain {
    plan {
      plan = { action: "run_tests" }
    }
    
    execute {
      if plan.action == "run_tests" {
        // Test brain process cycle
        test_brain_cycle()
        
        // Test memory management
        test_memory_operations()
        
        // Test message communication
        test_message_passing()
      }
    }
    
    remember {
      memory.test_results.push("all_tests_passed")
    }
    
    communicate {
      broadcast "tests_complete"
    }
  }
}
```

### Test Categories

1. **Unit Tests** - Individual helper testing
2. **Integration Tests** - Helper interaction testing
3. **Brain Process Tests** - Cognitive cycle validation
4. **Compilation Tests** - Parser and compiler testing
5. **Optimization Tests** - Code optimization validation
6. **Distributed Tests** - Mesh networking testing

---

## 🤝 Contributing Guidelines

### Code Style

1. **Helper Naming**: Use descriptive names in snake_case
2. **Brain Process**: Always include all four phases
3. **Memory Variables**: Use clear, descriptive names
4. **Comments**: Add comments for complex logic
5. **Error Handling**: Include proper error handling

### Pull Request Process

1. **Fork** the repository
2. **Create** a feature branch
3. **Make** your changes
4. **Test** thoroughly
5. **Document** your changes
6. **Submit** a pull request

### Commit Message Format

```
type(scope): description

[optional body]

[optional footer]
```

Examples:
```
feat(compiler): add constant folding optimization
fix(runtime): resolve memory leak in helper cleanup
docs(readme): update installation instructions
test(parser): add tests for new syntax features
```

---

## 🐛 Debugging

### Debug Mode

```bash
# Run with debug output
./bin/gx --debug program.gx

# Parse only (no execution)
./bin/gx --parse program.gx

# Verbose logging
./bin/gx --verbose program.gx
```

### Common Issues

#### Brain Process Errors

```gx
// ❌ Missing brain process
helper "broken_helper" {
  can_do: ["test"]
  // Missing brain block
}

// ✅ Correct brain process
helper "working_helper" {
  can_do: ["test"]
  
  brain {
    plan { plan = { action: "test" } }
    execute { if plan.action == "test" { /* action */ } }
    remember { memory.result = "success" }
    communicate { broadcast "test_complete" }
  }
}
```

#### Memory Access Errors

```gx
// ❌ Accessing undefined memory
brain {
  execute {
    result = memory.undefined_variable // Error!
  }
}

// ✅ Proper memory initialization
remember {
  undefined_variable = null
}

brain {
  execute {
    if memory.undefined_variable {
      result = memory.undefined_variable
    }
  }
}
```

#### Message Communication Errors

```gx
// ❌ Sending to non-existent helper
communicate {
  send_to "non_existent_helper" { data: "test" } // Error!
}

// ✅ Check helper existence
communicate {
  if helper_exists("target_helper") {
    send_to "target_helper" { data: "test" }
  }
}
```

### Debugging Tools

1. **Brain Visualizer**: Real-time brain cycle visualization
2. **Memory Inspector**: Live memory state monitoring
3. **Message Tracer**: Inter-helper communication tracking
4. **Performance Profiler**: Execution time analysis

---

## ⚡ Performance Optimization

### Compilation Optimizations

```bash
# Enable aggressive optimization
./bin/gx_compiler --optimize-level=aggressive source.gx

# Profile compilation performance
./bin/gx_compiler --profile source.gx

# Generate optimization report
./bin/gx_optimizer --report bytecode.bin
```

### Runtime Optimizations

1. **Memory Pooling**: Reuse memory allocations
2. **Helper Caching**: Cache frequently used helpers
3. **Message Batching**: Batch multiple messages
4. **Brain Cycle Optimization**: Optimize brain execution

### Performance Best Practices

```gx
// ❌ Inefficient: Creating helpers in loops
brain {
  execute {
    for each item in memory.items {
      spawn_helper("processor") // Expensive!
    }
  }
}

// ✅ Efficient: Reuse existing helpers
brain {
  execute {
    for each item in memory.items {
      send_to "processor_pool" { data: item }
    }
  }
}
```

---

## 🚀 Deployment

### Development Deployment

```bash
# Build for development
./build.sh --dev

# Run with development settings
./bin/gx --dev-mode program.gx
```

### Production Deployment

```bash
# Build for production
./build.sh --production

# Run with production settings
./bin/gx --production-mode program.gx
```

### Container Deployment

```dockerfile
# Dockerfile
FROM ubuntu:20.04

# Install dependencies
RUN apt-get update && apt-get install -y \
    gcc \
    nasm \
    make \
    git

# Copy GX source
COPY . /gx
WORKDIR /gx

# Build GX
RUN ./build.sh

# Set entry point
ENTRYPOINT ["./bin/gx"]
```

### Distributed Deployment

```bash
# Start mesh network
./bin/gx_dnkn --mesh-mode --discovery-enabled

# Join existing mesh
./bin/gx_dnkn --connect-to node1:8080

# Monitor mesh health
./bin/gx_dnkn --health-check
```

---

## 📚 Additional Resources

- **[API Reference](API_REFERENCE.md)** - Complete language reference
- **[Examples](../examples/)** - Sample programs and use cases
- **[Tests](../tests/)** - Test suite and examples
- **[Reports](reports/)** - Implementation and completion reports
- **[Plans](plans/)** - Development and integration plans

---

*This guide is maintained by the GX Development Team. For questions or contributions, please see our [Contributing Guide](../CONTRIBUTING.md).*

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer.