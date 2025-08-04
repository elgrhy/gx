# 📚 GX Language API Reference

## 📋 Table of Contents

1. [Language Constructs](#language-constructs)
2. [Built-in Functions](#built-in-functions)
3. [System Calls](#system-calls)
4. [Standard Library](#standard-library)
5. [Error Handling](#error-handling)
6. [Performance APIs](#performance-apis)
7. [Distributed APIs](#distributed-apis)

---

## 🏗️ Language Constructs

### Helper Definition

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

**Parameters:**
- `helper_name` (string): Unique identifier for the helper
- `can_do` (array): List of capabilities the helper provides
- `remember` (block): Memory initialization and state management
- `receive` (block): Input channel definitions
- `brain` (block): Cognitive process implementation

### Brain Process

```gx
brain {
  plan {
    plan = { action: "process_data", priority: "high" }
  }
  
  execute {
    if plan.action == "process_data" {
      result = process(memory.data)
    }
  }
  
  remember {
    memory.result = result
    memory.last_execution = get_timestamp()
  }
  
  communicate {
    broadcast "processing_complete"
    send_to "logger" { data: result }
  }
}
```

**Phases:**
- `plan`: Analyze input and create execution plan
- `execute`: Perform actions based on plan
- `remember`: Persist state and results
- `communicate`: Signal completion and share results

### Memory Management

```gx
remember {
  // Primitive types
  string_var = "hello"
  number_var = 42
  boolean_var = true
  null_var = null
  
  // Arrays
  array_var = [1, 2, 3, 4]
  mixed_array = ["string", 42, true]
  
  // Objects
  object_var = {
    key1: "value1",
    key2: 42,
    nested: {
      subkey: "subvalue"
    }
  }
  
  // Functions
  function_var = function(x) { return x * 2 }
}
```

### Message Communication

```gx
receive {
  from "source" as "channel_name" {
    type: "data_type"
    bind: memory.variable
    on_receive: brain.handler_function
  }
}

communicate {
  broadcast "event_name"
  send_to "target_helper" {
    data: memory.result,
    timestamp: get_timestamp()
  }
}
```

### Recipe (Function) Definition

```gx
recipe "function_name" {
  needs: parameter1, parameter2
  gives: return_value
  
  brain {
    plan {
      plan = { action: "process_parameters" }
    }
    
    execute {
      if plan.action == "process_parameters" {
        return_value = process(parameter1, parameter2)
      }
    }
  }
}
```

### Objective (Conditional Logic)

```gx
objective "objective_name" {
  when condition_expression
  then {
    action: "action_name",
    parameters: action_parameters
  }
}
```

### Message Handler

```gx
message "message_name" {
  do {
    // Handler implementation
    process_message(memory.message_data)
  }
}
```

---

## 🔧 Built-in Functions

### System Functions

```gx
// Time and timing
get_timestamp()           // Returns current timestamp
sleep(milliseconds)       // Sleep for specified milliseconds
delay(milliseconds)       // Non-blocking delay

// Memory management
allocate_memory(size)     // Allocate memory block
free_memory(pointer)      // Free allocated memory
get_memory_usage()        // Get current memory usage

// Process management
get_current_process_id()  // Get current process ID
spawn_process(command)    // Spawn new process
kill_process(pid)         // Terminate process

// File system
read_file(filename)       // Read file contents
write_file(filename, data) // Write data to file
file_exists(filename)     // Check if file exists
delete_file(filename)     // Delete file
```

### String Functions

```gx
// String manipulation
string_length(str)        // Get string length
string_concat(str1, str2) // Concatenate strings
string_substring(str, start, end) // Extract substring
string_split(str, delimiter) // Split string
string_replace(str, old, new) // Replace substring
string_to_upper(str)     // Convert to uppercase
string_to_lower(str)     // Convert to lowercase
string_trim(str)         // Remove whitespace
```

### Array Functions

```gx
// Array operations
array_length(arr)         // Get array length
array_push(arr, item)     // Add item to end
array_pop(arr)           // Remove and return last item
array_shift(arr)         // Remove and return first item
array_unshift(arr, item) // Add item to beginning
array_slice(arr, start, end) // Extract subarray
array_join(arr, separator) // Join array elements
array_sort(arr)          // Sort array
array_reverse(arr)       // Reverse array
```

### Object Functions

```gx
// Object operations
object_keys(obj)         // Get object keys
object_values(obj)       // Get object values
object_has_key(obj, key) // Check if key exists
object_get(obj, key)     // Get object value
object_set(obj, key, value) // Set object value
object_delete(obj, key)  // Delete object property
object_merge(obj1, obj2) // Merge objects
object_clone(obj)        // Clone object
```

### Mathematical Functions

```gx
// Basic math
add(a, b)                // Addition
subtract(a, b)           // Subtraction
multiply(a, b)           // Multiplication
divide(a, b)             // Division
modulo(a, b)             // Modulo
power(base, exponent)    // Exponentiation

// Advanced math
sqrt(number)             // Square root
abs(number)              // Absolute value
floor(number)            // Floor function
ceil(number)             // Ceiling function
round(number)            // Round to nearest integer
random(min, max)         // Random number generation
```

### Network Functions

```gx
// Network operations
http_get(url)            // HTTP GET request
http_post(url, data)     // HTTP POST request
http_put(url, data)      // HTTP PUT request
http_delete(url)         // HTTP DELETE request
websocket_connect(url)   // WebSocket connection
websocket_send(data)     // Send WebSocket message
websocket_close()        // Close WebSocket connection
```

---

## 💻 System Calls

### Process Management

```gx
// Process creation and control
sys_fork()               // Create child process
sys_exec(filename, args) // Execute program
sys_exit(exit_code)      // Terminate process
sys_wait(pid)           // Wait for process
sys_kill(pid, signal)   // Send signal to process
sys_getpid()            // Get process ID
sys_getppid()           // Get parent process ID
```

### Memory Management

```gx
// Memory allocation and management
sys_brk(new_break)      // Set heap break point
sys_mmap(address, length, prot, flags) // Memory mapping
sys_munmap(address, length) // Unmap memory
sys_mprotect(address, length, prot) // Change protection
```

### File System

```gx
// File operations
sys_open(filename, flags) // Open file
sys_read(fd, buffer, size) // Read from file
sys_write(fd, buffer, size) // Write to file
sys_close(fd)           // Close file
sys_lseek(fd, offset, whence) // Seek in file
sys_unlink(filename)    // Delete file
sys_mkdir(pathname, mode) // Create directory
sys_rmdir(pathname)     // Remove directory
```

### Device Management

```gx
// Device operations
sys_ioctl(fd, request, arg) // Device control
sys_device_open(device, mode) // Open device
sys_device_close(fd)    // Close device
sys_device_read(fd, buffer, size) // Read from device
sys_device_write(fd, buffer, size) // Write to device
```

### Network Operations

```gx
// Network socket operations
sys_socket(domain, type, protocol) // Create socket
sys_bind(sockfd, addr, addrlen) // Bind socket
sys_connect(sockfd, addr, addrlen) // Connect socket
sys_listen(sockfd, backlog) // Listen for connections
sys_accept(sockfd, addr, addrlen) // Accept connection
sys_send(sockfd, buffer, size, flags) // Send data
sys_recv(sockfd, buffer, size, flags) // Receive data
sys_close(sockfd)       // Close socket
```

---

## 📚 Standard Library

### Configuration Management

```gx
helper "config_manager" {
  can_do: ["configuration", "settings_management"]
  
  remember {
    config = {}
    config_file = "config.json"
  }

  brain {
    plan {
      plan = { action: "load_configuration" }
    }
    
    execute {
      if plan.action == "load_configuration" {
        memory.config = load_config_file(memory.config_file)
      }
    }
    
    communicate {
      broadcast "configuration_loaded"
    }
  }
}
```

### Logging System

```gx
helper "logger" {
  can_do: ["logging", "debug_output"]
  
  remember {
    log_level = "info"
    log_file = "app.log"
  }

  brain {
    plan {
      plan = { action: "log_message" }
    }
    
    execute {
      if plan.action == "log_message" {
        write_log(memory.log_level, memory.message)
      }
    }
  }
}
```

### Database Interface

```gx
helper "database_manager" {
  can_do: ["database_operations", "query_execution"]
  
  remember {
    connection = null
    database_url = "sqlite://data.db"
  }

  brain {
    plan {
      plan = { action: "execute_query" }
    }
    
    execute {
      if plan.action == "execute_query" {
        memory.result = execute_sql_query(memory.query)
      }
    }
  }
}
```

---

## ⚠️ Error Handling

### Try-Catch Blocks

```gx
try {
  // Risky operation
  result = divide(a, b)
} catch (error) {
  // Handle error
  log("Division error: " + error.message)
  result = 0
}
```

### Error Types

```gx
// Common error types
RuntimeError              // Runtime execution error
SyntaxError              // Syntax parsing error
TypeError                // Type mismatch error
ReferenceError           // Undefined variable error
NetworkError             // Network communication error
FileError                // File system error
MemoryError              // Memory allocation error
```

### Error Handling Functions

```gx
// Error handling utilities
throw_error(message)     // Throw custom error
catch_error(operation)   // Catch operation errors
is_error(value)         // Check if value is error
get_error_message(error) // Get error message
get_error_stack(error)  // Get error stack trace
```

---

## ⚡ Performance APIs

### Profiling

```gx
// Performance profiling
start_profiler()         // Start performance profiling
stop_profiler()          // Stop profiling
get_profiler_data()      // Get profiling results
get_execution_time()     // Get current execution time
get_memory_usage()       // Get memory usage statistics
get_cpu_usage()          // Get CPU usage statistics
```

### Optimization

```gx
// Code optimization
optimize_code(source)    // Optimize source code
compile_to_bytecode(source) // Compile to bytecode
optimize_bytecode(bytecode) // Optimize bytecode
generate_native_code(bytecode) // Generate native code
```

### Caching

```gx
// Caching mechanisms
cache_set(key, value, ttl) // Set cache value
cache_get(key)           // Get cache value
cache_delete(key)        // Delete cache value
cache_clear()            // Clear all cache
cache_stats()            // Get cache statistics
```

---

## 🌐 Distributed APIs

### Mesh Networking

```gx
// Distributed mesh operations
mesh_connect(node_address) // Connect to mesh node
mesh_disconnect(node_id)  // Disconnect from node
mesh_broadcast(message)   // Broadcast to all nodes
mesh_send(node_id, message) // Send to specific node
mesh_receive()           // Receive mesh messages
mesh_get_nodes()         // Get connected nodes
mesh_get_topology()      // Get network topology
```

### Knowledge Sharing

```gx
// Knowledge sharing operations
share_knowledge(data)    // Share knowledge with mesh
receive_knowledge()      // Receive shared knowledge
get_shared_patterns()    // Get discovered patterns
get_optimization_tips()  // Get optimization tips
get_best_practices()     // Get best practices
```

### Pattern Discovery

```gx
// Pattern discovery and learning
discover_patterns(data)  // Discover patterns in data
learn_from_pattern(pattern) // Learn from discovered pattern
apply_pattern(pattern, data) // Apply pattern to data
get_learning_stats()     // Get learning statistics
get_pattern_effectiveness(pattern) // Get pattern effectiveness
```

---

## 🔍 Debugging APIs

### Debugging Functions

```gx
// Debugging utilities
debug_log(message)       // Log debug message
debug_break()           // Set debug breakpoint
debug_inspect(variable) // Inspect variable value
debug_trace()           // Enable execution tracing
debug_profile()         // Enable performance profiling
debug_memory()          // Inspect memory usage
```

### Brain Process Debugging

```gx
// Brain process debugging
debug_brain_cycle()     // Debug brain cycle execution
debug_memory_state()    // Debug memory state
debug_message_flow()    // Debug message communication
debug_helper_lifecycle() // Debug helper lifecycle
debug_optimization()    // Debug optimization process
```

---

## 📊 Monitoring APIs

### System Monitoring

```gx
// System monitoring
get_system_stats()      // Get system statistics
get_process_stats()     // Get process statistics
get_memory_stats()      // Get memory statistics
get_network_stats()     // Get network statistics
get_disk_stats()        // Get disk statistics
get_cpu_stats()         // Get CPU statistics
```

### Application Monitoring

```gx
// Application monitoring
get_app_metrics()       // Get application metrics
get_performance_metrics() // Get performance metrics
get_error_rates()       // Get error rates
get_throughput_stats()  // Get throughput statistics
get_latency_stats()     // Get latency statistics
```

---

## 🚀 Deployment APIs

### Container Management

```gx
// Container operations
container_create(image) // Create container
container_start(id)     // Start container
container_stop(id)      // Stop container
container_restart(id)   // Restart container
container_delete(id)    // Delete container
container_logs(id)      // Get container logs
```

### Kubernetes Integration

```gx
// Kubernetes operations
k8s_deploy(manifest)   // Deploy to Kubernetes
k8s_scale(deployment, replicas) // Scale deployment
k8s_update(deployment, image) // Update deployment
k8s_delete(resource)   // Delete Kubernetes resource
k8s_get_pods()         // Get pod information
k8s_get_services()     // Get service information
```

---

*This API reference is maintained by the GX Development Team. For questions or contributions, please see our [Contributing Guide](../CONTRIBUTING.md).*

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer.