# 🧠 GX Language

**GX** is a **zero-dependency, cognitive-first OS-grade language** built from scratch, designed to implement the **CBX Grammar & Mental Process** (`Think → Act → Save → Reflect`) with full **UCP Compliance** (Agents, Goals, Memory, Autonomy) and **DNKN Integration** (Multi-agent messaging and orchestration).

## 🌟 Vision

GX represents a paradigm shift in programming languages, moving from traditional imperative programming to **cognitive-first programming**. The language is built around the concept of **agents** that think, act, save, and reflect - mirroring human cognitive processes.

## 🚀 Key Features

- **🧠 Cognitive-First**: Every program is structured around mental processes
- **🤖 Agent-Based**: Programs are composed of autonomous agents
- **🔄 Self-Hosting**: The language can compile and run itself
- **⚡ Zero-Dependency**: No external runtime dependencies
- **💻 OS-Grade**: Can run as a complete operating system
- **🌐 Multi-Architecture**: Supports x86, ARM64, and RISC-V

## 📁 Project Structure

```
gx/
├── gx.seed.asm              # Universal bootstrapper
├── gx.kernel.gx             # Core kernel
├── gx_parser.gx             # Syntax parser
├── gx_runtime.gx            # Runtime engine
├── main.gx                  # Main orchestrator
├── gx_stdlib/
│   ├── configurator.gx      # Environment setup
│   └── installer.gx         # Package management
├── examples/
│   └── weather_app.gx       # Weather application
├── .env                     # Environment configuration
├── gx.config               # GX configuration
├── package.json            # Package dependencies
├── README.md               # This file
└── report.md               # Detailed development report
```

## 🧠 Mental Model

Each GX agent follows this cognitive cycle:

```
INPUT → THINK → ACT → SAVE → REFLECT → GOAL → SIGNAL → RESOLVE
```

1. **Input**: Receive data from channels
2. **Think**: Analyze and plan
3. **Act**: Execute actions
4. **Save**: Persist state
5. **Reflect**: Evaluate and emit signals

## 📝 Quick Start

### 1. Basic Agent Example

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

  goal "repeat_message" {
    when memory.counter < 3
    then {
      action: "display_message",
      message: memory.message
    }
  }
}
```

### 2. Multi-Agent Weather System

See `examples/weather_app.gx` for a complete weather monitoring system with multiple collaborating agents.

## 🏗️ Architecture

### System Layers

```
┌──────────────────────────────────┐
│        GX Bootstrap Layer        │
│  (.gx.seed.asm → platform init)  │
└──────────────────────────────────┘
                   │
                   ▼
┌──────────────────────────────────┐
│        GX Kernel Loader          │
│  Parses .gx file and builds AST  │
└──────────────────────────────────┘
                   │
                   ▼
┌──────────────────────────────────┐
│       GX Runtime Engine          │
├──────────────────────────────────┤
│  🧠 Input Controller              │
│  🧠 Mental Process Loop           │
│  🧠 Goal Evaluator                │
│  🧠 Signal Dispatcher             │
│  🧠 Agent Executor                │
│  🧠 Memory State Manager          │
│  🧠 Render Engine (GX UI)         │
│  🧠 DNKN Mesh Connector           │
└──────────────────────────────────┘
                   │
                   ▼
┌──────────────────────────────────┐
│     Multi-Agent Grid Runtime     │
│  (distributed orchestration bus) │
└──────────────────────────────────┘
```

## 🔧 Core Components

### 1. GX Bootstrapper (`gx.seed.asm`)
Universal Assembly seed for platform initialization with multi-architecture support.

### 2. GX Kernel (`gx.kernel.gx`)
Core system initialization and agent management with self-hosting capabilities.

### 3. GX Parser (`gx_parser.gx`)
Parse GX syntax and build Abstract Syntax Tree (AST) for cognitive structures.

### 4. GX Runtime (`gx_runtime.gx`)
Execute mental processes and manage agent lifecycle with Think → Act → Save → Reflect cycles.

### 5. Standard Library
- **Configurator**: Environment setup and dependency resolution
- **Installer**: Package management and version control

## 📚 Language Grammar

### Core Keywords

- `agent`: Define cognitive agents
- `mental`: Mental process blocks
- `think`, `act`, `save`, `reflect`: Mental cycle steps
- `memory`: Persistent state storage
- `input`: Input channel definitions
- `goal`: Conditional logic blocks
- `signal`: Inter-agent communication
- `render`, `view`: UI composition
- `assign.agent`: Task delegation

### Data Types

- **Primitives**: strings, numbers, booleans
- **Collections**: arrays, objects
- **Mental**: plans, actions, reflections
- **Signals**: messages, events

## 🌟 Example Applications

### Weather Application
A complete multi-agent weather monitoring system demonstrating:

- **Weather Orchestrator**: Main coordination agent
- **Weather Fetcher**: API data retrieval
- **Weather Analyzer**: Data analysis and alert generation
- **Alert Sender**: Notification dispatch

See `examples/weather_app.gx` for the complete implementation.

## 🚀 Getting Started

### Prerequisites

- Assembly compiler (NASM, GAS, or equivalent)
- Basic understanding of cognitive programming concepts

### Installation

1. **Clone the repository**
   ```bash
   git clone https://github.com/your-org/gx-language.git
   cd gx-language
   ```

2. **Build the bootstrapper**
   ```bash
   # For x86_64
   nasm -f bin gx.seed.asm -o gx_boot.bin
   
   # For ARM64
   as -o gx_boot.o gx.seed.asm
   ld -o gx_boot gx_boot.o
   ```

3. **Run the system**
   ```bash
   # Boot from the generated binary
   qemu-system-x86_64 -fda gx_boot.bin
   ```

### Development

1. **Create a new GX application**
   ```gx
   agent "my_app" {
     mental {
       think { plan = analyze_input() }
       act { execute_plan(plan) }
       save { memory.result = plan.result }
       reflect { emit "completed" }
     }
   }
   ```

2. **Run your application**
   ```bash
   gx run my_app.gx
   ```

## 📊 Development Status

### ✅ Completed
- Core language implementation
- Agent system and mental execution
- Standard library foundation
- Multi-agent orchestration
- Error handling and recovery

### 🚧 In Progress
- UI rendering system
- DNKN integration
- Self-hosting compiler

### 📋 Planned
- Native OS runtime (GXOS)
- Machine learning integration
- Advanced cognitive features

## 🤝 Contributing

We welcome contributions to the GX language! Please see our contributing guidelines for more information.

### Development Areas
- Core language features
- Standard library components
- Documentation and examples
- Testing and validation
- Performance optimization

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.

## 📖 Documentation

- **[Development Report](report.md)**: Comprehensive technical documentation
- **[Examples](examples/)**: Sample applications and use cases
- **[Standard Library](gx_stdlib/)**: Built-in components and utilities

## 🎯 Roadmap

### Phase 1: Foundation ✅
- Core language implementation
- Basic agent system
- Mental execution engine
- Standard library

### Phase 2: Self-Hosting 🔄
- GX compiler in GX
- Bytecode optimization
- Advanced AST manipulation

### Phase 3: Native Runtime 📋
- GXOS kernel implementation
- Hardware abstraction
- Device drivers

### Phase 4: Cognitive Enhancement 📋
- Machine learning agents
- Natural language processing
- Advanced pattern recognition

### Phase 5: Ecosystem 📋
- Package registry
- Development tools
- Community platform

## 🎉 Acknowledgments

GX is inspired by:
- **CBX Grammar**: Cognitive programming model
- **UCP**: Universal Cognitive Protocol
- **DNKN**: Distributed Neural Knowledge Network
- **Self-hosting languages**: Lisp, Forth, and others

---

**GX Language** - *Cognitive-First Programming for the Future*

*Version: 0.1.0*  
*Date: 2024* 