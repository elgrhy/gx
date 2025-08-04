# 🌐 Building Web Applications with GX

## Overview

In this tutorial, we'll build complete web applications using GX, including frontend interfaces, backend APIs, database integration, and deployment. We'll learn how to create full-stack applications with GX's brain-first approach.

## Architecture Overview

Our web applications will include:
- **Frontend Interface**: User-facing web pages and components
- **Backend API**: Server-side logic and data processing
- **Database Integration**: Data storage and retrieval
- **Authentication System**: User login and session management
- **API Gateway**: Request routing and load balancing
- **Deployment System**: Application deployment and scaling

## Step 1: Web Server and API

```gx
helper "web_server" {
  can_do: ["http_server", "api_handling", "request_routing"]
  
  remember {
    server_config = {
      port: 3000,
      host: "localhost",
      routes: {},
      middleware: []
    }
    active_connections = []
    request_log = []
  }

  brain {
    plan {
      plan = { action: "start_server" }
    }

    execute {
      if plan.action == "start_server" {
        // Initialize server
        initialize_server()
        
        // Register routes
        register_default_routes()
        
        // Start listening
        start_listening()
        
        output("Web server started on http://" + memory.server_config.host + ":" + memory.server_config.port)
      }
    }
  }

  recipe "initialize_server" {
    needs: none
    gives: server_status
    
    brain {
      plan {
        plan = { action: "initialize" }
      }
      
      execute {
        if plan.action == "initialize" {
          // Set up server configuration
          memory.server_config.routes = {
            "GET": {},
            "POST": {},
            "PUT": {},
            "DELETE": {}
          }
          
          // Add default middleware
          memory.server_config.middleware = [
            "cors",
            "body_parser",
            "logger",
            "auth"
          ]
          
          server_status = {
            status: "initialized",
            config: memory.server_config
          }
        }
      }
    }
  }

  recipe "register_default_routes" {
    needs: none
    gives: routes_registered
    
    brain {
      plan {
        plan = { action: "register" }
      }
      
      execute {
        if plan.action == "register" {
          // Register API routes
          register_route("GET", "/api/health", "health_check")
          register_route("GET", "/api/users", "get_users")
          register_route("POST", "/api/users", "create_user")
          register_route("GET", "/api/users/:id", "get_user")
          register_route("PUT", "/api/users/:id", "update_user")
          register_route("DELETE", "/api/users/:id", "delete_user")
          
          // Register static file routes
          register_route("GET", "/", "serve_index")
          register_route("GET", "/static/*", "serve_static")
          
          routes_registered = Object.keys(memory.server_config.routes.GET).length + 
                             Object.keys(memory.server_config.routes.POST).length
        }
      }
    }
  }

  recipe "handle_request" {
    needs: request
    gives: response
    
    brain {
      plan {
        plan = { action: "handle" }
      }
      
      execute {
        if plan.action == "handle" {
          method = request.method
          url = request.url
          headers = request.headers
          body = request.body
          
          // Log request
          log_request(method, url, headers)
          
          // Apply middleware
          processed_request = apply_middleware(request)
          
          // Route request
          route_handler = find_route_handler(method, url)
          
          if route_handler {
            // Execute route handler
            response = execute_route_handler(route_handler, processed_request)
          } else {
            // Return 404
            response = {
              status: 404,
              body: { error: "Route not found" },
              headers: { "Content-Type": "application/json" }
            }
          }
          
          // Apply response middleware
          response = apply_response_middleware(response)
        }
      }
    }
  }

  recipe "health_check" {
    needs: request
    gives: response
    
    brain {
      plan {
        plan = { action: "check" }
      }
      
      execute {
        if plan.action == "check" {
          response = {
            status: 200,
            body: {
              status: "healthy",
              timestamp: get_timestamp(),
              uptime: get_server_uptime(),
              active_connections: memory.active_connections.length
            },
            headers: { "Content-Type": "application/json" }
          }
        }
      }
    }
  }
}
```

## Step 2: Database Integration

```gx
helper "database_manager" {
  can_do: ["database_operations", "query_optimization", "connection_pooling"]
  
  remember {
    db_config = {
      host: "localhost",
      port: 5432,
      database: "gx_web_app",
      username: "gx_user",
      password: "secure_password"
    }
    connection_pool = []
    query_cache = {}
    active_connections = 0
  }

  brain {
    plan {
      plan = { action: "manage_database" }
    }

    execute {
      if plan.action == "manage_database" {
        // Initialize database connection
        initialize_database_connection()
        
        // Create tables if they don't exist
        create_database_tables()
        
        // Optimize queries
        optimize_query_performance()
        
        // Monitor database health
        monitor_database_health()
      }
    }
  }

  recipe "initialize_database_connection" {
    needs: none
    gives: connection_status
    
    brain {
      plan {
        plan = { action: "initialize" }
      }
      
      execute {
        if plan.action == "initialize" {
          // Create connection pool
          for i in range(10) {
            connection = create_database_connection(memory.db_config)
            memory.connection_pool.push(connection)
          }
          
          connection_status = {
            status: "connected",
            pool_size: memory.connection_pool.length,
            active_connections: memory.active_connections
          }
        }
      }
    }
  }

  recipe "create_database_tables" {
    needs: none
    gives: tables_created
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          tables_created = []
          
          // Create users table
          users_table_sql = `
            CREATE TABLE IF NOT EXISTS users (
              id SERIAL PRIMARY KEY,
              username VARCHAR(50) UNIQUE NOT NULL,
              email VARCHAR(100) UNIQUE NOT NULL,
              password_hash VARCHAR(255) NOT NULL,
              created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
          `
          execute_sql(users_table_sql)
          tables_created.push("users")
          
          // Create posts table
          posts_table_sql = `
            CREATE TABLE IF NOT EXISTS posts (
              id SERIAL PRIMARY KEY,
              user_id INTEGER REFERENCES users(id),
              title VARCHAR(200) NOT NULL,
              content TEXT,
              created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
          `
          execute_sql(posts_table_sql)
          tables_created.push("posts")
          
          // Create comments table
          comments_table_sql = `
            CREATE TABLE IF NOT EXISTS comments (
              id SERIAL PRIMARY KEY,
              post_id INTEGER REFERENCES posts(id),
              user_id INTEGER REFERENCES users(id),
              content TEXT NOT NULL,
              created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
          `
          execute_sql(comments_table_sql)
          tables_created.push("comments")
        }
      }
    }
  }

  recipe "execute_query" {
    needs: sql, params
    gives: result
    
    brain {
      plan {
        plan = { action: "execute" }
      }
      
      execute {
        if plan.action == "execute" {
          // Get connection from pool
          connection = get_connection_from_pool()
          
          try {
            // Execute query
            result = connection.execute(sql, params)
            
            // Cache result if it's a SELECT query
            if sql.trim().toUpperCase().startsWith("SELECT") {
              cache_key = generate_cache_key(sql, params)
              memory.query_cache[cache_key] = {
                result: result,
                timestamp: get_timestamp(),
                ttl: 300 // 5 minutes
              }
            }
            
            // Return connection to pool
            return_connection_to_pool(connection)
            
          } catch error {
            // Handle database error
            log_database_error(error, sql, params)
            result = { error: "Database operation failed" }
          }
        }
      }
    }
  }

  recipe "get_users" {
    needs: filters
    gives: users
    
    brain {
      plan {
        plan = { action: "get" }
      }
      
      execute {
        if plan.action == "get" {
          sql = "SELECT id, username, email, created_at FROM users"
          params = []
          
          // Add filters
          if filters.limit {
            sql += " LIMIT ?"
            params.push(filters.limit)
          }
          
          if filters.offset {
            sql += " OFFSET ?"
            params.push(filters.offset)
          }
          
          result = execute_query(sql, params)
          users = result.rows || []
        }
      }
    }
  }

  recipe "create_user" {
    needs: user_data
    gives: new_user
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          // Hash password
          password_hash = hash_password(user_data.password)
          
          sql = `
            INSERT INTO users (username, email, password_hash)
            VALUES (?, ?, ?)
            RETURNING id, username, email, created_at
          `
          params = [user_data.username, user_data.email, password_hash]
          
          result = execute_query(sql, params)
          new_user = result.rows[0]
        }
      }
    }
  }
}
```

## Step 3: Authentication System

```gx
helper "auth_system" {
  can_do: ["user_authentication", "session_management", "authorization"]
  
  remember {
    auth_config = {
      jwt_secret: "your-secret-key",
      session_duration: 3600, // 1 hour
      bcrypt_rounds: 12
    }
    active_sessions = {}
    blacklisted_tokens = []
  }

  brain {
    plan {
      plan = { action: "manage_auth" }
    }

    execute {
      if plan.action == "manage_auth" {
        // Clean up expired sessions
        cleanup_expired_sessions()
        
        // Process authentication requests
        process_auth_requests()
        
        // Monitor security events
        monitor_security_events()
      }
    }
  }

  recipe "authenticate_user" {
    needs: credentials
    gives: auth_result
    
    brain {
      plan {
        plan = { action: "authenticate" }
      }
      
      execute {
        if plan.action == "authenticate" {
          username = credentials.username
          password = credentials.password
          
          // Get user from database
          user = get_user_by_username(username)
          
          if user && verify_password(password, user.password_hash) {
            // Generate JWT token
            token = generate_jwt_token(user)
            
            // Create session
            session = create_user_session(user.id, token)
            
            auth_result = {
              success: true,
              user: {
                id: user.id,
                username: user.username,
                email: user.email
              },
              token: token,
              session_id: session.id
            }
          } else {
            auth_result = {
              success: false,
              error: "Invalid credentials"
            }
          }
        }
      }
    }
  }

  recipe "verify_token" {
    needs: token
    gives: verification_result
    
    brain {
      plan {
        plan = { action: "verify" }
      }
      
      execute {
        if plan.action == "verify" {
          // Check if token is blacklisted
          if memory.blacklisted_tokens.includes(token) {
            verification_result = {
              valid: false,
              error: "Token is blacklisted"
            }
            return verification_result
          }
          
          try {
            // Verify JWT token
            decoded = verify_jwt_token(token)
            
            // Check if session exists
            session = get_session_by_token(token)
            
            if session && session.active {
              verification_result = {
                valid: true,
                user_id: decoded.user_id,
                session: session
              }
            } else {
              verification_result = {
                valid: false,
                error: "Session not found or inactive"
              }
            }
          } catch error {
            verification_result = {
              valid: false,
              error: "Invalid token"
            }
          }
        }
      }
    }
  }

  recipe "logout_user" {
    needs: token
    gives: logout_result
    
    brain {
      plan {
        plan = { action: "logout" }
      }
      
      execute {
        if plan.action == "logout" {
          // Invalidate session
          invalidate_session(token)
          
          // Add token to blacklist
          memory.blacklisted_tokens.push(token)
          
          logout_result = {
            success: true,
            message: "User logged out successfully"
          }
        }
      }
    }
  }

  recipe "generate_jwt_token" {
    needs: user
    gives: token
    
    brain {
      plan {
        plan = { action: "generate" }
      }
      
      execute {
        if plan.action == "generate" {
          payload = {
            user_id: user.id,
            username: user.username,
            iat: Math.floor(get_timestamp() / 1000),
            exp: Math.floor(get_timestamp() / 1000) + memory.auth_config.session_duration
          }
          
          token = sign_jwt(payload, memory.auth_config.jwt_secret)
        }
      }
    }
  }
}
```

## Step 4: Frontend Interface

```gx
helper "frontend_interface" {
  can_do: ["ui_rendering", "api_integration", "state_management"]
  
  remember {
    ui_state = {
      current_page: "home",
      user: null,
      posts: [],
      loading: false,
      error: null
    }
    api_endpoints = {
      base_url: "http://localhost:3000/api",
      endpoints: {
        login: "/auth/login",
        register: "/auth/register",
        posts: "/posts",
        users: "/users"
      }
    }
  }

  brain {
    plan {
      plan = { action: "render_interface" }
    }

    execute {
      if plan.action == "render_interface" {
        // Render current page
        render_current_page()
        
        // Handle user interactions
        handle_user_interactions()
        
        // Update UI state
        update_ui_state()
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
          
          if current_page === "home" {
            rendered_page = render_home_page()
          } else if current_page === "login" {
            rendered_page = render_login_page()
          } else if current_page === "register" {
            rendered_page = render_register_page()
          } else if current_page === "dashboard" {
            rendered_page = render_dashboard_page()
          } else if current_page === "posts" {
            rendered_page = render_posts_page()
          } else {
            rendered_page = render_404_page()
          }
        }
      }
    }
  }

  recipe "render_home_page" {
    needs: none
    gives: home_page
    
    brain {
      plan {
        plan = { action: "render" }
      }
      
      execute {
        if plan.action == "render" {
          home_page = {
            type: "page",
            title: "Welcome to GX Web App",
            content: {
              header: {
                type: "header",
                title: "GX Web Application",
                navigation: [
                  { text: "Home", url: "/" },
                  { text: "Posts", url: "/posts" },
                  { text: "Login", url: "/login" },
                  { text: "Register", url: "/register" }
                ]
              },
              main: {
                type: "main",
                sections: [
                  {
                    type: "hero",
                    title: "Welcome to GX Web Development",
                    subtitle: "Build powerful web applications with brain-first programming",
                    cta: {
                      text: "Get Started",
                      action: "navigate_to_register"
                    }
                  },
                  {
                    type: "features",
                    title: "Key Features",
                    items: [
                      { title: "Brain-First Design", description: "Built around cognitive processes" },
                      { title: "Real-time Updates", description: "Live data synchronization" },
                      { title: "Scalable Architecture", description: "Enterprise-ready infrastructure" }
                    ]
                  }
                ]
              },
              footer: {
                type: "footer",
                content: "© 2025 GX Web Application"
              }
            }
          }
        }
      }
    }
  }

  recipe "render_login_page" {
    needs: none
    gives: login_page
    
    brain {
      plan {
        plan = { action: "render" }
      }
      
      execute {
        if plan.action == "render" {
          login_page = {
            type: "page",
            title: "Login - GX Web App",
            content: {
              header: {
                type: "header",
                title: "Login",
                navigation: [
                  { text: "Home", url: "/" },
                  { text: "Register", url: "/register" }
                ]
              },
              main: {
                type: "main",
                sections: [
                  {
                    type: "form",
                    id: "login_form",
                    title: "Login to Your Account",
                    fields: [
                      {
                        type: "text",
                        name: "username",
                        label: "Username",
                        required: true,
                        placeholder: "Enter your username"
                      },
                      {
                        type: "password",
                        name: "password",
                        label: "Password",
                        required: true,
                        placeholder: "Enter your password"
                      }
                    ],
                    submit: {
                      text: "Login",
                      action: "submit_login"
                    }
                  }
                ]
              }
            }
          }
        }
      }
    }
  }

  recipe "handle_api_request" {
    needs: endpoint, method, data
    gives: response
    
    brain {
      plan {
        plan = { action: "request" }
      }
      
      execute {
        if plan.action == "request" {
          url = memory.api_endpoints.base_url + endpoint
          
          request_config = {
            method: method,
            headers: {
              "Content-Type": "application/json"
            }
          }
          
          // Add authentication token if available
          if memory.ui_state.user && memory.ui_state.user.token {
            request_config.headers["Authorization"] = "Bearer " + memory.ui_state.user.token
          }
          
          // Add request body for POST/PUT requests
          if data && (method === "POST" || method === "PUT") {
            request_config.body = JSON.stringify(data)
          }
          
          // Make API request
          response = make_http_request(url, request_config)
          
          // Handle response
          if response.status >= 200 && response.status < 300 {
            return response.data
          } else {
            throw new Error("API request failed: " + response.status)
          }
        }
      }
    }
  }

  recipe "submit_login" {
    needs: form_data
    gives: login_result
    
    brain {
      plan {
        plan = { action: "submit" }
      }
      
      execute {
        if plan.action == "submit" {
          try {
            // Make login API request
            response = handle_api_request("/auth/login", "POST", {
              username: form_data.username,
              password: form_data.password
            })
            
            // Update UI state with user data
            memory.ui_state.user = response.user
            memory.ui_state.current_page = "dashboard"
            
            // Store token in local storage
            store_auth_token(response.token)
            
            login_result = {
              success: true,
              message: "Login successful"
            }
            
          } catch error {
            login_result = {
              success: false,
              error: "Login failed: " + error.message
            }
          }
        }
      }
    }
  }
}
```

## Step 5: API Gateway

```gx
helper "api_gateway" {
  can_do: ["request_routing", "load_balancing", "rate_limiting"]
  
  remember {
    gateway_config = {
      port: 8080,
      routes: {},
      rate_limits: {},
      load_balancers: {}
    }
    request_stats = {
      total_requests: 0,
      successful_requests: 0,
      failed_requests: 0,
      average_response_time: 0
    }
  }

  brain {
    plan {
      plan = { action: "manage_gateway" }
    }

    execute {
      if plan.action == "manage_gateway" {
        // Initialize gateway
        initialize_gateway()
        
        // Route incoming requests
        route_requests()
        
        // Monitor performance
        monitor_gateway_performance()
        
        // Update load balancers
        update_load_balancers()
      }
    }
  }

  recipe "route_request" {
    needs: request
    gives: response
    
    brain {
      plan {
        plan = { action: "route" }
      }
      
      execute {
        if plan.action == "route" {
          // Extract request details
          method = request.method
          path = request.path
          headers = request.headers
          body = request.body
          
          // Apply rate limiting
          rate_limit_result = check_rate_limit(request)
          if !rate_limit_result.allowed {
            response = {
              status: 429,
              body: { error: "Rate limit exceeded" },
              headers: { "Retry-After": rate_limit_result.retry_after }
            }
            return response
          }
          
          // Find target service
          target_service = find_target_service(method, path)
          
          if target_service {
            // Forward request to target service
            response = forward_request(target_service, request)
            
            // Update statistics
            update_request_stats(response)
          } else {
            response = {
              status: 404,
              body: { error: "Service not found" }
            }
          }
        }
      }
    }
  }

  recipe "check_rate_limit" {
    needs: request
    gives: rate_limit_result
    
    brain {
      plan {
        plan = { action: "check" }
      }
      
      execute {
        if plan.action == "check" {
          client_ip = get_client_ip(request)
          endpoint = request.path
          
          // Get rate limit configuration
          rate_limit_config = memory.gateway_config.rate_limits[endpoint] || {
            requests_per_minute: 100,
            burst_size: 10
          }
          
          // Check current request count
          current_requests = get_request_count(client_ip, endpoint)
          
          if current_requests < rate_limit_config.requests_per_minute {
            rate_limit_result = {
              allowed: true,
              remaining: rate_limit_config.requests_per_minute - current_requests
            }
          } else {
            rate_limit_result = {
              allowed: false,
              retry_after: 60
            }
          }
        }
      }
    }
  }

  recipe "load_balance_request" {
    needs: service_name
    gives: target_instance
    
    brain {
      plan {
        plan = { action: "balance" }
      }
      
      execute {
        if plan.action == "balance" {
          load_balancer = memory.gateway_config.load_balancers[service_name]
          
          if load_balancer {
            // Get available instances
            available_instances = get_healthy_instances(service_name)
            
            if available_instances.length > 0 {
              // Apply load balancing algorithm
              if load_balancer.algorithm === "round_robin" {
                target_instance = get_next_instance_round_robin(available_instances)
              } else if load_balancer.algorithm === "least_connections" {
                target_instance = get_least_loaded_instance(available_instances)
              } else if load_balancer.algorithm === "weighted" {
                target_instance = get_weighted_instance(available_instances)
              } else {
                // Default to round robin
                target_instance = available_instances[0]
              }
            }
          }
        }
      }
    }
  }
}
```

## Running Web Applications

1. **Save the complete application** to a file:
   ```bash
   # Save all helpers to web_app.gx
   # (Include all the helper code above)
   ```

2. **Run the application**:
   ```bash
   ./bin/gx web_app.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: web_app.gx
     📊 File size: 16800 bytes
   
     🚀 Executing GX Runtime: web_app.gx
     🧠 Initializing cognitive runtime...
     📊 Found 5 helpers with 25 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     Web server started on http://localhost:3000
     API Gateway started on http://localhost:8080
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Advanced Features to Add

1. **Microservices**: Split into multiple services
2. **Caching**: Implement Redis caching
3. **CDN Integration**: Add content delivery network
4. **SSL/TLS**: Add HTTPS support
5. **Monitoring**: Add application monitoring
6. **CI/CD**: Implement continuous deployment

## Practice Exercises

1. **Build a blog system** with posts, comments, and user management
2. **Create an e-commerce API** with products, orders, and payments
3. **Make a real-time chat application** with WebSocket support
4. **Build a file upload service** with image processing
5. **Create a RESTful API** with full CRUD operations

## Next Steps

Now that you understand web applications, you're ready to:
- [Build AI Applications](06_ai_applications.md)
- [Build a ChatGPT Clone](07_chatgpt_clone.md)
- [Build a TikTok Clone](08_tiktok_clone.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 