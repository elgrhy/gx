# 🛒 Building an E-commerce System with GX

## Overview

In this tutorial, we'll build a complete e-commerce system using GX, including product management, shopping cart, order processing, payment integration, and inventory management. We'll learn how to create a scalable online shopping platform with GX's brain-first approach.

## Architecture Overview

Our e-commerce system will include:
- **Product Management**: Product catalog, categories, and inventory
- **Shopping Cart**: Cart management and checkout process
- **Order Processing**: Order management and fulfillment
- **Payment System**: Payment processing and security
- **User Management**: Customer accounts and preferences
- **Analytics**: Sales analytics and business intelligence

## Step 1: Product Management System

```gx
helper "product_management" {
  can_do: ["product_catalog", "inventory_management", "category_management"]
  
  remember {
    products = {}
    categories = {}
    inventory = {}
    product_analytics = {}
  }

  brain {
    plan {
      plan = { action: "manage_products" }
    }

    execute {
      if plan.action == "manage_products" {
        // Process product updates
        process_product_updates()
        
        // Manage inventory levels
        manage_inventory_levels()
        
        // Update product analytics
        update_product_analytics()
        
        // Optimize product recommendations
        optimize_product_recommendations()
      }
    }
  }

  recipe "create_product" {
    needs: product_data
    gives: product_result
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          // Generate product ID
          product_id = generate_product_id()
          
          // Create product object
          product = {
            id: product_id,
            name: product_data.name,
            description: product_data.description,
            price: product_data.price,
            sale_price: product_data.sale_price || null,
            category_id: product_data.category_id,
            images: product_data.images || [],
            attributes: product_data.attributes || {},
            tags: product_data.tags || [],
            sku: product_data.sku,
            created_at: get_timestamp(),
            updated_at: get_timestamp(),
            status: "active",
            featured: product_data.featured || false,
            rating: 0,
            review_count: 0
          }
          
          // Create inventory record
          inventory_record = {
            product_id: product_id,
            quantity: product_data.initial_quantity || 0,
            low_stock_threshold: product_data.low_stock_threshold || 10,
            reserved_quantity: 0,
            last_updated: get_timestamp()
          }
          
          // Save product and inventory
          memory.products[product_id] = product
          memory.inventory[product_id] = inventory_record
          
          product_result = {
            success: true,
            product_id: product_id,
            message: "Product created successfully"
          }
        }
      }
    }
  }

  recipe "update_inventory" {
    needs: product_id, quantity_change, change_type
    gives: inventory_result
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          inventory = memory.inventory[product_id]
          
          if inventory {
            if change_type === "add" {
              inventory.quantity += quantity_change
            } else if change_type === "subtract" {
              inventory.quantity -= quantity_change
            } else if change_type === "reserve" {
              inventory.reserved_quantity += quantity_change
            } else if change_type === "release" {
              inventory.reserved_quantity -= quantity_change
            }
            
            inventory.last_updated = get_timestamp()
            
            // Check low stock alert
            if inventory.quantity <= inventory.low_stock_threshold {
              send_low_stock_alert(product_id, inventory.quantity)
            }
            
            inventory_result = {
              success: true,
              new_quantity: inventory.quantity,
              reserved_quantity: inventory.reserved_quantity
            }
          } else {
            inventory_result = {
              success: false,
              error: "Product inventory not found"
            }
          }
        }
      }
    }
  }

  recipe "search_products" {
    needs: search_query, filters
    gives: search_results
    
    brain {
      plan {
        plan = { action: "search" }
      }
      
      execute {
        if plan.action == "search" {
          search_results = []
          
          for each product_id in memory.products {
            product = memory.products[product_id]
            
            // Check if product matches search criteria
            if product.status === "active" {
              matches_search = check_product_match(product, search_query, filters)
              
              if matches_search {
                // Add inventory information
                inventory = memory.inventory[product_id]
                product_with_inventory = {
                  ...product,
                  available_quantity: inventory ? inventory.quantity - inventory.reserved_quantity : 0,
                  in_stock: inventory ? (inventory.quantity - inventory.reserved_quantity) > 0 : false
                }
                
                search_results.push(product_with_inventory)
              }
            }
          }
          
          // Sort results by relevance
          search_results = sort_search_results(search_results, search_query)
        }
      }
    }
  }
}
```

## Step 2: Shopping Cart System

```gx
helper "shopping_cart" {
  can_do: ["cart_management", "checkout_process", "price_calculation"]
  
  remember {
    user_carts = {}
    cart_sessions = {}
    price_rules = {}
    discount_codes = {}
  }

  brain {
    plan {
      plan = { action: "manage_carts" }
    }

    execute {
      if plan.action == "manage_carts" {
        // Process cart updates
        process_cart_updates()
        
        // Apply price rules
        apply_price_rules()
        
        // Clean up expired carts
        cleanup_expired_carts()
        
        // Update cart analytics
        update_cart_analytics()
      }
    }
  }

  recipe "add_to_cart" {
    needs: user_id, product_id, quantity
    gives: cart_result
    
    brain {
      plan {
        plan = { action: "add" }
      }
      
      execute {
        if plan.action == "add" {
          // Get or create user cart
          if !memory.user_carts[user_id] {
            memory.user_carts[user_id] = {
              user_id: user_id,
              items: [],
              created_at: get_timestamp(),
              updated_at: get_timestamp()
            }
          }
          
          cart = memory.user_carts[user_id]
          
          // Check product availability
          inventory = memory.inventory[product_id]
          available_quantity = inventory ? inventory.quantity - inventory.reserved_quantity : 0
          
          if available_quantity >= quantity {
            // Check if item already in cart
            existing_item = find_cart_item(cart, product_id)
            
            if existing_item {
              // Update existing item quantity
              existing_item.quantity += quantity
              existing_item.updated_at = get_timestamp()
            } else {
              // Add new item to cart
              product = memory.products[product_id]
              cart_item = {
                product_id: product_id,
                name: product.name,
                price: product.price,
                sale_price: product.sale_price,
                quantity: quantity,
                added_at: get_timestamp(),
                updated_at: get_timestamp()
              }
              
              cart.items.push(cart_item)
            }
            
            cart.updated_at = get_timestamp()
            
            // Reserve inventory
            update_inventory(product_id, quantity, "reserve")
            
            cart_result = {
              success: true,
              message: "Item added to cart successfully",
              cart_total: calculate_cart_total(cart)
            }
          } else {
            cart_result = {
              success: false,
              error: "Insufficient inventory",
              available_quantity: available_quantity
            }
          }
        }
      }
    }
  }

  recipe "update_cart_item" {
    needs: user_id, product_id, new_quantity
    gives: update_result
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          cart = memory.user_carts[user_id]
          
          if cart {
            cart_item = find_cart_item(cart, product_id)
            
            if cart_item {
              old_quantity = cart_item.quantity
              quantity_change = new_quantity - old_quantity
              
              // Check inventory availability
              inventory = memory.inventory[product_id]
              available_quantity = inventory ? inventory.quantity - inventory.reserved_quantity : 0
              
              if available_quantity >= quantity_change {
                // Update item quantity
                cart_item.quantity = new_quantity
                cart_item.updated_at = get_timestamp()
                cart.updated_at = get_timestamp()
                
                // Update inventory reservation
                if quantity_change > 0 {
                  update_inventory(product_id, quantity_change, "reserve")
                } else if quantity_change < 0 {
                  update_inventory(product_id, -quantity_change, "release")
                }
                
                update_result = {
                  success: true,
                  message: "Cart item updated successfully",
                  cart_total: calculate_cart_total(cart)
                }
              } else {
                update_result = {
                  success: false,
                  error: "Insufficient inventory for requested quantity"
                }
              }
            } else {
              update_result = {
                success: false,
                error: "Item not found in cart"
              }
            }
          } else {
            update_result = {
              success: false,
              error: "Cart not found"
            }
          }
        }
      }
    }
  }

  recipe "calculate_cart_total" {
    needs: cart
    gives: total
    
    brain {
      plan {
        plan = { action: "calculate" }
      }
      
      execute {
        if plan.action == "calculate" {
          subtotal = 0
          
          for each item in cart.items {
            price = item.sale_price || item.price
            subtotal += price * item.quantity
          }
          
          // Apply discounts
          discount_amount = calculate_discounts(cart)
          
          // Calculate tax
          tax_amount = calculate_tax(subtotal - discount_amount)
          
          // Calculate shipping
          shipping_cost = calculate_shipping(cart)
          
          total = {
            subtotal: subtotal,
            discount: discount_amount,
            tax: tax_amount,
            shipping: shipping_cost,
            total: subtotal - discount_amount + tax_amount + shipping_cost
          }
        }
      }
    }
  }
}
```

## Step 3: Order Processing System

```gx
helper "order_processing" {
  can_do: ["order_management", "fulfillment", "order_tracking"]
  
  remember {
    orders = {}
    order_statuses = {}
    fulfillment_centers = {}
    shipping_methods = {}
  }

  brain {
    plan {
      plan = { action: "process_orders" }
    }

    execute {
      if plan.action == "process_orders" {
        // Process new orders
        process_new_orders()
        
        // Update order statuses
        update_order_statuses()
        
        // Handle fulfillment
        handle_fulfillment()
        
        // Generate shipping labels
        generate_shipping_labels()
      }
    }
  }

  recipe "create_order" {
    needs: user_id, cart, shipping_info, payment_info
    gives: order_result
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          // Generate order ID
          order_id = generate_order_id()
          
          // Calculate order totals
          totals = calculate_cart_total(cart)
          
          // Create order object
          order = {
            id: order_id,
            user_id: user_id,
            items: cart.items,
            subtotal: totals.subtotal,
            discount: totals.discount,
            tax: totals.tax,
            shipping: totals.shipping,
            total: totals.total,
            shipping_address: shipping_info.address,
            billing_address: payment_info.billing_address,
            payment_method: payment_info.method,
            status: "pending_payment",
            created_at: get_timestamp(),
            updated_at: get_timestamp()
          }
          
          // Save order
          memory.orders[order_id] = order
          
          // Process payment
          payment_result = process_payment(order, payment_info)
          
          if payment_result.success {
            // Update order status
            order.status = "paid"
            order.payment_id = payment_result.payment_id
            
            // Reserve inventory
            reserve_order_inventory(order)
            
            // Send confirmation
            send_order_confirmation(user_id, order_id)
            
            order_result = {
              success: true,
              order_id: order_id,
              payment_id: payment_result.payment_id,
              message: "Order created and payment processed successfully"
            }
          } else {
            order.status = "payment_failed"
            order_result = {
              success: false,
              error: "Payment processing failed",
              payment_error: payment_result.error
            }
          }
        }
      }
    }
  }

  recipe "process_payment" {
    needs: order, payment_info
    gives: payment_result
    
    brain {
      plan {
        plan = { action: "process" }
      }
      
      execute {
        if plan.action == "process" {
          // Validate payment information
          validation_result = validate_payment_info(payment_info)
          
          if validation_result.is_valid {
            // Process payment through payment gateway
            gateway_result = process_payment_gateway(order, payment_info)
            
            if gateway_result.success {
              payment_result = {
                success: true,
                payment_id: gateway_result.payment_id,
                transaction_id: gateway_result.transaction_id
              }
            } else {
              payment_result = {
                success: false,
                error: gateway_result.error
              }
            }
          } else {
            payment_result = {
              success: false,
              error: "Invalid payment information",
              validation_errors: validation_result.errors
            }
          }
        }
      }
    }
  }

  recipe "update_order_status" {
    needs: order_id, new_status
    gives: status_result
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          order = memory.orders[order_id]
          
          if order {
            old_status = order.status
            order.status = new_status
            order.updated_at = get_timestamp()
            
            // Handle status-specific actions
            if new_status === "shipped" {
              // Generate tracking number
              tracking_number = generate_tracking_number(order_id)
              order.tracking_number = tracking_number
              
              // Send shipping notification
              send_shipping_notification(order.user_id, order_id, tracking_number)
            } else if new_status === "delivered" {
              // Mark inventory as sold
              mark_inventory_as_sold(order)
              
              // Send delivery confirmation
              send_delivery_confirmation(order.user_id, order_id)
            }
            
            status_result = {
              success: true,
              old_status: old_status,
              new_status: new_status,
              message: "Order status updated successfully"
            }
          } else {
            status_result = {
              success: false,
              error: "Order not found"
            }
          }
        }
      }
    }
  }
}
```

## Step 4: Payment System

```gx
helper "payment_system" {
  can_do: ["payment_processing", "security", "refund_handling"]
  
  remember {
    payment_methods = {}
    transactions = {}
    refunds = {}
    security_logs = {}
  }

  brain {
    plan {
      plan = { action: "manage_payments" }
    }

    execute {
      if plan.action == "manage_payments" {
        // Process payment requests
        process_payment_requests()
        
        // Handle refunds
        handle_refunds()
        
        // Monitor security
        monitor_payment_security()
        
        // Update transaction logs
        update_transaction_logs()
      }
    }
  }

  recipe "process_payment_gateway" {
    needs: order, payment_info
    gives: gateway_result
    
    brain {
      plan {
        plan = { action: "process" }
      }
      
      execute {
        if plan.action == "process" {
          // Validate payment amount
          if order.total <= 0 {
            gateway_result = {
              success: false,
              error: "Invalid payment amount"
            }
            return gateway_result
          }
          
          // Check for fraud
          fraud_check = perform_fraud_check(order, payment_info)
          
          if fraud_check.is_suspicious {
            gateway_result = {
              success: false,
              error: "Payment flagged for review",
              fraud_score: fraud_check.score
            }
            return gateway_result
          }
          
          // Process payment based on method
          if payment_info.method === "credit_card" {
            result = process_credit_card_payment(order, payment_info)
          } else if payment_info.method === "paypal" {
            result = process_paypal_payment(order, payment_info)
          } else if payment_info.method === "crypto" {
            result = process_crypto_payment(order, payment_info)
          } else {
            result = {
              success: false,
              error: "Unsupported payment method"
            }
          }
          
          if result.success {
            // Log successful transaction
            log_transaction(order.id, result.transaction_id, "success")
            
            gateway_result = {
              success: true,
              payment_id: result.payment_id,
              transaction_id: result.transaction_id,
              amount: order.total,
              currency: "USD"
            }
          } else {
            // Log failed transaction
            log_transaction(order.id, null, "failed", result.error)
            
            gateway_result = {
              success: false,
              error: result.error
            }
          }
        }
      }
    }
  }

  recipe "process_refund" {
    needs: order_id, refund_amount, reason
    gives: refund_result
    
    brain {
      plan {
        plan = { action: "refund" }
      }
      
      execute {
        if plan.action == "refund" {
          order = memory.orders[order_id]
          
          if order && order.status === "delivered" {
            // Validate refund amount
            if refund_amount > order.total {
              refund_result = {
                success: false,
                error: "Refund amount exceeds order total"
              }
              return refund_result
            }
            
            // Process refund through payment gateway
            gateway_refund = process_refund_gateway(order, refund_amount)
            
            if gateway_refund.success {
              // Create refund record
              refund_id = generate_refund_id()
              refund = {
                id: refund_id,
                order_id: order_id,
                amount: refund_amount,
                reason: reason,
                processed_at: get_timestamp(),
                status: "completed"
              }
              
              memory.refunds[refund_id] = refund
              
              // Update order status
              update_order_status(order_id, "refunded")
              
              // Send refund notification
              send_refund_notification(order.user_id, order_id, refund_amount)
              
              refund_result = {
                success: true,
                refund_id: refund_id,
                amount: refund_amount,
                message: "Refund processed successfully"
              }
            } else {
              refund_result = {
                success: false,
                error: "Refund processing failed",
                gateway_error: gateway_refund.error
              }
            }
          } else {
            refund_result = {
              success: false,
              error: "Order not eligible for refund"
            }
          }
        }
      }
    }
  }

  recipe "perform_fraud_check" {
    needs: order, payment_info
    gives: fraud_result
    
    brain {
      plan {
        plan = { action: "check" }
      }
      
      execute {
        if plan.action == "check" {
          fraud_score = 0
          risk_factors = []
          
          // Check for suspicious patterns
          if order.total > 1000 {
            fraud_score += 20
            risk_factors.push("high_amount")
          }
          
          if payment_info.ip_address {
            ip_risk = check_ip_risk(payment_info.ip_address)
            fraud_score += ip_risk.score
            if ip_risk.risk_factors.length > 0 {
              risk_factors.push(...ip_risk.risk_factors)
            }
          }
          
          if payment_info.email {
            email_risk = check_email_risk(payment_info.email)
            fraud_score += email_risk.score
            if email_risk.risk_factors.length > 0 {
              risk_factors.push(...email_risk.risk_factors)
            }
          }
          
          // Check for velocity (multiple orders in short time)
          velocity_risk = check_velocity_risk(order.user_id)
          fraud_score += velocity_risk.score
          if velocity_risk.risk_factors.length > 0 {
            risk_factors.push(...velocity_risk.risk_factors)
          }
          
          fraud_result = {
            is_suspicious: fraud_score > 50,
            score: fraud_score,
            risk_factors: risk_factors
          }
        }
      }
    }
  }
}
```

## Step 5: Analytics and Reporting

```gx
helper "ecommerce_analytics" {
  can_do: ["sales_analytics", "inventory_analytics", "customer_analytics"]
  
  remember {
    sales_data = {}
    inventory_reports = {}
    customer_insights = {}
    performance_metrics = {}
  }

  brain {
    plan {
      plan = { action: "generate_analytics" }
    }

    execute {
      if plan.action == "generate_analytics" {
        // Generate sales reports
        generate_sales_reports()
        
        // Analyze inventory performance
        analyze_inventory_performance()
        
        // Generate customer insights
        generate_customer_insights()
        
        // Update performance metrics
        update_performance_metrics()
      }
    }
  }

  recipe "generate_sales_report" {
    needs: start_date, end_date
    gives: sales_report
    
    brain {
      plan {
        plan = { action: "generate" }
      }
      
      execute {
        if plan.action == "generate" {
          // Get orders in date range
          orders_in_range = get_orders_in_range(start_date, end_date)
          
          // Calculate sales metrics
          total_sales = 0
          total_orders = orders_in_range.length
          average_order_value = 0
          top_products = {}
          
          for each order in orders_in_range {
            if order.status === "delivered" {
              total_sales += order.total
              
              // Track product sales
              for each item in order.items {
                if top_products[item.product_id] {
                  top_products[item.product_id].quantity += item.quantity
                  top_products[item.product_id].revenue += item.price * item.quantity
                } else {
                  top_products[item.product_id] = {
                    product_id: item.product_id,
                    name: item.name,
                    quantity: item.quantity,
                    revenue: item.price * item.quantity
                  }
                }
              }
            }
          }
          
          if total_orders > 0 {
            average_order_value = total_sales / total_orders
          }
          
          // Sort top products by revenue
          top_products_array = Object.values(top_products)
          top_products_array.sort((a, b) => b.revenue - a.revenue)
          
          sales_report = {
            period: {
              start: start_date,
              end: end_date
            },
            metrics: {
              total_sales: total_sales,
              total_orders: total_orders,
              average_order_value: average_order_value,
              conversion_rate: calculate_conversion_rate(start_date, end_date)
            },
            top_products: top_products_array.slice(0, 10),
            sales_by_day: generate_daily_sales_data(orders_in_range)
          }
        }
      }
    }
  }

  recipe "analyze_inventory_performance" {
    needs: none
    gives: inventory_report
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          inventory_report = {
            low_stock_items: [],
            out_of_stock_items: [],
            slow_moving_items: [],
            fast_moving_items: []
          }
          
          for each product_id in memory.inventory {
            inventory = memory.inventory[product_id]
            product = memory.products[product_id]
            
            // Check low stock
            if inventory.quantity <= inventory.low_stock_threshold {
              inventory_report.low_stock_items.push({
                product_id: product_id,
                name: product.name,
                current_quantity: inventory.quantity,
                threshold: inventory.low_stock_threshold
              })
            }
            
            // Check out of stock
            if inventory.quantity === 0 {
              inventory_report.out_of_stock_items.push({
                product_id: product_id,
                name: product.name,
                last_restocked: inventory.last_updated
              })
            }
            
            // Analyze movement
            movement_analysis = analyze_product_movement(product_id)
            
            if movement_analysis.movement_rate < 0.1 {
              inventory_report.slow_moving_items.push({
                product_id: product_id,
                name: product.name,
                movement_rate: movement_analysis.movement_rate
              })
            } else if movement_analysis.movement_rate > 0.5 {
              inventory_report.fast_moving_items.push({
                product_id: product_id,
                name: product.name,
                movement_rate: movement_analysis.movement_rate
              })
            }
          }
        }
      }
    }
  }
}
```

## Running the E-commerce System

1. **Save the complete application** to a file:
   ```bash
   # Save all helpers to ecommerce_system.gx
   # (Include all the helper code above)
   ```

2. **Run the application**:
   ```bash
   ./bin/gx ecommerce_system.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: ecommerce_system.gx
     📊 File size: 17200 bytes
   
     🚀 Executing GX Runtime: ecommerce_system.gx
     🧠 Initializing cognitive runtime...
     📊 Found 5 helpers with 25 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     E-commerce System initialized successfully!
     Product Management: Active
     Shopping Cart: Active
     Order Processing: Active
     Payment System: Active
     Analytics: Active
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Advanced Features to Add

1. **Multi-vendor Support**: Add marketplace functionality
2. **Subscription Management**: Implement recurring billing
3. **Advanced Shipping**: Add real-time shipping calculations
4. **Customer Support**: Integrate help desk and live chat
5. **Mobile App**: Create mobile e-commerce application
6. **AI Recommendations**: Add intelligent product recommendations

## Practice Exercises

1. **Build a product catalog** with categories and filters
2. **Create a shopping cart** with quantity updates and price calculations
3. **Implement a checkout process** with address validation
4. **Build an order management system** with status tracking
5. **Create a payment processing system** with multiple payment methods

## Next Steps

Now that you have an e-commerce system, you can:
- [Build a Gaming Platform](11_gaming_platform.md)
- [Build a Social Media Platform](09_social_media_platform.md)
- [Build a TikTok Clone](08_tiktok_clone.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 