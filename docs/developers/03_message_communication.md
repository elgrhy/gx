# 📡 Message Communication in GX

## Understanding Message Communication

In GX, helpers communicate with each other through **messages**. This is how different parts of your application can work together and share information.

## Basic Message Structure

```gx
receive {
  from "source" as "channel_name" {
    type: "data_type"
    bind: memory.variable
    on_receive: brain.handler_function
  }
}
```

## Simple Message Examples

### Basic Sender and Receiver

```gx
helper "message_sender" {
  can_do: ["sending_messages"]
  
  remember {
    message_count = 0
  }

  brain {
    plan {
      plan = { action: "send_messages" }
    }
    
    execute {
      if plan.action == "send_messages" {
        // Send a simple message
        send_to "message_receiver" {
          type: "greeting",
          content: "Hello from sender!",
          timestamp: get_timestamp()
        }
        
        memory.message_count += 1
        
        // Send another message
        send_to "message_receiver" {
          type: "data",
          numbers: [1, 2, 3, 4, 5],
          count: memory.message_count
        }
      }
    }
    
    communicate {
      broadcast "messages_sent"
    }
  }
}

helper "message_receiver" {
  can_do: ["receiving_messages"]
  
  remember {
    received_messages = []
    greeting_count = 0
  }

  receive {
    from "message_sender" as "incoming_messages" {
      type: "greeting"
      bind: memory.greeting_message
      on_receive: brain.handle_greeting
    }
    
    from "message_sender" as "data_messages" {
      type: "data"
      bind: memory.data_message
      on_receive: brain.handle_data
    }
  }

  brain {
    plan {
      if memory.greeting_message {
        plan = { action: "process_greeting" }
      } else if memory.data_message {
        plan = { action: "process_data" }
      } else {
        plan = { action: "wait_for_messages" }
      }
    }
    
    execute {
      if plan.action == "process_greeting" {
        output("Received greeting: " + memory.greeting_message.content)
        memory.greeting_count += 1
        memory.received_messages.push(memory.greeting_message)
      } else if plan.action == "process_data" {
        output("Received data with " + memory.data_message.numbers.length + " numbers")
        output("Message count: " + memory.data_message.count)
        memory.received_messages.push(memory.data_message)
      }
    }
    
    communicate {
      if memory.greeting_count > 0 {
        broadcast "greetings_processed"
      }
    }
  }
}
```

## Advanced Message Patterns

### Request-Response Pattern

```gx
helper "calculator_service" {
  can_do: ["mathematical_operations"]
  
  remember {
    operations_performed = 0
  }

  receive {
    from "client" as "calculation_requests" {
      type: "calculate"
      bind: memory.request
      on_receive: brain.handle_calculation
    }
  }

  brain {
    plan {
      if memory.request {
        plan = { action: "perform_calculation" }
      } else {
        plan = { action: "wait_for_requests" }
      }
    }
    
    execute {
      if plan.action == "perform_calculation" {
        operation = memory.request.operation
        a = memory.request.a
        b = memory.request.b
        result = 0
        
        if operation == "add" {
          result = a + b
        } else if operation == "subtract" {
          result = a - b
        } else if operation == "multiply" {
          result = a * b
        } else if operation == "divide" {
          if b != 0 {
            result = a / b
          } else {
            result = { error: "Division by zero" }
          }
        }
        
        // Send response back to client
        send_to "client" {
          type: "calculation_result",
          request_id: memory.request.request_id,
          result: result,
          operation: operation
        }
        
        memory.operations_performed += 1
      }
    }
    
    communicate {
      broadcast "calculation_completed"
    }
  }
}

helper "calculation_client" {
  can_do: ["requesting_calculations"]
  
  remember {
    pending_requests = {}
    results = []
  }

  brain {
    plan {
      plan = { action: "request_calculations" }
    }
    
    execute {
      if plan.action == "request_calculations" {
        // Send calculation requests
        request_id = generate_request_id()
        
        send_to "calculator_service" {
          type: "calculate",
          request_id: request_id,
          operation: "add",
          a: 10,
          b: 5
        }
        
        memory.pending_requests[request_id] = {
          operation: "add",
          a: 10,
          b: 5
        }
        
        // Send another request
        request_id2 = generate_request_id()
        
        send_to "calculator_service" {
          type: "calculate",
          request_id: request_id2,
          operation: "multiply",
          a: 7,
          b: 8
        }
        
        memory.pending_requests[request_id2] = {
          operation: "multiply",
          a: 7,
          b: 8
        }
      }
    }
  }

  receive {
    from "calculator_service" as "calculation_results" {
      type: "calculation_result"
      bind: memory.result
      on_receive: brain.handle_result
    }
  }

  brain {
    plan {
      if memory.result {
        plan = { action: "process_result" }
      }
    }
    
    execute {
      if plan.action == "process_result" {
        request_id = memory.result.request_id
        operation = memory.result.operation
        result = memory.result.result
        
        output("Calculation result for " + operation + ": " + result)
        memory.results.push(memory.result)
        
        // Remove from pending requests
        delete memory.pending_requests[request_id]
      }
    }
  }
}
```

### Broadcast Pattern

```gx
helper "event_broadcaster" {
  can_do: ["broadcasting_events"]
  
  remember {
    event_count = 0
  }

  brain {
    plan {
      plan = { action: "broadcast_events" }
    }
    
    execute {
      if plan.action == "broadcast_events" {
        // Broadcast to all listeners
        broadcast "user_login" {
          user_id: "user123",
          timestamp: get_timestamp(),
          event_type: "login"
        }
        
        memory.event_count += 1
        
        // Broadcast another event
        broadcast "data_updated" {
          table: "users",
          record_count: 150,
          timestamp: get_timestamp()
        }
        
        memory.event_count += 1
      }
    }
    
    communicate {
      broadcast "broadcasting_complete"
    }
  }
}

helper "event_listener_1" {
  can_do: ["listening_to_events"]
  
  remember {
    events_received = []
  }

  receive {
    from "event_broadcaster" as "broadcast_events" {
      type: "user_login"
      bind: memory.login_event
      on_receive: brain.handle_login
    }
  }

  brain {
    plan {
      if memory.login_event {
        plan = { action: "handle_login_event" }
      }
    }
    
    execute {
      if plan.action == "handle_login_event" {
        output("Listener 1: User " + memory.login_event.user_id + " logged in")
        memory.events_received.push(memory.login_event)
      }
    }
  }
}

helper "event_listener_2" {
  can_do: ["listening_to_events"]
  
  remember {
    events_received = []
  }

  receive {
    from "event_broadcaster" as "broadcast_events" {
      type: "data_updated"
      bind: memory.data_event
      on_receive: brain.handle_data_update
    }
  }

  brain {
    plan {
      if memory.data_event {
        plan = { action: "handle_data_event" }
      }
    }
    
    execute {
      if plan.action == "handle_data_event" {
        output("Listener 2: Data updated in " + memory.data_event.table)
        output("Records affected: " + memory.data_event.record_count)
        memory.events_received.push(memory.data_event)
      }
    }
  }
}
```

## Message Filtering and Routing

```gx
helper "message_router" {
  can_do: ["routing_messages"]
  
  remember {
    routing_rules = {
      "user_events": "user_processor",
      "data_events": "data_processor",
      "system_events": "system_monitor"
    }
  }

  receive {
    from "various_sources" as "incoming_messages" {
      type: "any"
      bind: memory.incoming_message
      on_receive: brain.route_message
    }
  }

  brain {
    plan {
      if memory.incoming_message {
        plan = { action: "route_message" }
      }
    }
    
    execute {
      if plan.action == "route_message" {
        message_type = memory.incoming_message.type
        target_helper = memory.routing_rules[message_type]
        
        if target_helper {
          // Route to appropriate helper
          send_to target_helper {
            original_message: memory.incoming_message,
            routed_by: "message_router",
            timestamp: get_timestamp()
          }
          
          output("Routed " + message_type + " message to " + target_helper)
        } else {
          // Default routing
          send_to "default_processor" {
            message: memory.incoming_message,
            reason: "no_specific_route"
          }
        }
      }
    }
  }
}
```

## Message Queuing and Processing

```gx
helper "message_queue" {
  can_do: ["queuing_messages"]
  
  remember {
    message_queue = []
    processing = false
  }

  receive {
    from "various_sources" as "incoming_messages" {
      type: "any"
      bind: memory.new_message
      on_receive: brain.queue_message
    }
  }

  brain {
    plan {
      if memory.new_message {
        plan = { action: "add_to_queue" }
      } else if memory.message_queue.length > 0 && !memory.processing {
        plan = { action: "process_queue" }
      } else {
        plan = { action: "wait_for_messages" }
      }
    }
    
    execute {
      if plan.action == "add_to_queue" {
        // Add message to queue
        memory.message_queue.push({
          message: memory.new_message,
          timestamp: get_timestamp(),
          priority: memory.new_message.priority || "normal"
        })
        
        output("Message added to queue. Queue size: " + memory.message_queue.length)
      } else if plan.action == "process_queue" {
        memory.processing = true
        
        // Process messages in order
        while memory.message_queue.length > 0 {
          queued_message = memory.message_queue.shift()
          
          // Process the message
          process_message(queued_message.message)
          
          output("Processed message. Remaining: " + memory.message_queue.length)
        }
        
        memory.processing = false
      }
    }
    
    communicate {
      if memory.message_queue.length == 0 && memory.processing == false {
        broadcast "queue_empty"
      }
    }
  }

  recipe "process_message" {
    needs: message
    gives: result
    
    brain {
      plan {
        plan = { action: "process" }
      }
      
      execute {
        if plan.action == "process" {
          // Simulate message processing
          result = {
            processed: true,
            message_id: message.id,
            processing_time: get_timestamp() - message.timestamp
          }
        }
      }
    }
  }
}
```

## Error Handling in Messages

```gx
helper "robust_message_handler" {
  can_do: ["handling_messages"]
  
  remember {
    successful_messages = 0
    failed_messages = 0
    retry_queue = []
  }

  receive {
    from "various_sources" as "incoming_messages" {
      type: "any"
      bind: memory.incoming_message
      on_receive: brain.handle_message
    }
  }

  brain {
    plan {
      if memory.incoming_message {
        plan = { action: "process_message" }
      }
    }
    
    execute {
      if plan.action == "process_message" {
        try {
          // Attempt to process message
          result = process_message_safely(memory.incoming_message)
          
          if result.success {
            memory.successful_messages += 1
            output("Message processed successfully")
          } else {
            // Handle processing failure
            handle_processing_failure(memory.incoming_message, result.error)
          }
        } catch error {
          // Handle unexpected errors
          memory.failed_messages += 1
          output("Error processing message: " + error)
          
          // Add to retry queue
          memory.retry_queue.push({
            message: memory.incoming_message,
            error: error,
            retry_count: 0
          })
        }
      }
    }
  }

  recipe "process_message_safely" {
    needs: message
    gives: result
    
    brain {
      plan {
        plan = { action: "process" }
      }
      
      execute {
        if plan.action == "process" {
          // Validate message
          if !message.content {
            result = { success: false, error: "Empty message" }
            return
          }
          
          // Process message
          result = { success: true, processed_content: message.content }
        }
      }
    }
  }

  recipe "handle_processing_failure" {
    needs: message, error
    gives: handled
    
    brain {
      plan {
        plan = { action: "handle" }
      }
      
      execute {
        if plan.action == "handle" {
          // Log the failure
          log_error("Message processing failed", {
            message: message,
            error: error,
            timestamp: get_timestamp()
          })
          
          // Send error notification
          send_to "error_monitor" {
            type: "processing_error",
            original_message: message,
            error: error
          }
          
          handled = true
        }
      }
    }
  }
}
```

## Practice Exercises

1. **Create a chat system** where multiple helpers can send and receive messages
2. **Build a notification system** that broadcasts events to multiple listeners
3. **Make a request-response service** that processes requests and sends back responses
4. **Create a message filter** that routes different types of messages to appropriate handlers
5. **Build a message queue** that processes messages in order with priority

## Next Steps

Now that you understand message communication, you're ready to:
- [Build Interactive Applications](04_interactive_applications.md)
- [Create Web Applications](05_web_applications.md)
- [Build AI Applications](06_ai_applications.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 