# 🔧 GX Installation Guide

Complete guide for setting up your GX development environment.

## 🎯 Prerequisites

### System Requirements

- **Operating System**: macOS, Linux, or Windows
- **Architecture**: x86_64, ARM64, or RISC-V
- **Memory**: Minimum 2GB RAM (4GB+ recommended)
- **Storage**: 1GB free space for GX system and tools

### Required Tools

- **Git**: For cloning the repository
- **Assembly Compiler**: NASM (x86), GAS (ARM), or equivalent
- **Text Editor**: VS Code, Vim, or any code editor
- **Terminal**: Command line interface

## 🚀 Installation Methods

### Method 1: From Source (Recommended)

#### Step 1: Clone the Repository

```bash
git clone https://github.com/elgrhy/gx.git
cd gx
```

#### Step 2: Install Assembly Compiler

**For macOS:**
```bash
# Install NASM for x86_64
brew install nasm

# Install ARM toolchain
xcode-select --install
```

**For Ubuntu/Debian:**
```bash
# Install NASM
sudo apt-get update
sudo apt-get install nasm

# Install ARM toolchain
sudo apt-get install gcc-aarch64-linux-gnu
```

**For Windows:**
```bash
# Install NASM via Chocolatey
choco install nasm

# Or download from https://www.nasm.us/
```

#### Step 3: Build the Bootstrapper

**For x86_64:**
```bash
nasm -f bin gx.seed.asm -o gx_boot.bin
```

**For ARM64:**
```bash
as -o gx_boot.o gx.seed.asm
ld -o gx_boot gx_boot.o
```

**For RISC-V:**
```bash
riscv64-unknown-elf-as -o gx_boot.o gx.seed.asm
riscv64-unknown-elf-ld -o gx_boot gx_boot.o
```

#### Step 4: Verify Installation

```bash
# Check if bootstrapper was created
ls -la gx_boot*

# Test the bootstrapper (optional)
qemu-system-x86_64 -fda gx_boot.bin
```

### Method 2: Using Package Manager (Future)

*Note: Package managers will be available in future releases*

```bash
# Using Homebrew (macOS)
brew install gx-language

# Using apt (Ubuntu/Debian)
sudo apt-get install gx-language

# Using Chocolatey (Windows)
choco install gx-language
```

## 🔧 Development Environment Setup

### 1. Install Development Tools

#### VS Code Setup (Recommended)

1. **Install VS Code**: Download from https://code.visualstudio.com/
2. **Install Extensions**:
   - GX Language Extension (when available)
   - Git Integration
   - Terminal Integration

#### Alternative Editors

- **Vim/Neovim**: With syntax highlighting
- **Sublime Text**: With GX syntax support
- **Atom**: With language packages

### 2. Configure Environment Variables

Create a `.env` file in your project root:

```bash
# GX Environment Configuration
GX_VERSION=0.1.0
GX_ENV=development
GX_DEBUG=true
GX_LOG_LEVEL=info

# Development Settings
GX_DEV_MODE=true
GX_HOT_RELOAD=true
GX_AUTO_COMPILE=true

# Paths
GX_HOME=/usr/local/gx
GX_BIN=/usr/local/gx/bin
GX_LIB=/usr/local/gx/lib
```

### 3. Set Up Project Structure

```bash
# Create project directories
mkdir -p my_gx_project
cd my_gx_project

# Initialize GX project
gx init

# Create your first agent
touch main.gx
```

## 🧪 Testing Your Installation

### 1. Create a Test Agent

Create a file called `test.gx`:

```gx
agent "hello_world" {
  capabilities: ["render", "communication"]
  
  memory {
    message = "Hello, GX World!"
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
      emit "test_complete"
    }
  }
}
```

### 2. Run the Test

```bash
# Run the test agent
gx run test.gx

# Expected output:
# [INFO] GX Runtime initialized
# [INFO] Agent 'hello_world' spawned
# [INFO] Message displayed successfully
# [INFO] Signal 'test_complete' emitted
```

### 3. Verify System Components

```bash
# Check GX kernel
gx kernel --version

# Check parser
gx parse test.gx

# Check runtime
gx runtime --status
```

## 🔧 Configuration

### GX Configuration File

Create `gx.config` in your project root:

```json
{
  "version": "0.1.0",
  "name": "my_gx_project",
  "description": "My first GX project",
  "main": "main.gx",
  "agents": [
    "hello_world",
    "data_processor"
  ],
  "dependencies": {
    "gxchart": "^1.0.0",
    "gxui": "^1.0.0"
  },
  "scripts": {
    "start": "gx run main.gx",
    "dev": "gx run --dev main.gx",
    "build": "gx build main.gx",
    "test": "gx test"
  },
  "settings": {
    "debug": true,
    "log_level": "info",
    "auto_reload": true
  }
}
```

### Package Configuration

Create `package.json` for dependency management:

```json
{
  "name": "my-gx-project",
  "version": "0.1.0",
  "description": "My first GX project",
  "main": "main.gx",
  "scripts": {
    "start": "gx run main.gx",
    "dev": "gx run --dev main.gx",
    "build": "gx build main.gx",
    "test": "gx test"
  },
  "dependencies": {
    "gxchart": "^1.0.0",
    "gxui": "^1.0.0",
    "gxnet": "^1.0.0",
    "gxdb": "^1.0.0"
  },
  "devDependencies": {
    "gx-test": "^1.0.0"
  }
}
```

## 🚀 Quick Start Commands

### Basic Commands

```bash
# Initialize a new GX project
gx init my_project

# Run a GX file
gx run main.gx

# Run in development mode
gx run --dev main.gx

# Build for production
gx build main.gx

# Test your agents
gx test

# Parse and validate syntax
gx parse main.gx

# Show runtime status
gx status
```

### Development Commands

```bash
# Watch for changes and auto-reload
gx run --watch main.gx

# Run with debug logging
gx run --debug main.gx

# Profile performance
gx run --profile main.gx

# Generate documentation
gx docs generate

# Format code
gx format main.gx
```

## 🔧 Troubleshooting

### Common Issues

#### 1. Assembly Compiler Not Found

**Error**: `nasm: command not found`

**Solution**:
```bash
# macOS
brew install nasm

# Ubuntu/Debian
sudo apt-get install nasm

# Windows
choco install nasm
```

#### 2. Permission Denied

**Error**: `Permission denied: gx_boot.bin`

**Solution**:
```bash
chmod +x gx_boot.bin
```

#### 3. Architecture Mismatch

**Error**: `Invalid architecture`

**Solution**: Ensure you're using the correct assembly compiler for your target architecture:

```bash
# Check your architecture
uname -m

# Use appropriate compiler
# x86_64: nasm
# ARM64: as (GNU Assembler)
# RISC-V: riscv64-unknown-elf-as
```

#### 4. GX Runtime Not Found

**Error**: `gx: command not found`

**Solution**:
```bash
# Add GX to PATH
export PATH=$PATH:/usr/local/gx/bin

# Or create a symlink
sudo ln -s $(pwd)/gx_boot /usr/local/bin/gx
```

### Debug Mode

Enable debug mode for detailed logging:

```bash
# Set debug environment variable
export GX_DEBUG=true

# Run with debug output
gx run --debug main.gx
```

### Getting Help

```bash
# Show help
gx --help

# Show version
gx --version

# Show available commands
gx help

# Get help for specific command
gx run --help
```

## 🔧 Advanced Setup

### Multi-Architecture Development

For cross-platform development:

```bash
# Install toolchains for multiple architectures
# x86_64
sudo apt-get install nasm

# ARM64
sudo apt-get install gcc-aarch64-linux-gnu

# RISC-V
sudo apt-get install gcc-riscv64-linux-gnu
```

### Docker Development Environment

Create a `Dockerfile` for consistent development:

```dockerfile
FROM ubuntu:20.04

# Install dependencies
RUN apt-get update && apt-get install -y \
    nasm \
    gcc-aarch64-linux-gnu \
    git \
    make

# Clone GX
RUN git clone https://github.com/elgrhy/gx.git /opt/gx

# Build GX
WORKDIR /opt/gx
RUN make

# Set up environment
ENV PATH="/opt/gx/bin:$PATH"
ENV GX_HOME="/opt/gx"

# Default command
CMD ["gx", "run", "main.gx"]
```

### CI/CD Integration

Example GitHub Actions workflow:

```yaml
name: GX CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v2
    
    - name: Install dependencies
      run: |
        sudo apt-get update
        sudo apt-get install -y nasm
    
    - name: Build GX
      run: |
        nasm -f bin gx.seed.asm -o gx_boot.bin
    
    - name: Run tests
      run: |
        gx test
    
    - name: Build application
      run: |
        gx build main.gx
```

## 🎯 Next Steps

After installation:

1. **Read the [Quick Start Guide](quickstart.md)** - Build your first agent
2. **Explore [Examples](examples.md)** - See real applications
3. **Learn the [Keywords Reference](keywords.md)** - Master the language
4. **Join the community** - Contribute and learn together

---

**GX Installation Guide**  
*Version: 0.1.0*  
*Last Updated: 2024* 