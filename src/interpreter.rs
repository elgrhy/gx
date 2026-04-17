//! GX Interpreter — executes the AST produced by the parser.

use crate::ai;
use crate::ast::*;
use crate::bridge::Bridge;
use crate::value::Value;
use std::collections::HashMap;

// ── Control flow signals ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Signal {
    Return(Value),
    ReRun,
    EscalateToHuman,
    Error(String),
}

type IResult = Result<Value, Signal>;

// ── Environment ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Value {
        self.vars.get(name).cloned().unwrap_or(Value::Null)
    }

    pub fn set(&mut self, name: &str, val: Value) {
        self.vars.insert(name.to_string(), val);
    }

    pub fn get_memory(&self) -> HashMap<String, Value> {
        match self.get("memory") {
            Value::Object(m) => m,
            _ => HashMap::new(),
        }
    }

    pub fn set_memory(&mut self, mem: HashMap<String, Value>) {
        self.vars.insert("memory".to_string(), Value::Object(mem));
    }
}

// ── Interpreter ───────────────────────────────────────────────────────────────

pub struct Interpreter {
    helpers: HashMap<String, HelperDef>,
    imports: Vec<ImportDecl>,
    pub events: Vec<(String, Vec<(String, Value)>)>,
    #[allow(dead_code)]
    js_bridge: Option<Bridge>,
    py_bridge: Option<Bridge>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            helpers: HashMap::new(),
            imports: Vec::new(),
            events: Vec::new(),
            js_bridge: None,
            py_bridge: None,
        }
    }

    pub fn run_program(&mut self, program: &Program) -> Result<(), String> {
        self.imports = program.imports.clone();

        for h in &program.helpers {
            self.helpers.insert(h.name.clone(), h.clone());
        }

        for h in &program.helpers.clone() {
            self.run_helper(h).map_err(|e| match e {
                Signal::Error(m) => m,
                Signal::Return(_) => "Unexpected return at top level".into(),
                Signal::ReRun => "Unexpected re-run at top level".into(),
                Signal::EscalateToHuman => "Escalated to human".into(),
            })?;
        }

        if let Some(brain) = &program.top_level_brain.clone() {
            let mut env = Env::new();
            self.run_brain(brain, &mut env).map_err(|e| match e {
                Signal::Error(m) => m,
                _ => "Signal at top level".into(),
            })?;
        }

        Ok(())
    }

    fn run_helper(&mut self, helper: &HelperDef) -> Result<(), Signal> {
        // Build initial memory
        let mut memory: HashMap<String, Value> = HashMap::new();
        // Auto-initialize ai_trace for all helpers
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));

        let mut env = Env::new();
        for entry in &helper.memory {
            let val = self.eval_expr(&entry.value, &mut env)?;
            memory.insert(entry.key.clone(), val);
        }

        // Run `when started` blocks FIRST (before brain cycle)
        for wb in &helper.when_blocks.clone() {
            if matches!(wb.trigger, WhenTrigger::Started) {
                env.set_memory(memory.clone());
                self.run_stmts(&wb.body, &mut env)?;
                memory = env.get_memory();
            }
        }

        // Run brain cycle (with optional re-run loop)
        if let Some(brain) = &helper.brain.clone() {
            let mut cycles = 0;
            const MAX_CYCLES: usize = 100;
            loop {
                env.set_memory(memory.clone());
                env.set("plan", Value::Null);
                env.set("result", Value::Null);

                match self.run_brain(brain, &mut env) {
                    Ok(_) => {}
                    Err(Signal::ReRun) if cycles < MAX_CYCLES => {
                        cycles += 1;
                        memory = env.get_memory();
                        continue;
                    }
                    Err(Signal::ReRun) => {
                        return Err(Signal::Error(format!(
                            "Helper '{}' exceeded {} re-run cycles",
                            helper.name, MAX_CYCLES
                        )));
                    }
                    Err(e) => return Err(e),
                }
                memory = env.get_memory();
                break;
            }
        }

        // Run remaining when blocks (expr/changes triggers) after brain
        for wb in &helper.when_blocks.clone() {
            env.set_memory(memory.clone());
            match &wb.trigger {
                WhenTrigger::Started => { /* already ran above */ }
                WhenTrigger::Expr(cond) => {
                    let v = self.eval_expr(cond, &mut env)?;
                    if v.is_truthy() {
                        self.run_stmts(&wb.body, &mut env)?;
                        memory = env.get_memory();
                    }
                }
                WhenTrigger::Changes(expr) => {
                    // Store previous value in memory under __prev_<key>
                    let key = format!("__prev_{:?}", expr)
                        .replace('"', "")
                        .replace(' ', "_");
                    let current = self.eval_expr(expr, &mut env)?;
                    let prev = memory.get(&key).cloned().unwrap_or(Value::Null);
                    if current != prev {
                        memory.insert(key.clone(), current.clone());
                        env.set_memory(memory.clone());
                        self.run_stmts(&wb.body, &mut env)?;
                        memory = env.get_memory();
                    }
                }
            }
        }

        Ok(())
    }

    fn run_brain(&mut self, brain: &BrainBlock, env: &mut Env) -> IResult {
        self.run_stmts(&brain.plan, env)?;
        self.run_stmts(&brain.execute, env)?;
        self.run_stmts(&brain.remember, env)?;
        self.run_stmts(&brain.communicate, env)?;
        Ok(Value::Null)
    }

    // ── Statement execution ───────────────────────────────────────────────────

    fn run_stmts(&mut self, stmts: &[Stmt], env: &mut Env) -> IResult {
        let mut last = Value::Null;
        for stmt in stmts {
            last = self.run_stmt(stmt, env)?;
        }
        Ok(last)
    }

    fn run_stmt(&mut self, stmt: &Stmt, env: &mut Env) -> IResult {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                let val = self.eval_expr(value, env)?;
                self.assign(target, val, env)?;
                Ok(Value::Null)
            }

            Stmt::PlusAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.add_values(&cur, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for (cond, body) in branches {
                    if self.eval_expr(cond, env)?.is_truthy() {
                        return self.run_stmts(body, env);
                    }
                }
                if let Some(body) = else_body {
                    return self.run_stmts(body, env);
                }
                Ok(Value::Null)
            }

            Stmt::ForEach {
                var, iter, body, ..
            } => {
                let col = self.eval_expr(iter, env)?;
                let items = col.iter().map_err(Signal::Error)?;
                let mut last = Value::Null;
                for item in items {
                    env.set(var, item);
                    last = self.run_stmts(body, env)?;
                }
                Ok(last)
            }

            Stmt::TryCatch {
                try_body,
                catch_var,
                catch_body,
                ..
            } => match self.run_stmts(try_body, env) {
                Ok(v) => Ok(v),
                Err(Signal::Error(msg)) => {
                    env.set(catch_var, Value::Str(msg));
                    self.run_stmts(catch_body, env)
                }
                Err(other) => Err(other),
            },

            Stmt::Emit { event, payload, .. } => {
                let mut resolved = Vec::new();
                for (k, expr) in payload {
                    resolved.push((k.clone(), self.eval_expr(expr, env)?));
                }
                self.events.push((event.clone(), resolved));
                Ok(Value::Null)
            }

            Stmt::Broadcast { event, .. } => {
                self.events.push((event.clone(), Vec::new()));
                Ok(Value::Null)
            }

            Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                let v = self.eval_expr(value, env)?;
                println!("{}", v);
                Ok(Value::Null)
            }

            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Null,
                };
                Err(Signal::Return(v))
            }

            Stmt::Wait { ms, .. } => {
                if let Some(n) = self.eval_expr(ms, env)?.as_number() {
                    std::thread::sleep(std::time::Duration::from_millis(n as u64));
                }
                Ok(Value::Null)
            }

            Stmt::ReRun { .. } => Err(Signal::ReRun),

            Stmt::EscalateToHuman { .. } => {
                eprintln!("[gx] Escalating to human — agent cannot handle this request");
                self.events.push(("escalate_to_human".into(), Vec::new()));
                Err(Signal::EscalateToHuman)
            }

            Stmt::Expr { expr, .. } => self.eval_expr(expr, env),
        }
    }

    // ── Assignment ────────────────────────────────────────────────────────────

    fn assign(&mut self, target: &Expr, val: Value, env: &mut Env) -> Result<(), Signal> {
        match target {
            Expr::Ident(name) => {
                env.set(name, val);
                Ok(())
            }

            Expr::FieldAccess { object, field } => match object.as_ref() {
                Expr::Ident(obj_name) => {
                    let mut obj = env.get(obj_name);
                    if matches!(obj, Value::Null) {
                        obj = Value::Object(HashMap::new());
                    }
                    obj.set_field(field, val).map_err(Signal::Error)?;
                    env.set(obj_name, obj);
                    Ok(())
                }
                Expr::FieldAccess {
                    object: inner_obj,
                    field: inner_field,
                } => {
                    let root = self.expr_root_name(inner_obj);
                    let mut outer = env.get(&root);
                    if matches!(outer, Value::Null) {
                        outer = Value::Object(HashMap::new());
                    }
                    let mut inner = outer.get_field(inner_field);
                    if matches!(inner, Value::Null) {
                        inner = Value::Object(HashMap::new());
                    }
                    inner.set_field(field, val).map_err(Signal::Error)?;
                    outer.set_field(inner_field, inner).map_err(Signal::Error)?;
                    env.set(&root, outer);
                    Ok(())
                }
                _ => Err(Signal::Error("Cannot assign to complex expression".into())),
            },

            Expr::Index { object, index } => {
                if let Expr::Ident(name) = object.as_ref() {
                    let idx = self.eval_expr(index, env)?;
                    let mut obj = env.get(name);
                    match (&mut obj, &idx) {
                        (Value::Array(arr), Value::Number(n)) => {
                            let i = *n as usize;
                            if i < arr.len() {
                                arr[i] = val;
                            } else {
                                return Err(Signal::Error(format!(
                                    "Array index {} out of bounds",
                                    i
                                )));
                            }
                        }
                        (Value::Object(map), Value::Str(k)) => {
                            map.insert(k.clone(), val);
                        }
                        _ => return Err(Signal::Error("Cannot index assign to this type".into())),
                    }
                    env.set(name, obj);
                    Ok(())
                } else {
                    Err(Signal::Error(
                        "Cannot assign to complex index expression".into(),
                    ))
                }
            }

            _ => Err(Signal::Error(format!("Cannot assign to {:?}", target))),
        }
    }

    fn eval_lvalue(&mut self, expr: &Expr, env: &mut Env) -> Value {
        self.eval_expr(expr, env).unwrap_or(Value::Null)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn expr_root_name(&self, expr: &Expr) -> String {
        match expr {
            Expr::Ident(s) => s.clone(),
            Expr::FieldAccess { object, .. } => self.expr_root_name(object),
            _ => "unknown".into(),
        }
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    pub fn eval_expr(&mut self, expr: &Expr, env: &mut Env) -> IResult {
        match expr {
            Expr::Null => Ok(Value::Null),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Num(n) => Ok(Value::Number(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),

            Expr::Interpolated(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        InterpolatedPart::Literal(l) => s.push_str(l),
                        InterpolatedPart::Expr(e) => {
                            s.push_str(&self.eval_expr(e, env)?.to_string())
                        }
                    }
                }
                Ok(Value::Str(s))
            }

            Expr::Ident(name) => Ok(env.get(name)),

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
                    map.insert(k.clone(), self.eval_expr(v, env)?);
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

            Expr::Not(inner) => Ok(Value::Bool(!self.eval_expr(inner, env)?.is_truthy())),

            Expr::BinOp { left, op, right } => {
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                self.eval_binop(&lv, op, &rv)
            }

            Expr::Call { callee, args } => self.eval_call(callee, args, env),

            // Phase 3: AI primitives
            Expr::AskAI {
                provider,
                model,
                params,
            } => {
                let mut resolved: HashMap<String, Value> = HashMap::new();
                for (k, v) in params {
                    resolved.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let result = ai::ask_ai(provider, model.as_deref(), &resolved);
                // Auto-log to memory.ai_trace
                self.append_ai_trace(env, &result);
                Ok(result)
            }

            Expr::Embed { text } => {
                let t = self.eval_expr(text, env)?;
                Ok(ai::embed_text(&t.to_string()))
            }

            Expr::InferClassifier { input, classes } => {
                let input_val = self.eval_expr(input, env)?.to_string();
                let classes_val = self.eval_expr(classes, env)?;
                let class_list: Vec<String> = match classes_val {
                    Value::Array(arr) => arr.iter().map(|v| v.to_string()).collect(),
                    other => vec![other.to_string()],
                };
                let provider = "openai"; // default provider
                Ok(ai::infer_classifier(&input_val, &class_list, provider))
            }

            // Phase 4: Package bridge calls
            Expr::BridgeCall {
                namespace,
                module,
                method,
                args,
            } => {
                let resolved: Vec<Value> = args
                    .iter()
                    .map(|a| self.eval_expr(a, env))
                    .collect::<Result<Vec<_>, _>>()?;
                let (ns, mo, me) = (namespace.clone(), module.clone(), method.clone());
                self.bridge_call(&ns, &mo, &me, &resolved)
            }
        }
    }

    fn append_ai_trace(&mut self, env: &mut Env, result: &Value) {
        let mut memory = env.get_memory();
        let trace = memory
            .entry("ai_trace".into())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(arr) = trace {
            arr.push(result.clone());
        }
        env.set_memory(memory);
    }

    fn eval_binop(&self, lv: &Value, op: &BinOp, rv: &Value) -> IResult {
        match op {
            BinOp::Eq => Ok(Value::Bool(lv == rv)),
            BinOp::NotEq => Ok(Value::Bool(lv != rv)),
            BinOp::Lt => Ok(Value::Bool(lv < rv)),
            BinOp::LtEq => Ok(Value::Bool(lv <= rv)),
            BinOp::Gt => Ok(Value::Bool(lv > rv)),
            BinOp::GtEq => Ok(Value::Bool(lv >= rv)),
            BinOp::And => Ok(Value::Bool(lv.is_truthy() && rv.is_truthy())),
            BinOp::Or => Ok(Value::Bool(lv.is_truthy() || rv.is_truthy())),
            BinOp::Sub => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
                _ => Err(Signal::Error(format!(
                    "Cannot subtract {} from {}",
                    rv.type_name(),
                    lv.type_name()
                ))),
            },
            BinOp::Mul => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                _ => Err(Signal::Error(format!(
                    "Cannot multiply {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Div => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => {
                    if *b == 0.0 {
                        Err(Signal::Error("Division by zero".into()))
                    } else {
                        Ok(Value::Number(a / b))
                    }
                }
                _ => Err(Signal::Error(format!(
                    "Cannot divide {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Mod => match (lv, rv) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a % b)),
                _ => Err(Signal::Error(format!(
                    "Cannot mod {} by {}",
                    lv.type_name(),
                    rv.type_name()
                ))),
            },
            BinOp::Add | BinOp::Concat => self.add_values(lv, rv),
        }
    }

    fn add_values(&self, lv: &Value, rv: &Value) -> IResult {
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
            _ => Err(Signal::Error(format!(
                "Cannot add {} and {}",
                lv.type_name(),
                rv.type_name()
            ))),
        }
    }

    // ── Function calls ────────────────────────────────────────────────────────

    fn eval_call(&mut self, callee: &Expr, arg_exprs: &[Expr], env: &mut Env) -> IResult {
        let mut args = Vec::new();
        for a in arg_exprs {
            args.push(self.eval_expr(a, env)?);
        }

        if let Expr::FieldAccess { object, field } = callee {
            let obj = self.eval_expr(object, env)?;
            return self.eval_method(obj, field, args, env);
        }

        if let Expr::Ident(name) = callee {
            return self.eval_builtin(name, args, env);
        }

        Err(Signal::Error(format!("Cannot call {:?}", callee)))
    }

    fn eval_builtin(&mut self, name: &str, args: Vec<Value>, _env: &mut Env) -> IResult {
        match name {
            "log" | "output" | "print" | "say" => {
                println!("{}", args.first().cloned().unwrap_or(Value::Null));
                Ok(Value::Null)
            }
            "get_timestamp" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                Ok(Value::Number(ts as f64))
            }
            "count" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(a) => Ok(Value::Number(a.len() as f64)),
                Value::Object(o) => Ok(Value::Number(o.len() as f64)),
                Value::Str(s) => Ok(Value::Number(s.len() as f64)),
                Value::Null => Ok(Value::Number(0.0)),
                _ => Ok(Value::Number(1.0)),
            },
            "to_string" => Ok(Value::Str(
                args.first().cloned().unwrap_or(Value::Null).to_string(),
            )),
            "to_number" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Number(n) => Ok(Value::Number(n)),
                Value::Str(s) => s
                    .parse::<f64>()
                    .map(Value::Number)
                    .map_err(|_| Signal::Error(format!("Cannot convert '{}' to number", s))),
                _ => Ok(Value::Number(0.0)),
            },
            "type_of" => Ok(Value::Str(
                args.first()
                    .cloned()
                    .unwrap_or(Value::Null)
                    .type_name()
                    .into(),
            )),
            "is_null" => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Null) | None
            ))),
            "keys" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => Ok(Value::Array(
                    m.keys().map(|k| Value::Str(k.clone())).collect(),
                )),
                _ => Ok(Value::Array(Vec::new())),
            },
            "values" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => Ok(Value::Array(m.values().cloned().collect())),
                _ => Ok(Value::Array(Vec::new())),
            },
            "range" => {
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                let end = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                Ok(Value::Array(
                    (start..end).map(|n| Value::Number(n as f64)).collect(),
                ))
            }
            "floor" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .floor(),
            )),
            "ceil" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .ceil(),
            )),
            "round" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .round(),
            )),
            "abs" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .abs(),
            )),
            "max" => {
                let a = args
                    .first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::NEG_INFINITY);
                let b = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::NEG_INFINITY);
                Ok(Value::Number(a.max(b)))
            }
            "min" => {
                let a = args
                    .first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::INFINITY);
                let b = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::INFINITY);
                Ok(Value::Number(a.min(b)))
            }
            // Stubs for runtime management (Phase 7)
            "spawn_agent" | "spawn_helper" => {
                let n = args
                    .first()
                    .cloned()
                    .unwrap_or(Value::Str("unknown".into()));
                eprintln!("[gx] spawning agent: {}", n);
                Ok(Value::Str(format!("agent:{}", n)))
            }
            "wait_for_agent_ready"
            | "start_application"
            | "stop_application"
            | "restart_application"
            | "restart_all_failed_agents"
            | "initialize_memory_manager"
            | "initialize_message_router"
            | "initialize_helper_manager"
            | "parse_gx_file"
            | "execute_initial_brain_cycles"
            | "load_application"
            | "generate_job_id"
            | "start_training_process"
            | "monitor_training_jobs"
            | "deploy_ready_models"
            | "update_model_performance"
            | "cleanup_old_models" => Ok(Value::Null),
            _ => {
                eprintln!("[gx] warning: unknown function '{}' — returning null", name);
                Ok(Value::Null)
            }
        }
    }

    fn eval_method(
        &mut self,
        obj: Value,
        method: &str,
        args: Vec<Value>,
        _env: &mut Env,
    ) -> IResult {
        match (&obj, method) {
            (Value::Array(arr), "push") => {
                let mut a = arr.clone();
                if let Some(v) = args.into_iter().next() {
                    a.push(v);
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "pop") => {
                let mut a = arr.clone();
                Ok(a.pop().unwrap_or(Value::Null))
            }
            (Value::Array(arr), "length") | (Value::Array(arr), "len") => {
                Ok(Value::Number(arr.len() as f64))
            }
            (Value::Array(arr), "first") => Ok(arr.first().cloned().unwrap_or(Value::Null)),
            (Value::Array(arr), "last") => Ok(arr.last().cloned().unwrap_or(Value::Null)),
            (Value::Array(arr), "join") => {
                let sep = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    arr.iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(&sep),
                ))
            }
            (Value::Array(arr), "contains") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Bool(arr.contains(&needle)))
            }
            (Value::Array(arr), "reverse") => {
                let mut a = arr.clone();
                a.reverse();
                Ok(Value::Array(a))
            }
            (Value::Str(s), "length") | (Value::Str(s), "len") => Ok(Value::Number(s.len() as f64)),
            (Value::Str(s), "to_upper") => Ok(Value::Str(s.to_uppercase())),
            (Value::Str(s), "to_lower") => Ok(Value::Str(s.to_lowercase())),
            (Value::Str(s), "trim") => Ok(Value::Str(s.trim().to_string())),
            (Value::Str(s), "split") => {
                let sep = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or(" ".into());
                Ok(Value::Array(
                    s.split(&*sep).map(|p| Value::Str(p.to_string())).collect(),
                ))
            }
            (Value::Str(s), "contains") => {
                let needle = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.contains(&*needle)))
            }
            (Value::Str(s), "starts_with") => {
                let p = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.starts_with(&*p)))
            }
            (Value::Str(s), "ends_with") => {
                let p = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.ends_with(&*p)))
            }
            (Value::Str(s), "replace") => {
                let from = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let to = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replace(&*from, &to)))
            }
            (Value::Object(m), "has") | (Value::Object(m), "has_key") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(m.contains_key(&*key)))
            }
            (Value::Object(m), "keys") => Ok(Value::Array(
                m.keys().map(|k| Value::Str(k.clone())).collect(),
            )),
            (Value::Object(m), "values") => Ok(Value::Array(m.values().cloned().collect())),
            _ => {
                eprintln!(
                    "[gx] warning: unknown method '{}.{}' — returning null",
                    obj.type_name(),
                    method
                );
                Ok(Value::Null)
            }
        }
    }
}

impl Interpreter {
    pub fn call_js(&mut self, module: &str, method: &str, args: &[Value]) -> Result<Value, Signal> {
        use crate::bridge::{json_to_value, value_to_json};
        use std::process::Command;

        let json_args = serde_json::to_string(&args.iter().map(value_to_json).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".into());

        // Generate a one-shot Node.js script
        let script = format!(
            r#"try {{
  const mod = require('{}');
  const parts = '{}' .split('.');
  let fn_ref = mod;
  for (const p of parts) {{ fn_ref = fn_ref[p]; }}
  const args = {};
  const result = typeof fn_ref === 'function' ? fn_ref(...args) : fn_ref;
  if (result && typeof result.then === 'function') {{
    result.then(r => console.log(JSON.stringify({{ok:true,result:r && r.data !== undefined ? r.data : r}})))
          .catch(e => console.log(JSON.stringify({{ok:false,error:String(e)}})));
  }} else {{
    console.log(JSON.stringify({{ok:true,result:result}}));
  }}
}} catch(e) {{ console.log(JSON.stringify({{ok:false,error:String(e)}})); }}"#,
            module, method, json_args
        );

        let output = Command::new("node")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| Signal::Error(format!("Failed to run node: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().last().unwrap_or("").trim();

        if last_line.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Signal::Error(format!("JS error: {}", stderr.trim())));
        }

        match serde_json::from_str::<serde_json::Value>(last_line) {
            Ok(json) => {
                if json["ok"].as_bool() == Some(true) {
                    Ok(json_to_value(&json["result"]))
                } else {
                    Err(Signal::Error(format!(
                        "JS error: {}",
                        json["error"].as_str().unwrap_or("unknown")
                    )))
                }
            }
            Err(_) => Ok(Value::Str(last_line.to_string())),
        }
    }
}

// Fix get_js_bridge to use the simple bridge
impl Interpreter {
    pub fn bridge_call(
        &mut self,
        namespace: &str,
        module: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Signal> {
        match namespace {
            "js" => self.call_js(module, method, args),
            "py" => {
                let bridge = match self.py_bridge.as_mut() {
                    Some(b) => b,
                    None => match Bridge::new_python() {
                        Ok(b) => {
                            self.py_bridge = Some(b);
                            self.py_bridge.as_mut().unwrap()
                        }
                        Err(e) => return Err(Signal::Error(e)),
                    },
                };
                bridge.call(module, method, args).map_err(Signal::Error)
            }
            "rust" => Err(Signal::Error(format!(
                "Native Rust interop for '{}' requires recompiling GX with the crate linked.",
                module
            ))),
            other => Err(Signal::Error(format!(
                "Unknown namespace '{}'. Use: js, py, rust",
                other
            ))),
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
        Interpreter::new().run_program(&program)
    }

    #[test]
    fn test_hello_world() {
        run(r#"
helper "hello" {
  brain {
    plan { plan = { action: "greet" } }
    execute { if plan.action == "greet" { output("Hello, Brain-First World!") } }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_memory_read_write() {
        run(r#"
helper "mem" {
  remember { count = 0 }
  brain {
    plan { }
    execute { memory.count = memory.count + 1 }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_if_else() {
        run(r#"
helper "cond" {
  brain {
    plan { plan = { action: "test" } }
    execute {
      if plan.action == "test" { log("yes") }
      else { log("no") }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_for_each() {
        run(r#"
helper "loop" {
  brain {
    plan { }
    execute {
      items = ["a", "b", "c"]
      for each item in items { log(item) }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_arithmetic() {
        run(r#"
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
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_concat() {
        run(r#"
helper "str" {
  brain {
    plan { }
    execute { greeting = "Hello, " + "World!"; log(greeting) }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_agent_when_started() {
        run(r#"
agent "bot" {
  remember greeting = "hello from when block"
  when started {
    say memory.greeting
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_try_catch() {
        run(r#"
helper "safe" {
  brain {
    plan { }
    execute {
      try {
        result = 10 / 0
      } catch err {
        log("Caught: " + err)
      }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_interpolation() {
        run(r#"
helper "interp" {
  remember { name = "GX" }
  brain {
    plan { }
    execute { output("Hello from {memory.name}!") }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_nested_memory() {
        run(r#"
helper "nested" {
  remember { config = { debug: false, version: "1.0" } }
  brain {
    plan { }
    execute { memory.config.debug = true; log(memory.config.debug) }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_array_methods() {
        run(r#"
helper "arrays" {
  brain {
    plan { }
    execute {
      items = ["a", "b"]
      items = items.push("c")
      log(items.length)
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }
}
