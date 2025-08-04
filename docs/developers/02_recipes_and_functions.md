# 📚 Recipes and Functions in GX

## What are Recipes?

In GX, **recipes** are reusable functions that can be called by helpers. They follow the same brain process cycle but are designed to be modular and reusable.

## Basic Recipe Structure

```gx
recipe "recipe_name" {
  needs: parameter1, parameter2
  gives: result
  
  brain {
    plan {
      plan = { action: "process_data" }
    }
    
    execute {
      if plan.action == "process_data" {
        // Process the input parameters
        result = process(parameter1, parameter2)
      }
    }
  }
}
```

## Simple Recipe Examples

### Calculator Recipe

```gx
helper "calculator" {
  can_do: ["mathematics"]
  
  remember {
    result = 0
  }

  brain {
    plan {
      plan = { action: "calculate" }
    }
    
    execute {
      if plan.action == "calculate" {
        // Call the add recipe
        memory.result = add_numbers(10, 5)
        output("10 + 5 = " + memory.result)
        
        // Call the multiply recipe
        memory.result = multiply_numbers(4, 6)
        output("4 * 6 = " + memory.result)
      }
    }
    
    communicate {
      broadcast "calculation_complete"
    }
  }

  recipe "add_numbers" {
    needs: a, b
    gives: sum
    
    brain {
      plan {
        plan = { action: "add" }
      }
      
      execute {
        if plan.action == "add" {
          sum = a + b
        }
      }
    }
  }

  recipe "multiply_numbers" {
    needs: a, b
    gives: product
    
    brain {
      plan {
        plan = { action: "multiply" }
      }
      
      execute {
        if plan.action == "multiply" {
          product = a * b
        }
      }
    }
  }
}
```

### String Processing Recipe

```gx
helper "text_processor" {
  can_do: ["text_processing"]
  
  remember {
    processed_text = ""
  }

  brain {
    plan {
      plan = { action: "process_text" }
    }
    
    execute {
      if plan.action == "process_text" {
        original_text = "hello world"
        
        // Call string processing recipes
        memory.processed_text = capitalize_text(original_text)
        output("Capitalized: " + memory.processed_text)
        
        memory.processed_text = reverse_text(original_text)
        output("Reversed: " + memory.processed_text)
        
        word_count = count_words(original_text)
        output("Word count: " + word_count)
      }
    }
    
    communicate {
      broadcast "text_processing_complete"
    }
  }

  recipe "capitalize_text" {
    needs: text
    gives: capitalized
    
    brain {
      plan {
        plan = { action: "capitalize" }
      }
      
      execute {
        if plan.action == "capitalize" {
          capitalized = text.to_uppercase()
        }
      }
    }
  }

  recipe "reverse_text" {
    needs: text
    gives: reversed
    
    brain {
      plan {
        plan = { action: "reverse" }
      }
      
      execute {
        if plan.action == "reverse" {
          reversed = ""
          for i in range(text.length - 1, -1, -1) {
            reversed += text[i]
          }
        }
      }
    }
  }

  recipe "count_words" {
    needs: text
    gives: count
    
    brain {
      plan {
        plan = { action: "count" }
      }
      
      execute {
        if plan.action == "count" {
          words = text.split(" ")
          count = words.length
        }
      }
    }
  }
}
```

## Advanced Recipe Concepts

### Recipe with Multiple Parameters

```gx
helper "advanced_calculator" {
  can_do: ["advanced_math"]
  
  remember {
    results = []
  }

  brain {
    plan {
      plan = { action: "perform_calculations" }
    }
    
    execute {
      if plan.action == "perform_calculations" {
        // Calculate area of different shapes
        circle_area = calculate_circle_area(5)
        rectangle_area = calculate_rectangle_area(10, 8)
        triangle_area = calculate_triangle_area(6, 4)
        
        memory.results = [circle_area, rectangle_area, triangle_area]
        
        output("Circle area: " + circle_area)
        output("Rectangle area: " + rectangle_area)
        output("Triangle area: " + triangle_area)
      }
    }
    
    communicate {
      broadcast "calculations_complete"
    }
  }

  recipe "calculate_circle_area" {
    needs: radius
    gives: area
    
    brain {
      plan {
        plan = { action: "calculate_area" }
      }
      
      execute {
        if plan.action == "calculate_area" {
          pi = 3.14159
          area = pi * radius * radius
        }
      }
    }
  }

  recipe "calculate_rectangle_area" {
    needs: width, height
    gives: area
    
    brain {
      plan {
        plan = { action: "calculate_area" }
      }
      
      execute {
        if plan.action == "calculate_area" {
          area = width * height
        }
      }
    }
  }

  recipe "calculate_triangle_area" {
    needs: base, height
    gives: area
    
    brain {
      plan {
        plan = { action: "calculate_area" }
      }
      
      execute {
        if plan.action == "calculate_area" {
          area = (base * height) / 2
        }
      }
    }
  }
}
```

### Recipe with Complex Data Structures

```gx
helper "data_processor" {
  can_do: ["data_analysis"]
  
  remember {
    processed_data = {}
  }

  brain {
    plan {
      plan = { action: "analyze_data" }
    }
    
    execute {
      if plan.action == "analyze_data" {
        // Sample data
        students = [
          { name: "Alice", scores: [85, 92, 78] },
          { name: "Bob", scores: [90, 88, 95] },
          { name: "Charlie", scores: [75, 80, 85] }
        ]
        
        // Process the data
        memory.processed_data = analyze_student_data(students)
        
        output("Analysis complete:")
        output("Average scores: " + memory.processed_data.averages)
        output("Top student: " + memory.processed_data.top_student)
        output("Class average: " + memory.processed_data.class_average)
      }
    }
    
    communicate {
      broadcast "data_analysis_complete"
    }
  }

  recipe "analyze_student_data" {
    needs: students
    gives: analysis
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          analysis = {
            averages: {},
            top_student: "",
            class_average: 0,
            total_score: 0,
            student_count: students.length
          }
          
          // Calculate individual averages
          for each student in students {
            student_average = calculate_average(student.scores)
            analysis.averages[student.name] = student_average
            analysis.total_score += student_average
          }
          
          // Find top student
          top_score = 0
          for each student_name in analysis.averages {
            if analysis.averages[student_name] > top_score {
              top_score = analysis.averages[student_name]
              analysis.top_student = student_name
            }
          }
          
          // Calculate class average
          analysis.class_average = analysis.total_score / analysis.student_count
        }
      }
    }
  }

  recipe "calculate_average" {
    needs: numbers
    gives: average
    
    brain {
      plan {
        plan = { action: "calculate" }
      }
      
      execute {
        if plan.action == "calculate" {
          sum = 0
          for each number in numbers {
            sum += number
          }
          average = sum / numbers.length
        }
      }
    }
  }
}
```

## Recipe Best Practices

### 1. Clear Naming

```gx
// ❌ Bad naming
recipe "calc" {
  needs: x, y
  gives: z
}

// ✅ Good naming
recipe "calculate_percentage" {
  needs: value, total
  gives: percentage
}
```

### 2. Single Responsibility

```gx
// ❌ Recipe doing too much
recipe "process_user_data" {
  needs: user_data
  gives: result
  
  brain {
    execute {
      // Validates, formats, calculates, and saves - too many responsibilities
    }
  }
}

// ✅ Separate recipes for each responsibility
recipe "validate_user_data" {
  needs: user_data
  gives: is_valid
}

recipe "format_user_data" {
  needs: user_data
  gives: formatted_data
}

recipe "calculate_user_metrics" {
  needs: user_data
  gives: metrics
}
```

### 3. Error Handling

```gx
recipe "safe_division" {
  needs: numerator, denominator
  gives: result
  
  brain {
    plan {
      plan = { action: "divide" }
    }
    
    execute {
      if plan.action == "divide" {
        if denominator == 0 {
          result = { error: "Division by zero", success: false }
        } else {
          result = { value: numerator / denominator, success: true }
        }
      }
    }
  }
}
```

## Recursive Recipes

```gx
helper "recursive_examples" {
  can_do: ["recursion"]
  
  brain {
    plan {
      plan = { action: "demonstrate_recursion" }
    }
    
    execute {
      if plan.action == "demonstrate_recursion" {
        // Calculate factorial
        factorial_5 = calculate_factorial(5)
        output("5! = " + factorial_5)
        
        // Calculate Fibonacci
        fib_10 = calculate_fibonacci(10)
        output("Fibonacci(10) = " + fib_10)
      }
    }
  }

  recipe "calculate_factorial" {
    needs: n
    gives: factorial
    
    brain {
      plan {
        plan = { action: "calculate" }
      }
      
      execute {
        if plan.action == "calculate" {
          if n <= 1 {
            factorial = 1
          } else {
            factorial = n * calculate_factorial(n - 1)
          }
        }
      }
    }
  }

  recipe "calculate_fibonacci" {
    needs: n
    gives: fibonacci
    
    brain {
      plan {
        plan = { action: "calculate" }
      }
      
      execute {
        if plan.action == "calculate" {
          if n <= 1 {
            fibonacci = n
          } else {
            fibonacci = calculate_fibonacci(n - 1) + calculate_fibonacci(n - 2)
          }
        }
      }
    }
  }
}
```

## Practice Exercises

1. **Create a recipe** that finds the maximum value in an array
2. **Build a recipe** that sorts an array of numbers
3. **Make a recipe** that checks if a string is a palindrome
4. **Create a recipe** that converts between different temperature scales
5. **Build a recipe** that calculates compound interest

## Next Steps

Now that you understand recipes, you're ready to:
- [Learn about Message Communication](03_message_communication.md)
- [Build Interactive Applications](04_interactive_applications.md)
- [Create Web Applications](05_web_applications.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 