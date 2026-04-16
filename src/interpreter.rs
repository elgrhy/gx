/// GX Interpreter — executes the AST produced by the parser.

use std::collections::HashMap;
use crate::ast::*;
use crate::value::Value;

// ── Control flow signals ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Signal {
    Return(Value),
    Error(String),
}

type ExecResult = Result<Value, Signal>;

// ── Environment (scope) ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Env {
    vars: HashMap<String, Value>,
    parent: Option<Box<Env>>,
}

impl Env {
    pub fn new() -> Self {
        Env { vars: HashMap::new(), parent: None }
    }

    pub fn child(parent: Env) -> Self {
        Env { vars: HashMap::new(), parent: Some(Box::new(parent)) }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.get(name) {
            Some(v.clone())
        } else if let Some(p) = &self.parent {
            p.get(name)
        } else {
            None
        }
    }

    pub fn set(&mut self, name: &str, val: Value) {
        // Set in the scope where it already exists, or current scope
        if self.vars.contains_key(name) {
            self.vars.insert(name.to_string(), val.clone());
        } else if let Some(p) = &mut self.parent {
            if p.get(name).is_some() {
                p.set(name, val);
                return;
            }
        }
        self.vars.insert(name.to_string(), val);
    }

    pub fn set_local(&mut self, name: &str, val: Value) {
        self.vars.insert(name.to_string(), val);
    }
}

// ── Interpreter ───────────────────────────────────────────────────────────────

pub struct Interpreter {
    /// Named helpers (available for calling)
    helpers: HashMap<String, HelperDef>,
    /// Global memory (shared across top-level)
    global_memory: HashMap<String, Value>,
    /// Emitted events (collected for inspection)
    pub events: Vec<(String, Vec<(String, Value)>)>,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            helpers: HashMap::new(),
            global_memory: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn run_program(&mut self, program: &Program) -> Result<(), String> {
        // Register all helpers
        for h in &program.helpers {
            self.helpers.insert(h.name.clone(), h.clone());
        }

        // Execute each helper's brain cycle in definition order
        for h in &program.helpers {
            self.run_helper(h).map_err(|e| match e {
                Signal::Error(msg) => msg,
                Signal::Return(_) => "Unexpected return at top level".to_string(),
            })?;
        }

        // Run top-level brain block if present
        if let Some(brain) = &program.top_level_brain {
            let mut env = self.make_env_with_memory(&HashMap::new());
            self.run_brain(brain, &mut env).map_err(|e| match e {
                Signal::Error(msg) => msg,
                Signal::Return(_) => "Unexpected return at top level".to_string(),
            })?;
        }

        Ok(())
    }

    fn run_helper(&mut self, helper: &HelperDef) -> Result<(), Signal> {
        // Build the helper's memory from its memory block
        let mut memory: HashMap<String, Value> = HashMap::new();
        let mut init_env = Env::new();

        for entry in &helper.memory {
            let val = self.eval_expr(&entry.value, &mut init_env)?;
            memory.insert(entry.key.clone(), val);
        }

        // Run the brain cycle
        if let Some(brain) = &helper.brain.clone() {
            let mut env = self.make_env_with_memory(&memory);
            self.run_brain(brain, &mut env)?;
            // Persist memory updates back
            if let Some(Value::Object(m)) = env.get("memory") {
                memory = m;
            }
        }

        Ok(())
    }

    fn make_env_with_memory(&self, memory: &HashMap<String, Value>) -> Env {
        let mut env = Env::new();
        env.set_local("memory", Value::Object(memory.clone()));
        env.set_local("plan", Value::Null);
        env.set_local("result", Value::Null);
        env
    }

    // ── Brain cycle ───────────────────────────────────────────────────────────

    fn run_brain(&mut self, brain: &BrainBlock, env: &mut Env) -> Result<Value, Signal> {
        // Phase 1: Plan
        self.run_stmts(&brain.plan, env)?;
        // Phase 2: Execute
        self.run_stmts(&brain.execute, env)?;
        // Phase 3: Remember
        self.run_stmts(&brain.remember, env)?;
        // Phase 4: Communicate
        self.run_stmts(&brain.communicate, env)?;
        Ok(Value::Null)
    }

    // ── Statement execution ───────────────────────────────────────────────────

    fn run_stmts(&mut self, stmts: &[Stmt], env: &mut Env) -> Result<Value, Signal> {
        let mut last = Value::Null;
        for stmt in stmts {
            last = self.run_stmt(stmt, env)?;
        }
        Ok(last)
    }

    fn run_stmt(&mut self, stmt: &Stmt, env: &mut Env) -> Result<Value, Signal> {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                let val = self.eval_expr(value, env)?;
                self.assign(target, val, env)?;
                Ok(Value::Null)
            }

            Stmt::PlusAssign { target, value, .. } => {
                let current = self.eval_expr(target, env)?;
                let rhs = self.eval_expr(value, env)?;
                let result = self.add_values(current, rhs)?;
                self.assign(target, result, env)?;
                Ok(Value::Null)
            }

            Stmt::If { branches, else_body, .. } => {
                for (cond, body) in branches {
                    let cv = self.eval_expr(cond, env)?;
                    if cv.is_truthy() {
                        return self.run_stmts(body, env);
                    }
                }
                if let Some(body) = else_body {
                    return self.run_stmts(body, env);
                }
                Ok(Value::Null)
            }

            Stmt::ForEach { var, iter, body, .. } => {
                let collection = self.eval_expr(iter, env)?;
                let items = collection.iter().map_err(|e| Signal::Error(e))?;
                let mut last = Value::Null;
                for item in items {
                    env.set_local(var, item);
                    last = self.run_stmts(body, env)?;
                }
                Ok(last)
            }

            Stmt::TryCatch { try_body, catch_var, catch_body, .. } => {
                match self.run_stmts(try_body, env) {
                    Ok(v) => Ok(v),
                    Err(Signal::Error(msg)) => {
                        env.set_local(catch_var, Value::Str(msg));
                        self.run_stmts(catch_body, env)
                    }
                    Err(other) => Err(other),
                }
            }

            Stmt::Emit { event, payload, .. } => {
                let mut resolved = Vec::new();
                for (k, expr) in payload {
                    let v = self.eval_expr(expr, env)?;
                    resolved.push((k.clone(), v));
                }
                self.events.push((event.clone(), resolved));
                Ok(Value::Null)
            }

            Stmt::Broadcast { event, .. } => {
                self.events.push((event.clone(), Vec::new()));
                Ok(Value::Null)
            }

            Stmt::Log { value, .. } => {
                let v = self.eval_expr(value, env)?;
                println!("{}", v);
                Ok(Value::Null)
            }

            Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                let v = self.eval_expr(value, env)?;
                println!("{}", v);
                Ok(Value::Null)
            }

            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(expr) => self.eval_expr(expr, env)?,
                    None => Value::Null,
                };
                Err(Signal::Return(v))
            }

            Stmt::Wait { ms, .. } => {
                let ms_val = self.eval_expr(ms, env)?;
                if let Some(n) = ms_val.as_number() {
                    std::thread::sleep(std::time::Duration::from_millis(n as u64));
                }
                Ok(Value::Null)
            }

            Stmt::Expr { expr, .. } => {
                self.eval_expr(expr, env)
            }
        }
    }

    // ── Assignment helper ─────────────────────────────────────────────────────

    fn assign(&mut self, target: &Expr, val: Value, env: &mut Env) -> Result<(), Signal> {
        match target {
            Expr::Ident(name) => {
                env.set(name, val);
                Ok(())
            }
            Expr::FieldAccess { object, field } => {
                match object.as_ref() {
                    Expr::Ident(obj_name) => {
                        let mut obj = env.get(obj_name).unwrap_or(Value::Object(HashMap::new()));
                        obj.set_field(field, val).map_err(|e| Signal::Error(e))?;
                        env.set(obj_name, obj);
                        Ok(())
                    }
                    Expr::FieldAccess { object: inner_obj, field: inner_field } => {
                        // memory.nested.key = val — get memory, get nested, set key
                        let outer_name = self.expr_root_name(inner_obj);
                        let mut outer = env.get(&outer_name).unwrap_or(Value::Null);
                        let mut inner = outer.get_field(inner_field);
                        inner.set_field(field, val).map_err(|e| Signal::Error(e))?;
                        outer.set_field(inner_field, inner).map_err(|e| Signal::Error(e))?;
                        env.set(&outer_name, outer);
                        Ok(())
                    }
                    _ => Err(Signal::Error(format!("Cannot assign to complex expression"))),
                }
            }
            _ => Err(Signal::Error(format!("Cannot assign to {:?}", target))),
        }
    }

    fn expr_root_name(&self, expr: &Expr) -> String {
        match expr {
            Expr::Ident(s) => s.clone(),
            Expr::FieldAccess { object, .. } => self.expr_root_name(object),
            _ => "unknown".to_string(),
        }
    }

    // Merge child env changes back to parent (memory, plan, result etc.)
    fn merge_env(&self, parent: &mut Env, child: Env) {
        for (k, v) in child.vars {
            parent.set(&k, v);
        }
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    fn eval_expr(&mut self, expr: &Expr, env: &mut Env) -> Result<Value, Signal> {
        match expr {
            Expr::Null => Ok(Value::Null),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Num(n) => Ok(Value::Number(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),

            Expr::Interpolated(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        InterpolatedPart::Literal(s) => result.push_str(s),
                        InterpolatedPart::Expr(e) => {
                            let v = self.eval_expr(e, env)?;
                            result.push_str(&v.to_string());
                        }
                    }
                }
                Ok(Value::Str(result))
            }

            Expr::Ident(name) => {
                Ok(env.get(name).unwrap_or(Value::Null))
            }

            Expr::FieldAccess { object, field } => {
                let obj = self.eval_expr(object, env)?;
                Ok(obj.get_field(field))
            }

            Expr::Index { object, index } => {
                let obj = self.eval_expr(object, env)?;
                let idx = self.eval_expr(index, env)?;
                Ok(obj.get_index(&idx))
            }

            Expr::Object(pairs) => {
                let mut map = HashMap::new();
                for (k, v) in pairs {
                    let val = self.eval_expr(v, env)?;
                    map.insert(k.clone(), val);
                }
                Ok(Value::Object(map))
            }

            Expr::Array(items) => {
                let mut arr = Vec::new();
                for item in items {
                    arr.push(self.eval_expr(item, env)?);
                }
                Ok(Value::Array(arr))
            }

            Expr::Not(inner) => {
                let v = self.eval_expr(inner, env)?;
                Ok(Value::Bool(!v.is_truthy()))
            }

            Expr::BinOp { left, op, right } => {
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                self.eval_binop(lv, op, rv)
            }

            Expr::Call { callee, args } => {
                self.eval_call(callee, args, env)
            }
        }
    }

    fn eval_binop(&self, lv: Value, op: &BinOp, rv: Value) -> Result<Value, Signal> {
        match op {
            BinOp::Eq     => Ok(Value::Bool(lv == rv)),
            BinOp::NotEq  => Ok(Value::Bool(lv != rv)),
            BinOp::Lt     => Ok(Value::Bool(lv < rv)),
            BinOp::LtEq   => Ok(Value::Bool(lv <= rv)),
            BinOp::Gt     => Ok(Value::Bool(lv > rv)),
            BinOp::GtEq   => Ok(Value::Bool(lv >= rv)),
            BinOp::And    => Ok(Value::Bool(lv.is_truthy() && rv.is_truthy())),
            BinOp::Or     => Ok(Value::Bool(lv.is_truthy() || rv.is_truthy())),
            BinOp::Sub    => match (&lv, &rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
                _ => Err(Signal::Error(format!("Cannot subtract {} from {}", rv.type_name(), lv.type_name()))),
            },
            BinOp::Mul    => match (&lv, &rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                _ => Err(Signal::Error(format!("Cannot multiply {} by {}", lv.type_name(), rv.type_name()))),
            },
            BinOp::Div    => match (&lv, &rv) {
                (Value::Number(a), Value::Number(b)) => {
                    if *b == 0.0 { Err(Signal::Error("Division by zero".into())) }
                    else { Ok(Value::Number(a / b)) }
                }
                _ => Err(Signal::Error(format!("Cannot divide {} by {}", lv.type_name(), rv.type_name()))),
            },
            BinOp::Mod    => match (&lv, &rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a % b)),
                _ => Err(Signal::Error(format!("Cannot mod {} by {}", lv.type_name(), rv.type_name()))),
            },
            BinOp::Add | BinOp::Concat => self.add_values_ref(&lv, &rv),
        }
    }

    fn add_values(&self, lv: Value, rv: Value) -> Result<Value, Signal> {
        self.add_values_ref(&lv, &rv)
    }

    fn add_values_ref(&self, lv: &Value, rv: &Value) -> Result<Value, Signal> {
        match (lv, rv) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
            (Value::Str(a), b) => Ok(Value::Str(format!("{}{}", a, b))),
            (a, Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
            (Value::Array(a), Value::Array(b)) => {
                let mut arr = a.clone();
                arr.extend(b.clone());
                Ok(Value::Array(arr))
            }
            _ => Err(Signal::Error(format!("Cannot add {} and {}", lv.type_name(), rv.type_name()))),
        }
    }

    // ── Built-in function calls ───────────────────────────────────────────────

    fn eval_call(&mut self, callee: &Expr, arg_exprs: &[Expr], env: &mut Env) -> Result<Value, Signal> {
        // Evaluate args
        let mut args = Vec::new();
        for a in arg_exprs {
            args.push(self.eval_expr(a, env)?);
        }

        // Check for method calls: obj.method(...)
        if let Expr::FieldAccess { object, field } = callee {
            let obj = self.eval_expr(object, env)?;
            return self.eval_method(obj, field, args, env);
        }

        // Named function / builtin
        if let Expr::Ident(name) = callee {
            return self.eval_builtin(name, args, env);
        }

        Err(Signal::Error(format!("Cannot call {:?}", callee)))
    }

    fn eval_builtin(&mut self, name: &str, args: Vec<Value>, env: &mut Env) -> Result<Value, Signal> {
        match name {
            "log" | "output" | "print" | "say" => {
                let msg = args.first().cloned().unwrap_or(Value::Null);
                println!("{}", msg);
                Ok(Value::Null)
            }
            "get_timestamp" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                Ok(Value::Number(ts as f64))
            }
            "count" => {
                let v = args.first().cloned().unwrap_or(Value::Null);
                match v {
                    Value::Array(a) => Ok(Value::Number(a.len() as f64)),
                    Value::Object(o) => Ok(Value::Number(o.len() as f64)),
                    Value::Str(s) => Ok(Value::Number(s.len() as f64)),
                    Value::Null => Ok(Value::Number(0.0)),
                    _ => Ok(Value::Number(1.0)),
                }
            }
            "to_string" => {
                let v = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Str(v.to_string()))
            }
            "to_number" => {
                let v = args.first().cloned().unwrap_or(Value::Null);
                match v {
                    Value::Number(n) => Ok(Value::Number(n)),
                    Value::Str(s) => s.parse::<f64>()
                        .map(Value::Number)
                        .map_err(|_| Signal::Error(format!("Cannot convert '{}' to number", s))),
                    _ => Err(Signal::Error(format!("Cannot convert {} to number", v.type_name()))),
                }
            }
            "spawn_agent" | "spawn_helper" => {
                // Phase 2 — for now just log
                let name = args.first().cloned().unwrap_or(Value::Str("unknown".into()));
                println!("[gx] spawning agent: {}", name);
                Ok(Value::Str(format!("agent:{}", name)))
            }
            "wait_for_agent_ready" | "wait" => Ok(Value::Null),
            "start_application" | "stop_application" | "restart_application" => Ok(Value::Null),
            "restart_all_failed_agents" => Ok(Value::Null),
            "initialize_memory_manager" | "initialize_message_router" | "initialize_helper_manager" => Ok(Value::Null),
            "parse_gx_file" | "execute_initial_brain_cycles" | "load_application" => Ok(Value::Null),
            _ => {
                // Try to call a recipe on any loaded helper
                // For now, just return null and warn
                eprintln!("[gx] warning: unknown function '{}' — returning null", name);
                Ok(Value::Null)
            }
        }
    }

    fn eval_method(&mut self, obj: Value, method: &str, args: Vec<Value>, _env: &mut Env) -> Result<Value, Signal> {
        match (&obj, method) {
            (Value::Array(arr), "push") => {
                // Returns new array with item appended — caller must reassign
                let mut new_arr = arr.clone();
                if let Some(v) = args.into_iter().next() {
                    new_arr.push(v);
                }
                Ok(Value::Array(new_arr))
            }
            (Value::Array(arr), "length") => Ok(Value::Number(arr.len() as f64)),
            (Value::Array(arr), "join") => {
                let sep = args.first().and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let s = arr.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&sep);
                Ok(Value::Str(s))
            }
            (Value::Str(s), "length") => Ok(Value::Number(s.len() as f64)),
            (Value::Str(s), "to_upper") => Ok(Value::Str(s.to_uppercase())),
            (Value::Str(s), "to_lower") => Ok(Value::Str(s.to_lowercase())),
            (Value::Str(s), "trim") => Ok(Value::Str(s.trim().to_string())),
            (Value::Str(s), "contains") => {
                let needle = args.first().and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(s.contains(&needle)))
            }
            (Value::Object(_), "push") => {
                // push on an object used as a list — not meaningful, return as-is
                Ok(obj)
            }
            _ => {
                eprintln!("[gx] warning: unknown method '{}.{}' — returning null", obj.type_name(), method);
                Ok(Value::Null)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        let mut interp = Interpreter::new();
        interp.run_program(&program)
    }

    #[test]
    fn test_hello_world() {
        let src = r#"
helper "hello" {
  brain {
    plan { plan = { action: "greet" } }
    execute {
      if plan.action == "greet" {
        output("Hello, Brain-First World!")
      }
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }

    #[test]
    fn test_memory_read_write() {
        let src = r#"
helper "mem_test" {
  remember {
    count = 0
  }
  brain {
    plan { }
    execute {
      memory.count = memory.count + 1
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }

    #[test]
    fn test_if_else() {
        let src = r#"
helper "cond" {
  brain {
    plan { plan = { action: "test" } }
    execute {
      if plan.action == "test" {
        log("condition is true")
      } else {
        log("condition is false")
      }
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }

    #[test]
    fn test_for_each() {
        let src = r#"
helper "loop_test" {
  brain {
    plan { }
    execute {
      items = ["a", "b", "c"]
      for each item in items {
        log(item)
      }
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }

    #[test]
    fn test_arithmetic() {
        let src = r#"
helper "math" {
  brain {
    plan { }
    execute {
      result = 5 + 3
      log(result)
      result2 = 10 * 4
      log(result2)
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }

    #[test]
    fn test_string_concat() {
        let src = r#"
helper "concat_test" {
  brain {
    plan { }
    execute {
      greeting = "Hello, " + "World!"
      log(greeting)
    }
    remember { }
    communicate { }
  }
}"#;
        run(src).unwrap();
    }
}
