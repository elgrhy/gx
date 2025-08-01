# 🎨 Building Interactive Applications with GX

## Overview

In this tutorial, we'll build interactive applications with user interfaces, real-time updates, and dynamic content. We'll learn how to create engaging user experiences using GX's brain-first approach.

## Architecture Overview

Our interactive applications will include:
- **UI Manager**: Handles user interface rendering and updates
- **Event System**: Processes user interactions and events
- **State Management**: Manages application state and data flow
- **Real-time Updates**: Provides live data updates and notifications
- **Animation System**: Creates smooth transitions and animations

## Step 1: Basic Interactive UI

```gx
helper "interactive_ui" {
  can_do: ["ui_rendering", "event_handling", "state_management"]
  
  remember {
    ui_state = {
      current_page: "home",
      user_input: "",
      notifications: [],
      theme: "light"
    }
    event_queue = []
  }

  receive {
    from "user_interface" as "user_events" {
      type: "user_interaction"
      bind: memory.user_event
      on_receive: brain.handle_user_event
    }
  }

  brain {
    plan {
      if memory.user_event {
        plan = { action: "process_user_event" }
      } else {
        plan = { action: "update_ui" }
      }
    }

    execute {
      if plan.action == "process_user_event" {
        event_type = memory.user_event.type
        
        if event_type == "button_click" {
          handle_button_click(memory.user_event.button_id)
        } else if event_type == "text_input" {
          handle_text_input(memory.user_event.input_value)
        } else if event_type == "navigation" {
          handle_navigation(memory.user_event.target_page)
        }
        
      } else if plan.action == "update_ui" {
        // Update UI based on current state
        render_current_page()
        update_notifications()
        apply_theme()
      }
    }

    communicate {
      broadcast "ui_updated"
    }
  }

  recipe "handle_button_click" {
    needs: button_id
    gives: success
    
    brain {
      plan {
        plan = { action: "handle" }
      }
      
      execute {
        if plan.action == "handle" {
          if button_id == "submit" {
            process_form_submission()
          } else if button_id == "theme_toggle" {
            toggle_theme()
          } else if button_id == "clear" {
            clear_user_input()
          } else {
            // Handle other button clicks
            handle_custom_button(button_id)
          }
          
          success = true
        }
      }
    }
  }

  recipe "handle_text_input" {
    needs: input_value
    gives: processed_input
    
    brain {
      plan {
        plan = { action: "process" }
      }
      
      execute {
        if plan.action == "process" {
          // Update UI state with new input
          memory.ui_state.user_input = input_value
          
          // Process the input (e.g., validation, formatting)
          processed_input = validate_and_format_input(input_value)
          
          // Update UI to reflect changes
          update_input_display(processed_input)
        }
      }
    }
  }

  recipe "render_current_page" {
    needs: none
    gives: rendered_page
    
    brain {
      plan {
        plan = { action: "render" }
      }
      
      execute {
        if plan.action == "render" {
          current_page = memory.ui_state.current_page
          
          if current_page == "home" {
            rendered_page = render_home_page()
          } else if current_page == "profile" {
            rendered_page = render_profile_page()
          } else if current_page == "settings" {
            rendered_page = render_settings_page()
          } else {
            rendered_page = render_404_page()
          }
        }
      }
    }
  }
}
```

## Step 2: Real-time Dashboard

```gx
helper "realtime_dashboard" {
  can_do: ["dashboard_rendering", "data_visualization", "live_updates"]
  
  remember {
    dashboard_data = {
      metrics: {},
      charts: {},
      alerts: [],
      last_update: 0
    }
    update_interval = 5000 // 5 seconds
  }

  brain {
    plan {
      plan = { action: "update_dashboard" }
    }

    execute {
      if plan.action == "update_dashboard" {
        // Fetch latest data
        fetch_latest_metrics()
        
        // Update charts and visualizations
        update_charts()
        
        // Check for alerts
        check_alerts()
        
        // Render dashboard
        render_dashboard()
        
        // Schedule next update
        schedule_next_update()
      }
    }
  }

  recipe "fetch_latest_metrics" {
    needs: none
    gives: metrics
    
    brain {
      plan {
        plan = { action: "fetch" }
      }
      
      execute {
        if plan.action == "fetch" {
          // Simulate fetching real-time metrics
          metrics = {
            active_users: get_random_number(100, 1000),
            system_load: get_random_number(20, 80),
            response_time: get_random_number(50, 200),
            error_rate: get_random_number(0, 5),
            revenue: get_random_number(10000, 50000)
          }
          
          memory.dashboard_data.metrics = metrics
          memory.dashboard_data.last_update = get_timestamp()
        }
      }
    }
  }

  recipe "update_charts" {
    needs: none
    gives: updated_charts
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          metrics = memory.dashboard_data.metrics
          
          // Update line chart for system load
          system_load_chart = {
            type: "line",
            data: generate_time_series_data("system_load", 24),
            title: "System Load (24h)",
            color: "#4CAF50"
          }
          
          // Update bar chart for user activity
          user_activity_chart = {
            type: "bar",
            data: generate_user_activity_data(),
            title: "User Activity by Hour",
            color: "#2196F3"
          }
          
          // Update pie chart for error distribution
          error_distribution_chart = {
            type: "pie",
            data: generate_error_distribution_data(),
            title: "Error Distribution",
            colors: ["#F44336", "#FF9800", "#FFC107", "#4CAF50"]
          }
          
          updated_charts = {
            system_load: system_load_chart,
            user_activity: user_activity_chart,
            error_distribution: error_distribution_chart
          }
          
          memory.dashboard_data.charts = updated_charts
        }
      }
    }
  }

  recipe "check_alerts" {
    needs: none
    gives: alerts
    
    brain {
      plan {
        plan = { action: "check" }
      }
      
      execute {
        if plan.action == "check" {
          metrics = memory.dashboard_data.metrics
          alerts = []
          
          // Check for high system load
          if metrics.system_load > 70 {
            alerts.push({
              type: "warning",
              message: "High system load detected: " + metrics.system_load + "%",
              timestamp: get_timestamp(),
              priority: "medium"
            })
          }
          
          // Check for high error rate
          if metrics.error_rate > 3 {
            alerts.push({
              type: "error",
              message: "High error rate: " + metrics.error_rate + "%",
              timestamp: get_timestamp(),
              priority: "high"
            })
          }
          
          // Check for slow response time
          if metrics.response_time > 150 {
            alerts.push({
              type: "warning",
              message: "Slow response time: " + metrics.response_time + "ms",
              timestamp: get_timestamp(),
              priority: "medium"
            })
          }
          
          memory.dashboard_data.alerts = alerts
        }
      }
    }
  }
}
```

## Step 3: Interactive Form System

```gx
helper "interactive_form" {
  can_do: ["form_handling", "validation", "dynamic_fields"]
  
  remember {
    form_state = {
      fields: {},
      validation_errors: {},
      is_submitting: false,
      submission_count: 0
    }
    form_config = {}
  }

  receive {
    from "user_interface" as "form_events" {
      type: "form_event"
      bind: memory.form_event
      on_receive: brain.handle_form_event
    }
  }

  brain {
    plan {
      if memory.form_event {
        plan = { action: "process_form_event" }
      } else {
        plan = { action: "validate_form" }
      }
    }

    execute {
      if plan.action == "process_form_event" {
        event_type = memory.form_event.type
        
        if event_type == "field_change" {
          handle_field_change(memory.form_event.field_name, memory.form_event.value)
        } else if event_type == "field_focus" {
          handle_field_focus(memory.form_event.field_name)
        } else if event_type == "field_blur" {
          handle_field_blur(memory.form_event.field_name)
        } else if event_type == "form_submit" {
          handle_form_submit()
        }
        
      } else if plan.action == "validate_form" {
        // Validate all form fields
        validate_all_fields()
        
        // Update UI with validation results
        update_validation_display()
      }
    }
  }

  recipe "handle_field_change" {
    needs: field_name, value
    gives: validation_result
    
    brain {
      plan {
        plan = { action: "handle" }
      }
      
      execute {
        if plan.action == "handle" {
          // Update field value
          memory.form_state.fields[field_name] = value
          
          // Validate this specific field
          validation_result = validate_field(field_name, value)
          
          // Update validation errors
          if validation_result.is_valid {
            delete memory.form_state.validation_errors[field_name]
          } else {
            memory.form_state.validation_errors[field_name] = validation_result.error_message
          }
          
          // Trigger dependent field updates
          update_dependent_fields(field_name, value)
        }
      }
    }
  }

  recipe "validate_field" {
    needs: field_name, value
    gives: validation_result
    
    brain {
      plan {
        plan = { action: "validate" }
      }
      
      execute {
        if plan.action == "validate" {
          validation_result = {
            is_valid: true,
            error_message: ""
          }
          
          // Get field configuration
          field_config = memory.form_config[field_name] || {}
          
          // Check required field
          if field_config.required && (!value || value.trim() === "") {
            validation_result.is_valid = false
            validation_result.error_message = "This field is required"
            return validation_result
          }
          
          // Check minimum length
          if field_config.min_length && value.length < field_config.min_length {
            validation_result.is_valid = false
            validation_result.error_message = "Minimum length is " + field_config.min_length + " characters"
            return validation_result
          }
          
          // Check maximum length
          if field_config.max_length && value.length > field_config.max_length {
            validation_result.is_valid = false
            validation_result.error_message = "Maximum length is " + field_config.max_length + " characters"
            return validation_result
          }
          
          // Check email format
          if field_name === "email" && value {
            email_regex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
            if !email_regex.test(value) {
              validation_result.is_valid = false
              validation_result.error_message = "Please enter a valid email address"
              return validation_result
            }
          }
          
          // Check phone format
          if field_name === "phone" && value {
            phone_regex = /^[\+]?[1-9][\d]{0,15}$/
            if !phone_regex.test(value.replace(/[\s\-\(\)]/g, "")) {
              validation_result.is_valid = false
              validation_result.error_message = "Please enter a valid phone number"
              return validation_result
            }
          }
        }
      }
    }
  }

  recipe "handle_form_submit" {
    needs: none
    gives: submission_result
    
    brain {
      plan {
        plan = { action: "submit" }
      }
      
      execute {
        if plan.action == "submit" {
          // Validate all fields before submission
          validate_all_fields()
          
          // Check if form is valid
          if Object.keys(memory.form_state.validation_errors).length === 0 {
            memory.form_state.is_submitting = true
            
            // Prepare form data
            form_data = prepare_form_data()
            
            // Submit form data
            submission_result = submit_form_data(form_data)
            
            if submission_result.success {
              memory.form_state.submission_count += 1
              clear_form()
              show_success_message("Form submitted successfully!")
            } else {
              show_error_message("Submission failed: " + submission_result.error)
            }
            
            memory.form_state.is_submitting = false
          } else {
            show_error_message("Please fix validation errors before submitting")
          }
        }
      }
    }
  }
}
```

## Step 4: Animation System

```gx
helper "animation_system" {
  can_do: ["animation_rendering", "transition_management", "timeline_control"]
  
  remember {
    active_animations = {}
    animation_queue = []
    animation_config = {
      default_duration: 300,
      easing_functions: {
        linear: "linear",
        ease_in: "cubic-bezier(0.4, 0, 1, 1)",
        ease_out: "cubic-bezier(0, 0, 0.2, 1)",
        ease_in_out: "cubic-bezier(0.4, 0, 0.2, 1)"
      }
    }
  }

  brain {
    plan {
      plan = { action: "manage_animations" }
    }

    execute {
      if plan.action == "manage_animations" {
        // Process animation queue
        process_animation_queue()
        
        // Update active animations
        update_active_animations()
        
        // Clean up completed animations
        cleanup_completed_animations()
      }
    }
  }

  recipe "create_animation" {
    needs: element_id, animation_type, properties
    gives: animation_id
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          animation_id = generate_animation_id()
          
          animation = {
            id: animation_id,
            element_id: element_id,
            type: animation_type,
            properties: properties,
            start_time: get_timestamp(),
            duration: properties.duration || memory.animation_config.default_duration,
            easing: properties.easing || "ease_in_out",
            status: "active"
          }
          
          memory.active_animations[animation_id] = animation
          
          // Apply initial animation state
          apply_animation_state(animation)
        }
      }
    }
  }

  recipe "fade_in" {
    needs: element_id, duration
    gives: animation_id
    
    brain {
      plan {
        plan = { action: "fade" }
      }
      
      execute {
        if plan.action == "fade" {
          properties = {
            opacity: {
              from: 0,
              to: 1
            },
            duration: duration || 300,
            easing: "ease_out"
          }
          
          animation_id = create_animation(element_id, "fade_in", properties)
        }
      }
    }
  }

  recipe "slide_in" {
    needs: element_id, direction, duration
    gives: animation_id
    
    brain {
      plan {
        plan = { action: "slide" }
      }
      
      execute {
        if plan.action == "slide" {
          // Set initial position based on direction
          if direction === "left" {
            transform_from = "translateX(-100%)"
            transform_to = "translateX(0)"
          } else if direction === "right" {
            transform_from = "translateX(100%)"
            transform_to = "translateX(0)"
          } else if direction === "up" {
            transform_from = "translateY(-100%)"
            transform_to = "translateY(0)"
          } else if direction === "down" {
            transform_from = "translateY(100%)"
            transform_to = "translateY(0)"
          }
          
          properties = {
            transform: {
              from: transform_from,
              to: transform_to
            },
            duration: duration || 400,
            easing: "ease_out"
          }
          
          animation_id = create_animation(element_id, "slide_in", properties)
        }
      }
    }
  }

  recipe "update_active_animations" {
    needs: none
    gives: updated_count
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          updated_count = 0
          current_time = get_timestamp()
          
          for each animation_id in memory.active_animations {
            animation = memory.active_animations[animation_id]
            
            if animation.status === "active" {
              // Calculate animation progress
              elapsed_time = current_time - animation.start_time
              progress = Math.min(elapsed_time / animation.duration, 1)
              
              // Apply easing function
              eased_progress = apply_easing_function(progress, animation.easing)
              
              // Update element properties
              update_element_properties(animation.element_id, animation.properties, eased_progress)
              
              // Check if animation is complete
              if progress >= 1 {
                animation.status = "completed"
                animation.completed_at = current_time
              }
              
              updated_count += 1
            }
          }
        }
      }
    }
  }
}
```

## Step 5: Interactive Game Interface

```gx
helper "game_interface" {
  can_do: ["game_rendering", "input_handling", "score_tracking"]
  
  remember {
    game_state = {
      score: 0,
      level: 1,
      lives: 3,
      is_playing: false,
      high_score: 0
    }
    game_objects = {}
    input_handlers = {}
  }

  receive {
    from "user_interface" as "game_inputs" {
      type: "game_input"
      bind: memory.game_input
      on_receive: brain.handle_game_input
    }
  }

  brain {
    plan {
      if memory.game_input {
        plan = { action: "process_game_input" }
      } else {
        plan = { action: "update_game_state" }
      }
    }

    execute {
      if plan.action == "process_game_input" {
        input_type = memory.game_input.type
        
        if input_type === "key_press" {
          handle_key_press(memory.game_input.key)
        } else if input_type === "mouse_click" {
          handle_mouse_click(memory.game_input.x, memory.game_input.y)
        } else if input_type === "touch" {
          handle_touch(memory.game_input.x, memory.game_input.y)
        }
        
      } else if plan.action == "update_game_state" {
        if memory.game_state.is_playing {
          update_game_objects()
          check_collisions()
          update_score()
          check_game_over()
        }
      }
    }
  }

  recipe "handle_key_press" {
    needs: key
    gives: action_taken
    
    brain {
      plan {
        plan = { action: "handle" }
      }
      
      execute {
        if plan.action == "handle" {
          action_taken = false
          
          if key === "ArrowLeft" || key === "a" {
            move_player_left()
            action_taken = true
          } else if key === "ArrowRight" || key === "d" {
            move_player_right()
            action_taken = true
          } else if key === "ArrowUp" || key === "w" {
            move_player_up()
            action_taken = true
          } else if key === "ArrowDown" || key === "s" {
            move_player_down()
            action_taken = true
          } else if key === " " {
            // Spacebar - jump or shoot
            if memory.game_state.is_playing {
              player_jump_or_shoot()
              action_taken = true
            }
          } else if key === "Enter" {
            // Enter - start/pause game
            toggle_game_pause()
            action_taken = true
          }
        }
      }
    }
  }

  recipe "update_game_objects" {
    needs: none
    gives: updated_objects
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          updated_objects = {}
          
          // Update player position
          if memory.game_objects.player {
            update_player_movement()
          }
          
          // Update enemies
          for each enemy_id in memory.game_objects.enemies {
            update_enemy_behavior(enemy_id)
          }
          
          // Update projectiles
          for each projectile_id in memory.game_objects.projectiles {
            update_projectile_movement(projectile_id)
          }
          
          // Update power-ups
          for each powerup_id in memory.game_objects.powerups {
            update_powerup_animation(powerup_id)
          }
          
          updated_objects = memory.game_objects
        }
      }
    }
  }

  recipe "check_collisions" {
    needs: none
    gives: collision_events
    
    brain {
      plan {
        plan = { action: "check" }
      }
      
      execute {
        if plan.action == "check" {
          collision_events = []
          
          player = memory.game_objects.player
          if player {
            // Check player-enemy collisions
            for each enemy_id in memory.game_objects.enemies {
              enemy = memory.game_objects.enemies[enemy_id]
              if check_collision(player, enemy) {
                collision_events.push({
                  type: "player_enemy_collision",
                  player_id: player.id,
                  enemy_id: enemy_id
                })
              }
            }
            
            // Check player-powerup collisions
            for each powerup_id in memory.game_objects.powerups {
              powerup = memory.game_objects.powerups[powerup_id]
              if check_collision(player, powerup) {
                collision_events.push({
                  type: "player_powerup_collision",
                  player_id: player.id,
                  powerup_id: powerup_id
                })
              }
            }
            
            // Check projectile-enemy collisions
            for each projectile_id in memory.game_objects.projectiles {
              projectile = memory.game_objects.projectiles[projectile_id]
              for each enemy_id in memory.game_objects.enemies {
                enemy = memory.game_objects.enemies[enemy_id]
                if check_collision(projectile, enemy) {
                  collision_events.push({
                    type: "projectile_enemy_collision",
                    projectile_id: projectile_id,
                    enemy_id: enemy_id
                  })
                }
              }
            }
          }
          
          // Process collision events
          for each event in collision_events {
            process_collision_event(event)
          }
        }
      }
    }
  }
}
```

## Running Interactive Applications

1. **Save the complete application** to a file:
   ```bash
   # Save all helpers to interactive_app.gx
   # (Include all the helper code above)
   ```

2. **Run the application**:
   ```bash
   ./bin/gx interactive_app.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: interactive_app.gx
     📊 File size: 12500 bytes
   
     🚀 Executing GX Runtime: interactive_app.gx
     🧠 Initializing cognitive runtime...
     📊 Found 4 helpers with 20 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     Interactive Application initialized successfully!
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Advanced Features to Add

1. **Drag and Drop**: Implement drag and drop functionality
2. **Gesture Recognition**: Add touch and mouse gesture support
3. **Accessibility**: Implement accessibility features
4. **Responsive Design**: Create responsive layouts
5. **Progressive Web App**: Add PWA capabilities
6. **Offline Support**: Implement offline functionality

## Practice Exercises

1. **Create a calculator** with interactive buttons and real-time display
2. **Build a todo list** with add, edit, and delete functionality
3. **Make a color picker** with live preview
4. **Create a slider component** with real-time value updates
5. **Build a modal system** with animations and backdrop

## Next Steps

Now that you understand interactive applications, you're ready to:
- [Create Web Applications](05_web_applications.md)
- [Build AI Applications](06_ai_applications.md)
- [Build a ChatGPT Clone](07_chatgpt_clone.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: www.devjsx.com** 