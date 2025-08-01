# 🤝 Contributing to GX Language

Thank you for your interest in contributing to GX Language! This document provides guidelines and information for contributors.

## 📋 Table of Contents

1. [Getting Started](#getting-started)
2. [Development Setup](#development-setup)
3. [Code Style Guidelines](#code-style-guidelines)
4. [Testing Guidelines](#testing-guidelines)
5. [Pull Request Process](#pull-request-process)
6. [Issue Reporting](#issue-reporting)
7. [Community Guidelines](#community-guidelines)

---

## 🚀 Getting Started

### Prerequisites

Before contributing, ensure you have:

- **GCC** (GNU Compiler Collection) 7.0 or higher
- **NASM** (Netwide Assembler) for x86 builds
- **GNU Binutils** for ARM64/RISC-V builds
- **Git** for version control
- **Make** for build automation

### Quick Setup

```bash
# Fork and clone the repository
git clone https://github.com/your-username/gx.git
cd gx

# Build the system
./build.sh

# Run tests to ensure everything works
./tests/run_tests.sh
```

---

## 🔧 Development Setup

### Environment Setup

1. **Install Dependencies**:
   ```bash
   # macOS
   brew install gcc nasm make
   
   # Ubuntu/Debian
   sudo apt-get install build-essential nasm
   
   # CentOS/RHEL
   sudo yum groupinstall "Development Tools"
   sudo yum install nasm
   ```

2. **Build Development Environment**:
   ```bash
   ./build.sh --dev
   ```

3. **Verify Installation**:
   ```bash
   ./bin/gx --version
   ./bin/gx_compiler --help
   ```

### IDE Setup

#### VS Code (Recommended)

1. Install the GX Language extension
2. Configure syntax highlighting
3. Set up debugging configuration

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

## 📝 Code Style Guidelines

### GX Language Style

#### Helper Naming
```gx
// ✅ Good: Descriptive names in snake_case
helper "data_processor" {
  can_do: ["data_analysis", "pattern_recognition"]
}

// ❌ Bad: Unclear names
helper "dp" {
  can_do: ["da", "pr"]
}
```

#### Brain Process Structure
```gx
// ✅ Good: Complete brain cycle
brain {
  plan {
    plan = { action: "process_data" }
  }
  
  execute {
    if plan.action == "process_data" {
      result = process(memory.data)
    }
  }
  
  remember {
    memory.result = result
  }
  
  communicate {
    broadcast "processing_complete"
  }
}

// ❌ Bad: Missing brain phases
brain {
  execute {
    result = process(memory.data)
  }
}
```

#### Memory Management
```gx
// ✅ Good: Clear variable names
remember {
  data_points = []
  analysis_results = {}
  last_processed_time = null
}

// ❌ Bad: Unclear variable names
remember {
  dp = []
  ar = {}
  lpt = null
}
```

#### Comments and Documentation
```gx
// ✅ Good: Clear comments
helper "complex_processor" {
  can_do: ["complex_analysis"]
  
  remember {
    // Cache for optimization results
    optimization_cache = {}
    
    // Configuration for analysis
    analysis_config = {
      timeout: 5000,
      max_iterations: 100
    }
  }

  brain {
    plan {
      // Analyze input and create execution plan
      plan = { action: "perform_complex_analysis" }
    }
    
    execute {
      if plan.action == "perform_complex_analysis" {
        // Process data with optimization
        result = process_with_optimization(memory.data)
      }
    }
  }
}
```

### C/C++ Style (for runtime components)

#### Naming Conventions
```c
// ✅ Good: Clear function and variable names
void process_gx_helper(const char* helper_name) {
    int helper_count = 0;
    struct helper_data* current_helper = NULL;
}

// ❌ Bad: Unclear names
void pgh(const char* hn) {
    int hc = 0;
    struct hd* ch = NULL;
}
```

#### Error Handling
```c
// ✅ Good: Proper error handling
int compile_gx_file(const char* filename) {
    FILE* file = fopen(filename, "r");
    if (!file) {
        fprintf(stderr, "Error: Cannot open file %s\n", filename);
        return -1;
    }
    
    // Process file...
    
    fclose(file);
    return 0;
}
```

---

## 🧪 Testing Guidelines

### Writing Tests

Create test files in the `tests/` directory:

```gx
// tests/test_your_feature.gx
helper "feature_tester" {
  can_do: ["testing", "feature_validation"]
  
  remember {
    test_results = []
    test_count = 0
  }

  brain {
    plan {
      plan = { action: "run_feature_tests" }
    }
    
    execute {
      if plan.action == "run_feature_tests" {
        // Test your feature
        result = test_your_feature()
        memory.test_results.push(result)
        memory.test_count += 1
      }
    }
    
    remember {
      memory.last_test_time = get_timestamp()
    }
    
    communicate {
      broadcast "feature_test_complete" {
        test_count: memory.test_count,
        results: memory.test_results
      }
    }
  }
}
```

### Running Tests

```bash
# Run all tests
./tests/run_tests.sh

# Run specific test category
./tests/test_brain_processes.sh

# Run with verbose output
./bin/gx --debug tests/test_your_feature.gx
```

### Test Categories

1. **Unit Tests**: Test individual helpers and functions
2. **Integration Tests**: Test helper interactions
3. **Brain Process Tests**: Validate cognitive cycles
4. **Compilation Tests**: Test parser and compiler
5. **Performance Tests**: Test optimization and performance
6. **Distributed Tests**: Test mesh networking

---

## 🔄 Pull Request Process

### Before Submitting

1. **Fork the repository**
2. **Create a feature branch**:
   ```bash
   git checkout -b feature/your-feature-name
   ```
3. **Make your changes**
4. **Test thoroughly**:
   ```bash
   ./build.sh
   ./tests/run_tests.sh
   ```
5. **Update documentation** if needed

### Commit Message Format

Use conventional commit format:

```
type(scope): description

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes
- `refactor`: Code refactoring
- `test`: Test additions/changes
- `chore`: Build/tooling changes

**Examples:**
```
feat(compiler): add constant folding optimization
fix(runtime): resolve memory leak in helper cleanup
docs(readme): update installation instructions
test(parser): add tests for new syntax features
```

### Pull Request Template

```markdown
## Description
Brief description of changes

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Documentation update
- [ ] Test addition
- [ ] Performance improvement
- [ ] Refactoring

## Testing
- [ ] All tests pass
- [ ] New tests added
- [ ] Manual testing completed

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] Tests added/updated
- [ ] No breaking changes
```

---

## 🐛 Issue Reporting

### Before Reporting

1. **Search existing issues** to avoid duplicates
2. **Check documentation** for solutions
3. **Test with latest version**

### Issue Template

```markdown
## Bug Description
Clear description of the issue

## Steps to Reproduce
1. Step 1
2. Step 2
3. Step 3

## Expected Behavior
What should happen

## Actual Behavior
What actually happens

## Environment
- OS: [e.g., macOS 12.0]
- GX Version: [e.g., 1.0.0]
- Architecture: [e.g., x86_64]

## Additional Information
Screenshots, logs, etc.
```

### Feature Request Template

```markdown
## Feature Description
Clear description of the requested feature

## Use Case
Why this feature is needed

## Proposed Implementation
How you think it should work

## Alternatives Considered
Other approaches you considered
```

---

## 👥 Community Guidelines

### Code of Conduct

We are committed to providing a welcoming and inclusive environment for all contributors.

#### Our Standards

**Examples of behavior that contributes to a positive environment:**
- Using welcoming and inclusive language
- Being respectful of differing viewpoints
- Gracefully accepting constructive criticism
- Focusing on what is best for the community
- Showing empathy towards other community members

**Examples of unacceptable behavior:**
- The use of sexualized language or imagery
- Trolling, insulting/derogatory comments
- Personal or political attacks
- Publishing others' private information
- Other conduct which could reasonably be considered inappropriate

### Communication

- **GitHub Issues**: For bug reports and feature requests
- **GitHub Discussions**: For questions and general discussion
- **Discord**: For real-time community chat

### Recognition

Contributors will be recognized in:
- **README.md** contributors section
- **Release notes** for significant contributions
- **GitHub contributors** page

---

## 📚 Additional Resources

- **[Developer Guide](docs/DEVELOPER_GUIDE.md)** - Detailed development guide
- **[API Reference](docs/API_REFERENCE.md)** - Complete language reference
- **[Examples](examples/)** - Sample programs and use cases
- **[Tests](tests/)** - Test suite and examples

---

## 🙏 Thank You

Thank you for contributing to GX Language! Your contributions help make brain-first programming accessible to everyone.

---

*This contributing guide is maintained by the GX Development Team. For questions or suggestions, please open an issue or join our discussions.* 