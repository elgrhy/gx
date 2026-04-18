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
    Break,
    Continue,
    ReRun,
    EscalateToHuman,
    AssertFail(String),
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
    functions: HashMap<String, FunctionDef>,
    imports: Vec<ImportDecl>,
    pub events: Vec<(String, Vec<(String, Value)>)>,
    // Shared event bus for inter-agent communication
    event_bus: HashMap<String, Vec<Value>>,
    #[allow(dead_code)]
    js_bridge: Option<Bridge>,
    py_bridge: Option<Bridge>,
    // Track import base path for relative imports
    pub base_path: Option<String>,
    // Assertion tracking for gx test
    pub assert_count: usize,
    pub assert_failures: Vec<String>,
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
            functions: HashMap::new(),
            imports: Vec::new(),
            events: Vec::new(),
            event_bus: HashMap::new(),
            js_bridge: None,
            py_bridge: None,
            base_path: None,
            assert_count: 0,
            assert_failures: Vec::new(),
        }
    }

    pub fn run_program(&mut self, program: &Program) -> Result<(), String> {
        self.imports = program.imports.clone();

        for f in &program.functions {
            self.functions.insert(f.name.clone(), f.clone());
        }

        // Process file imports — resolve paths relative to base_path
        for fi in &program.file_imports {
            let resolved_path = if std::path::Path::new(&fi.path).exists() {
                fi.path.clone()
            } else if let Some(ref base) = self.base_path.clone() {
                let base_dir = std::path::Path::new(base)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let candidate = base_dir.join(&fi.path).to_string_lossy().into_owned();
                if std::path::Path::new(&candidate).exists() {
                    candidate
                } else {
                    fi.path.clone()
                }
            } else {
                fi.path.clone()
            };

            let src = std::fs::read_to_string(&resolved_path).map_err(|e| {
                format!("Line {}: cannot import '{}': {}", fi.line, resolved_path, e)
            })?;

            let sub = parse_gx_source(&src, &resolved_path)?;

            for f in &sub.functions {
                self.functions.insert(f.name.clone(), f.clone());
            }
            for h in &sub.helpers {
                self.helpers.insert(h.name.clone(), h.clone());
            }
            self.imports.extend(sub.imports.clone());
        }

        for h in &program.helpers {
            self.helpers.insert(h.name.clone(), h.clone());
        }

        for h in &program.helpers.clone() {
            self.run_helper(h).map_err(|e| match e {
                Signal::Error(m) => m,
                Signal::AssertFail(m) => format!("Assertion failed: {}", m),
                Signal::Return(_) => "Unexpected return at top level".into(),
                Signal::ReRun => "Unexpected re-run at top level".into(),
                Signal::Break => "Unexpected break at top level".into(),
                Signal::Continue => "Unexpected continue at top level".into(),
                Signal::EscalateToHuman => "Escalated to human".into(),
            })?;
        }

        if let Some(brain) = &program.top_level_brain.clone() {
            let mut env = Env::new();
            self.run_brain(brain, &mut env).map_err(|e| match e {
                Signal::Error(m) => m,
                Signal::AssertFail(m) => format!("Assertion failed: {}", m),
                _ => "Signal at top level".into(),
            })?;
        }

        Ok(())
    }

    fn run_helper(&mut self, helper: &HelperDef) -> Result<(), Signal> {
        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));

        let mut env = Env::new();
        for entry in &helper.memory {
            let val = self.eval_expr(&entry.value, &mut env)?;
            memory.insert(entry.key.clone(), val);
        }

        // Run recipes as named functions (available to the brain cycle)
        for recipe in &helper.recipes.clone() {
            let func = FunctionDef {
                name: recipe.name.clone(),
                params: recipe.needs.clone(),
                body: {
                    let brain = &recipe.brain;
                    let mut body = brain.plan.clone();
                    body.extend(brain.execute.clone());
                    body.extend(brain.remember.clone());
                    body.extend(brain.communicate.clone());
                    body
                },
                line: recipe.line,
            };
            self.functions.insert(recipe.name.clone(), func);
        }

        // Run objectives: register as when-expr blocks
        let mut extra_when: Vec<WhenBlock> = helper
            .objectives
            .iter()
            .map(|obj| WhenBlock {
                trigger: WhenTrigger::Expr(obj.when_expr.clone()),
                body: vec![Stmt::Expr {
                    expr: obj.then_action.clone(),
                    line: obj.line,
                }],
                line: obj.line,
            })
            .collect();

        // Run `when started` blocks
        for wb in &helper.when_blocks.clone() {
            if matches!(wb.trigger, WhenTrigger::Started) {
                env.set_memory(memory.clone());
                self.run_stmts(&wb.body, &mut env)?;
                memory = env.get_memory();
            }
        }

        // Brain cycle
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

        // Receive blocks: process pending events from the event bus
        for ch in &helper.receive_block.clone() {
            if let Some(events) = self.event_bus.get(&ch.name).cloned() {
                for event_val in events {
                    env.set_memory(memory.clone());
                    env.set("event", event_val);
                    if let Some(ref handler_name) = ch.on_receive {
                        if let Some(func) = self.functions.get(handler_name).cloned() {
                            self.call_behavior(&func, &mut env)?;
                        }
                    }
                    if let Some(ref bind_expr) = ch.bind.clone() {
                        self.eval_expr(bind_expr, &mut env)?;
                    }
                    memory = env.get_memory();
                }
                self.event_bus.remove(&ch.name);
            }
        }

        // Remaining when blocks (expr/changes triggers) + objective triggers
        extra_when.extend(
            helper
                .when_blocks
                .clone()
                .into_iter()
                .filter(|wb| !matches!(wb.trigger, WhenTrigger::Started)),
        );
        for wb in &extra_when {
            env.set_memory(memory.clone());
            match &wb.trigger {
                WhenTrigger::Started => {}
                WhenTrigger::Expr(cond) => {
                    let v = self.eval_expr(cond, &mut env)?;
                    if v.is_truthy() {
                        self.run_stmts(&wb.body, &mut env)?;
                        memory = env.get_memory();
                    }
                }
                WhenTrigger::Changes(expr) => {
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

            Stmt::MinusAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Sub, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::MulAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Mul, &rhs)?;
                self.assign(target, res, env)?;
                Ok(Value::Null)
            }

            Stmt::DivAssign { target, value, .. } => {
                let cur = self.eval_lvalue(target, env);
                let rhs = self.eval_expr(value, env)?;
                let res = self.eval_binop(&cur, &BinOp::Div, &rhs)?;
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
                'outer: for item in items {
                    env.set(var, item);
                    match self.run_stmts(body, env) {
                        Ok(v) => last = v,
                        Err(Signal::Break) => break 'outer,
                        Err(Signal::Continue) => continue 'outer,
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }

            Stmt::While {
                condition, body, ..
            } => {
                let mut last = Value::Null;
                let mut iterations = 0usize;
                const MAX_WHILE: usize = 1_000_000;
                loop {
                    if iterations >= MAX_WHILE {
                        return Err(Signal::Error(
                            "while loop exceeded 1,000,000 iterations (infinite loop?)".into(),
                        ));
                    }
                    iterations += 1;
                    let cond = self.eval_expr(condition, env)?;
                    if !cond.is_truthy() {
                        break;
                    }
                    match self.run_stmts(body, env) {
                        Ok(v) => last = v,
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(last)
            }

            Stmt::Break { .. } => Err(Signal::Break),
            Stmt::Continue { .. } => Err(Signal::Continue),

            Stmt::Assert {
                condition,
                message,
                line,
            } => {
                self.assert_count += 1;
                let passed = self.eval_expr(condition, env)?.is_truthy();
                if !passed {
                    let msg = if let Some(msg_expr) = message {
                        self.eval_expr(msg_expr, env)?.to_string()
                    } else {
                        format!("assertion at line {} failed", line)
                    };
                    self.assert_failures.push(msg.clone());
                    return Err(Signal::AssertFail(msg));
                }
                Ok(Value::Null)
            }

            Stmt::TryCatch {
                try_body,
                catch_var,
                catch_body,
                ..
            } => match self.run_stmts(try_body, env) {
                Ok(v) => Ok(v),
                Err(Signal::Error(msg)) | Err(Signal::AssertFail(msg)) => {
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
                // Add to both the events list and the event bus
                self.events.push((event.clone(), resolved.clone()));
                let bus_val = {
                    let mut map = HashMap::new();
                    for (k, v) in resolved {
                        map.insert(k, v);
                    }
                    Value::Object(map)
                };
                self.event_bus
                    .entry(event.clone())
                    .or_default()
                    .push(bus_val);
                Ok(Value::Null)
            }

            Stmt::Broadcast { event, .. } => {
                self.events.push((event.clone(), Vec::new()));
                self.event_bus
                    .entry(event.clone())
                    .or_default()
                    .push(Value::Null);
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

            Stmt::Expr { expr, .. } => {
                // Auto-mutate: arr.push(x), arr.pop(), arr.sort(), arr.reverse() as statements
                if let Expr::Call { callee, args } = expr {
                    if let Expr::FieldAccess { object, field } = callee.as_ref() {
                        let method = field.as_str();
                        if matches!(method, "push" | "pop" | "sort" | "reverse" | "append") {
                            if let Some(var_name) = self.extract_ident_name(object) {
                                let mut obj = env.get(&var_name);
                                let resolved_args: Vec<Value> = args
                                    .iter()
                                    .map(|a| self.eval_expr(a, env))
                                    .collect::<Result<Vec<_>, _>>()?;
                                let new_val =
                                    self.eval_method(obj.clone(), method, resolved_args, env)?;
                                // For pop: return value, mutate the array in env
                                if method == "pop" {
                                    if let Value::Array(ref mut arr) = obj {
                                        arr.pop();
                                        env.set(&var_name, obj);
                                    }
                                    return Ok(new_val);
                                }
                                env.set(&var_name, new_val);
                                return Ok(Value::Null);
                            }
                        }
                    }
                }

                // Auto-call zero-arg user functions referenced as bare identifiers
                if let Expr::Ident(ref name) = expr {
                    if let Some(func) = self.functions.get(name).cloned() {
                        if func.params.is_empty() {
                            return self.call_behavior(&func, env);
                        }
                    }
                }
                self.eval_expr(expr, env)
            }
        }
    }

    fn extract_ident_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            _ => None,
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
                            let i = if *n < 0.0 {
                                (arr.len() as i64 + *n as i64).max(0) as usize
                            } else {
                                *n as usize
                            };
                            if i < arr.len() {
                                arr[i] = val;
                            } else {
                                // Auto-extend array with nulls
                                while arr.len() < i {
                                    arr.push(Value::Null);
                                }
                                arr.push(val);
                            }
                        }
                        (Value::Object(map), Value::Str(k)) => {
                            map.insert(k.clone(), val);
                        }
                        (Value::Null, _) => {
                            // Auto-create: null[key] = val → create object or array
                            match &idx {
                                Value::Str(k) => {
                                    let mut map = HashMap::new();
                                    map.insert(k.clone(), val);
                                    obj = Value::Object(map);
                                }
                                Value::Number(n) => {
                                    let mut arr = Vec::new();
                                    let i = *n as usize;
                                    for _ in 0..i {
                                        arr.push(Value::Null);
                                    }
                                    arr.push(val);
                                    obj = Value::Array(arr);
                                }
                                _ => {
                                    return Err(Signal::Error(
                                        "Cannot index assign to null with this key type".into(),
                                    ))
                                }
                            }
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

            Expr::Ident(name) => {
                let val = env.get(name);
                if matches!(val, Value::Null) {
                    let mem = env.get_memory();
                    if let Some(v) = mem.get(name) {
                        return Ok(v.clone());
                    }
                }
                Ok(val)
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
                // Short-circuit for NullCoalesce
                if matches!(op, BinOp::NullCoalesce) {
                    let lv = self.eval_expr(left, env)?;
                    if !lv.is_null() {
                        return Ok(lv);
                    }
                    return self.eval_expr(right, env);
                }
                // Short-circuit for And/Or
                if matches!(op, BinOp::And) {
                    let lv = self.eval_expr(left, env)?;
                    if !lv.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                    let rv = self.eval_expr(right, env)?;
                    return Ok(Value::Bool(rv.is_truthy()));
                }
                if matches!(op, BinOp::Or) {
                    let lv = self.eval_expr(left, env)?;
                    if lv.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                    let rv = self.eval_expr(right, env)?;
                    return Ok(Value::Bool(rv.is_truthy()));
                }
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                self.eval_binop(&lv, op, &rv)
            }

            Expr::Call { callee, args } => self.eval_call(callee, args, env),

            Expr::AskAI {
                provider,
                model,
                params,
            } => {
                let mut resolved: HashMap<String, Value> = HashMap::new();
                for (k, v) in params {
                    resolved.insert(k.clone(), self.eval_expr(v, env)?);
                }
                // model can be set in the ask syntax (ask ollama:llama3) OR
                // as a param (model: memory.model) — params take precedence
                let param_model = resolved
                    .get("model")
                    .and_then(|v| v.as_str().map(String::from));
                let effective_model = param_model.or_else(|| model.clone());
                let result = ai::ask_ai(provider, effective_model.as_deref(), &resolved);
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
                Ok(ai::infer_classifier(&input_val, &class_list, "openai"))
            }

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
            BinOp::NullCoalesce => {
                if lv.is_null() {
                    Ok(rv.clone())
                } else {
                    Ok(lv.clone())
                }
            }
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
                (Value::Str(s), Value::Number(n)) => Ok(Value::Str(s.repeat(*n as usize))),
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
                (Value::Number(a), Value::Number(b)) => {
                    if *b == 0.0 {
                        Err(Signal::Error("Modulo by zero".into()))
                    } else {
                        Ok(Value::Number(a % b))
                    }
                }
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
            if let Some(func) = self.functions.get(name).cloned() {
                let mem = env.get_memory();
                return self.call_user_function(&func, args, Some(mem));
            }
            return self.eval_builtin(name, args, env);
        }

        Err(Signal::Error(format!("Cannot call {:?}", callee)))
    }

    fn call_behavior(&mut self, func: &FunctionDef, caller_env: &mut Env) -> IResult {
        let mut env = Env::new();
        let mem = caller_env.get_memory();
        env.set_memory(mem.clone());
        // Flatten memory into local vars so bare names work in progressive syntax:
        // writing `count += 1` inside a behavior acts like `memory.count += 1`
        for (k, v) in &mem {
            env.set(k, v.clone());
        }
        let body = func.body.clone();
        let result = match self.run_stmts(&body, &mut env) {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        // Write back explicit memory changes
        let mut new_mem = env.get_memory();
        // Also write back any local var that was originally a memory key
        for k in mem.keys() {
            let local_val = env.get(k);
            if !matches!(local_val, Value::Null) {
                new_mem.insert(k.clone(), local_val);
            }
        }
        caller_env.set_memory(new_mem);
        Ok(result)
    }

    fn call_user_function(
        &mut self,
        func: &FunctionDef,
        args: Vec<Value>,
        parent_memory: Option<HashMap<String, Value>>,
    ) -> IResult {
        let mut env = Env::new();
        if let Some(mem) = parent_memory {
            env.set_memory(mem);
        }
        for (i, param) in func.params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = func.body.clone();
        match self.run_stmts(&body, &mut env) {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    fn eval_builtin(&mut self, name: &str, args: Vec<Value>, _env: &mut Env) -> IResult {
        match name {
            // ── Output ────────────────────────────────────────────────────────
            "log" | "output" | "print" | "say" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                println!("{}", parts.join(" "));
                Ok(Value::Null)
            }
            "eprint" | "elog" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                eprintln!("{}", parts.join(" "));
                Ok(Value::Null)
            }

            // ── Time ─────────────────────────────────────────────────────────
            "get_timestamp" | "now" | "timestamp" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                Ok(Value::Number(ts as f64))
            }
            "now_ms" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                Ok(Value::Number(ts as f64))
            }
            "sleep" => {
                if let Some(n) = args.first().and_then(|v| v.as_number()) {
                    std::thread::sleep(std::time::Duration::from_millis(n as u64));
                }
                Ok(Value::Null)
            }
            "date_string" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let days_since_epoch = ts / 86400;
                let year = 1970 + days_since_epoch / 365;
                Ok(Value::Str(format!(
                    "day {} (year ~{})",
                    days_since_epoch, year
                )))
            }

            // ── Type coercions ─────────────────────────────────────────────────
            "to_string" | "str" => Ok(Value::Str(
                args.first().cloned().unwrap_or(Value::Null).to_string(),
            )),
            "to_number" | "int" | "float" | "num" => {
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Number(n) => Ok(Value::Number(n)),
                    Value::Bool(b) => Ok(Value::Number(if b { 1.0 } else { 0.0 })),
                    Value::Str(s) => s
                        .trim()
                        .parse::<f64>()
                        .map(Value::Number)
                        .map_err(|_| Signal::Error(format!("Cannot convert '{}' to number", s))),
                    _ => Ok(Value::Number(0.0)),
                }
            }
            "to_bool" | "bool" => Ok(Value::Bool(
                args.first().cloned().unwrap_or(Value::Null).is_truthy(),
            )),
            "type_of" | "typeof" => Ok(Value::Str(
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
            "is_number" => Ok(Value::Bool(matches!(args.first(), Some(Value::Number(_))))),
            "is_string" => Ok(Value::Bool(matches!(args.first(), Some(Value::Str(_))))),
            "is_array" => Ok(Value::Bool(matches!(args.first(), Some(Value::Array(_))))),
            "is_object" => Ok(Value::Bool(matches!(args.first(), Some(Value::Object(_))))),
            "is_bool" => Ok(Value::Bool(matches!(args.first(), Some(Value::Bool(_))))),

            // ── Collections ───────────────────────────────────────────────────
            "count" | "len" | "length" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(a) => Ok(Value::Number(a.len() as f64)),
                Value::Object(o) => Ok(Value::Number(o.len() as f64)),
                Value::Str(s) => Ok(Value::Number(s.chars().count() as f64)),
                Value::Null => Ok(Value::Number(0.0)),
                _ => Ok(Value::Number(1.0)),
            },
            "keys" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let mut ks: Vec<Value> = m.keys().map(|k| Value::Str(k.clone())).collect();
                    ks.sort_by_key(|a| a.to_string());
                    Ok(Value::Array(ks))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "values" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => Ok(Value::Array(m.values().cloned().collect())),
                _ => Ok(Value::Array(Vec::new())),
            },
            "entries" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let pairs: Vec<Value> = m
                        .iter()
                        .map(|(k, v)| Value::Array(vec![Value::Str(k.clone()), v.clone()]))
                        .collect();
                    Ok(Value::Array(pairs))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "range" => {
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                let end = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i64;
                let step = args.get(2).and_then(|v| v.as_number()).unwrap_or(1.0) as i64;
                if step == 0 {
                    return Err(Signal::Error("range step cannot be 0".into()));
                }
                let mut arr = Vec::new();
                let mut i = start;
                while if step > 0 { i < end } else { i > end } {
                    arr.push(Value::Number(i as f64));
                    i += step;
                }
                Ok(Value::Array(arr))
            }
            "zip" => {
                let a = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Array(arr) => arr,
                    _ => return Ok(Value::Array(Vec::new())),
                };
                let b = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Array(arr) => arr,
                    _ => return Ok(Value::Array(Vec::new())),
                };
                let zipped = a
                    .into_iter()
                    .zip(b)
                    .map(|(x, y)| Value::Array(vec![x, y]))
                    .collect();
                Ok(Value::Array(zipped))
            }
            "flatten" | "flat" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => {
                    let mut flat = Vec::new();
                    for v in arr {
                        match v {
                            Value::Array(inner) => flat.extend(inner),
                            other => flat.push(other),
                        }
                    }
                    Ok(Value::Array(flat))
                }
                _ => Ok(Value::Array(Vec::new())),
            },

            // ── String global functions ───────────────────────────────────────
            "trim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim().to_string()))
            }
            "trim_start" | "ltrim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim_start().to_string()))
            }
            "trim_end" | "rtrim" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.trim_end().to_string()))
            }
            "to_upper" | "upper" | "uppercase" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.to_uppercase()))
            }
            "to_lower" | "lower" | "lowercase" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.to_lowercase()))
            }
            "starts_with" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let prefix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.starts_with(prefix.as_str())))
            }
            "ends_with" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let suffix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(s.ends_with(suffix.as_str())))
            }
            "replace" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let old = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let new = args
                    .get(2)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replace(old.as_str(), new.as_str())))
            }
            "split" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let delim = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or(" ".to_string());
                let parts: Vec<Value> = s
                    .split(delim.as_str())
                    .map(|p| Value::Str(p.to_string()))
                    .collect();
                Ok(Value::Array(parts))
            }
            "join" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => {
                    let sep = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    let parts: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                    Ok(Value::Str(parts.join(sep.as_str())))
                }
                _ => Ok(Value::Str(String::new())),
            },
            "has" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(m) => {
                    let key = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    Ok(Value::Bool(m.contains_key(&key)))
                }
                Value::Array(arr) => {
                    let needle = args.get(1).cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(arr.iter().any(|v| v == &needle)))
                }
                _ => Ok(Value::Bool(false)),
            },
            "contains" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Str(s) => {
                    let sub = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    Ok(Value::Bool(s.contains(sub.as_str())))
                }
                Value::Array(arr) => {
                    let needle = args.get(1).cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(arr.iter().any(|v| v == &needle)))
                }
                _ => Ok(Value::Bool(false)),
            },
            "set_key" | "set_field" => {
                // set_key(obj, key, val) → new object with key set to val
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Object(mut m) => {
                        let key = args
                            .get(1)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default();
                        let val = args.get(2).cloned().unwrap_or(Value::Null);
                        m.insert(key, val);
                        Ok(Value::Object(m))
                    }
                    _ => Err(Signal::Error(
                        "set_key requires an object as first argument".into(),
                    )),
                }
            }
            "delete_key" | "remove_key" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Object(mut m) => {
                    let key = args
                        .get(1)
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    m.remove(&key);
                    Ok(Value::Object(m))
                }
                _ => Err(Signal::Error(
                    "delete_key requires an object as first argument".into(),
                )),
            },
            "push" => {
                // Functional push: push(array, value) → new array with value appended
                match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Array(mut arr) => {
                        if let Some(v) = args.get(1) {
                            arr.push(v.clone());
                        }
                        Ok(Value::Array(arr))
                    }
                    _ => Err(Signal::Error(
                        "push() requires an array as first argument".into(),
                    )),
                }
            }
            "pop" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(mut arr) => {
                    arr.pop();
                    Ok(Value::Array(arr))
                }
                _ => Ok(Value::Array(Vec::new())),
            },
            "first" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => Ok(arr.into_iter().next().unwrap_or(Value::Null)),
                other => Ok(other),
            },
            "last" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Array(arr) => Ok(arr.into_iter().last().unwrap_or(Value::Null)),
                other => Ok(other),
            },
            "is_empty" => match args.first().cloned().unwrap_or(Value::Null) {
                Value::Str(s) => Ok(Value::Bool(s.is_empty())),
                Value::Array(a) => Ok(Value::Bool(a.is_empty())),
                Value::Object(o) => Ok(Value::Bool(o.is_empty())),
                Value::Null => Ok(Value::Bool(true)),
                _ => Ok(Value::Bool(false)),
            },
            // substr(s, start) or substr(s, start, len)
            "substr" | "substring" => {
                let s: Vec<char> = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
                    .chars()
                    .collect();
                let start = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let result: String = if let Some(length) = args.get(2).and_then(|v| v.as_number()) {
                    s.iter().skip(start).take(length as usize).collect()
                } else {
                    s.iter().skip(start).collect()
                };
                Ok(Value::Str(result))
            }
            // strip_prefix(s, prefix) — remove prefix only from start
            "strip_prefix" | "remove_prefix" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let prefix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            // strip_suffix(s, suffix) — remove suffix only from end
            "strip_suffix" | "remove_suffix" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let suffix = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            // index_of(s, needle) — first position of needle, or -1
            "index_of" | "find" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let needle = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match s.find(needle.as_str()) {
                    Some(i) => Ok(Value::Number(i as f64)),
                    None => Ok(Value::Number(-1.0)),
                }
            }

            // ── Math ──────────────────────────────────────────────────────────
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
            "round" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let decimals = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
                let factor = 10f64.powi(decimals);
                Ok(Value::Number((n * factor).round() / factor))
            }
            "abs" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .abs(),
            )),
            "sqrt" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                if n < 0.0 {
                    return Err(Signal::Error("sqrt of negative number".into()));
                }
                Ok(Value::Number(n.sqrt()))
            }
            "pow" => {
                let base = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let exp = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(Value::Number(base.powf(exp)))
            }
            "log2" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(1.0)
                    .log2(),
            )),
            "log10" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(1.0)
                    .log10(),
            )),
            "ln" => Ok(Value::Number(
                args.first().and_then(|v| v.as_number()).unwrap_or(1.0).ln(),
            )),
            "sin" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .sin(),
            )),
            "cos" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .cos(),
            )),
            "tan" => Ok(Value::Number(
                args.first()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .tan(),
            )),
            "max" => {
                if let Some(Value::Array(arr)) = args.first() {
                    let max = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::NEG_INFINITY, f64::max);
                    return Ok(Value::Number(max));
                }
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
                if let Some(Value::Array(arr)) = args.first() {
                    let min = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::INFINITY, f64::min);
                    return Ok(Value::Number(min));
                }
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
            "clamp" => {
                let v = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let lo = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::NEG_INFINITY);
                let hi = args
                    .get(2)
                    .and_then(|v| v.as_number())
                    .unwrap_or(f64::INFINITY);
                Ok(Value::Number(v.clamp(lo, hi)))
            }
            "random" => {
                // Simple LCG pseudo-random using system time as seed
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as u64;
                let lo = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let hi = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
                // LCG
                let r = ((seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407))
                    >> 33) as f64
                    / u32::MAX as f64;
                Ok(Value::Number(lo + r * (hi - lo)))
            }
            "pi" | "PI" => Ok(Value::Number(std::f64::consts::PI)),
            "e" | "E" => Ok(Value::Number(std::f64::consts::E)),

            // ── JSON ──────────────────────────────────────────────────────────
            "json_parse" | "parse_json" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match serde_json::from_str::<serde_json::Value>(&s) {
                    Ok(json) => Ok(json_to_gx_value(&json)),
                    Err(e) => Err(Signal::Error(format!("json_parse error: {}", e))),
                }
            }
            "json_stringify" | "to_json" | "json" => {
                let val = args.first().cloned().unwrap_or(Value::Null);
                let pretty = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);
                let json = gx_value_to_json(&val);
                let s = if pretty {
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                } else {
                    serde_json::to_string(&json).unwrap_or_default()
                };
                Ok(Value::Str(s))
            }

            // ── HTTP client ───────────────────────────────────────────────────
            "http_get" | "fetch" => {
                let url = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("http_get requires a URL string".into()))?;
                let headers_val = args.get(1).cloned().unwrap_or(Value::Null);
                let mut req = ureq::get(&url);
                if let Value::Object(headers) = headers_val {
                    for (k, v) in &headers {
                        req = req.set(k, &v.to_string());
                    }
                }
                match req.call() {
                    Ok(resp) => {
                        let status = resp.status() as f64;
                        let body = resp.into_string().unwrap_or_default();
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(status < 400.0));
                        map.insert("status".into(), Value::Number(status));
                        map.insert("body".into(), Value::Str(body.clone()));
                        // Auto-parse JSON if content is JSON
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            map.insert("data".into(), json_to_gx_value(&json));
                        }
                        Ok(Value::Object(map))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("status".into(), Value::Number(code as f64));
                        map.insert("body".into(), Value::Str(body));
                        map.insert("error".into(), Value::Str(format!("HTTP {}", code)));
                        Ok(Value::Object(map))
                    }
                    Err(e) => {
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("status".into(), Value::Number(0.0));
                        map.insert("error".into(), Value::Str(e.to_string()));
                        Ok(Value::Object(map))
                    }
                }
            }
            "http_post" => {
                let url = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("http_post requires a URL string".into()))?;
                let body_val = args.get(1).cloned().unwrap_or(Value::Null);
                let headers_val = args.get(2).cloned().unwrap_or(Value::Null);
                let mut req = ureq::post(&url).set("Content-Type", "application/json");
                if let Value::Object(headers) = headers_val {
                    for (k, v) in &headers {
                        req = req.set(k, &v.to_string());
                    }
                }
                let json_body = gx_value_to_json(&body_val);
                match req.send_json(&json_body) {
                    Ok(resp) => {
                        let status = resp.status() as f64;
                        let body = resp.into_string().unwrap_or_default();
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(status < 400.0));
                        map.insert("status".into(), Value::Number(status));
                        map.insert("body".into(), Value::Str(body.clone()));
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            map.insert("data".into(), json_to_gx_value(&json));
                        }
                        Ok(Value::Object(map))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("status".into(), Value::Number(code as f64));
                        map.insert("body".into(), Value::Str(body));
                        map.insert("error".into(), Value::Str(format!("HTTP {}", code)));
                        Ok(Value::Object(map))
                    }
                    Err(e) => {
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("error".into(), Value::Str(e.to_string()));
                        Ok(Value::Object(map))
                    }
                }
            }
            "http_put" => {
                let url = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("http_put requires a URL string".into()))?;
                let body_val = args.get(1).cloned().unwrap_or(Value::Null);
                let json_body = gx_value_to_json(&body_val);
                match ureq::put(&url)
                    .set("Content-Type", "application/json")
                    .send_json(&json_body)
                {
                    Ok(resp) => {
                        let status = resp.status() as f64;
                        let body = resp.into_string().unwrap_or_default();
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(status < 400.0));
                        map.insert("status".into(), Value::Number(status));
                        map.insert("body".into(), Value::Str(body));
                        Ok(Value::Object(map))
                    }
                    Err(e) => {
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("error".into(), Value::Str(e.to_string()));
                        Ok(Value::Object(map))
                    }
                }
            }
            "http_delete" => {
                let url = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("http_delete requires a URL string".into()))?;
                match ureq::delete(&url).call() {
                    Ok(resp) => {
                        let status = resp.status() as f64;
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(status < 400.0));
                        map.insert("status".into(), Value::Number(status));
                        Ok(Value::Object(map))
                    }
                    Err(e) => {
                        let mut map = HashMap::new();
                        map.insert("ok".into(), Value::Bool(false));
                        map.insert("error".into(), Value::Str(e.to_string()));
                        Ok(Value::Object(map))
                    }
                }
            }

            // ── File I/O ──────────────────────────────────────────────────────
            "read_file" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("read_file requires a path string".into()))?;
                std::fs::read_to_string(&path)
                    .map(Value::Str)
                    .map_err(|e| Signal::Error(format!("read_file '{}': {}", path, e)))
            }
            "read_file_lines" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| {
                        Signal::Error("read_file_lines requires a path string".into())
                    })?;
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Signal::Error(format!("read_file_lines '{}': {}", path, e)))?;
                Ok(Value::Array(
                    content.lines().map(|l| Value::Str(l.to_string())).collect(),
                ))
            }
            "write_file" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("write_file requires a path".into()))?;
                let content = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()))
                    .to_string();
                std::fs::write(&path, &content)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("write_file '{}': {}", path, e)))
            }
            "append_file" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("append_file requires a path".into()))?;
                let content = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()))
                    .to_string();
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&path)
                    .map_err(|e| Signal::Error(format!("append_file '{}': {}", path, e)))?;
                f.write_all(content.as_bytes())
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("append_file write '{}': {}", path, e)))
            }
            "file_exists" | "exists" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(std::path::Path::new(&path).exists()))
            }
            "delete_file" | "remove_file" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("delete_file requires a path".into()))?;
                std::fs::remove_file(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("delete_file '{}': {}", path, e)))
            }
            "list_dir" | "read_dir" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| ".".into());
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let files: Vec<Value> = entries
                            .flatten()
                            .map(|e| Value::Str(e.file_name().to_string_lossy().into_owned()))
                            .collect();
                        Ok(Value::Array(files))
                    }
                    Err(e) => Err(Signal::Error(format!("list_dir '{}': {}", path, e))),
                }
            }
            "make_dir" | "mkdir" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("make_dir requires a path".into()))?;
                std::fs::create_dir_all(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("make_dir '{}': {}", path, e)))
            }
            "path_join" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                let mut path = std::path::PathBuf::new();
                for p in parts {
                    path.push(p);
                }
                Ok(Value::Str(path.to_string_lossy().into_owned()))
            }

            // ── Environment ───────────────────────────────────────────────────
            "env" | "get_env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                match std::env::var(&key) {
                    Ok(val) => Ok(Value::Str(val)),
                    Err(_) => Ok(default),
                }
            }
            "set_env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let val = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                std::env::set_var(&key, &val);
                Ok(Value::Null)
            }

            // ── String utilities ──────────────────────────────────────────────
            "format" => {
                // format("Hello {0}, you are {1}", name, age)
                let template = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut result = template.clone();
                for (i, arg) in args.iter().skip(1).enumerate() {
                    result = result.replace(&format!("{{{}}}", i), &arg.to_string());
                }
                Ok(Value::Str(result))
            }
            "url_encode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let encoded: String = s
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                            c.to_string()
                        } else {
                            format!("%{:02X}", c as u32)
                        }
                    })
                    .collect();
                Ok(Value::Str(encoded))
            }
            "url_decode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let decoded = s.replace('+', " ");
                // Simple percent-decode
                let mut result = String::new();
                let mut chars = decoded.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '%' {
                        let h1 = chars.next().unwrap_or('0');
                        let h2 = chars.next().unwrap_or('0');
                        if let Ok(byte) = u8::from_str_radix(&format!("{}{}", h1, h2), 16) {
                            result.push(byte as char);
                        }
                    } else {
                        result.push(c);
                    }
                }
                Ok(Value::Str(result))
            }
            "html_escape" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let escaped = s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;");
                Ok(Value::Str(escaped))
            }
            "html_unescape" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let unescaped = s
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'");
                Ok(Value::Str(unescaped))
            }
            "base64_encode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let encoded = base64_encode(s.as_bytes());
                Ok(Value::Str(encoded))
            }
            "base64_decode" => {
                let s = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match base64_decode(&s) {
                    Ok(bytes) => Ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned())),
                    Err(e) => Err(Signal::Error(format!("base64_decode: {}", e))),
                }
            }

            // ── Process / System ──────────────────────────────────────────────
            "shell" | "exec" => {
                let cmd = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("shell requires a command string".into()))?;
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .map_err(|e| Signal::Error(format!("shell exec failed: {}", e)))?;
                let mut map = HashMap::new();
                map.insert(
                    "stdout".into(),
                    Value::Str(String::from_utf8_lossy(&output.stdout).into_owned()),
                );
                map.insert(
                    "stderr".into(),
                    Value::Str(String::from_utf8_lossy(&output.stderr).into_owned()),
                );
                map.insert(
                    "exit_code".into(),
                    Value::Number(output.status.code().unwrap_or(-1) as f64),
                );
                map.insert("ok".into(), Value::Bool(output.status.success()));
                Ok(Value::Object(map))
            }
            "input" => {
                use std::io::{self, Write};
                let prompt = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                if !prompt.is_empty() {
                    print!("{}", prompt);
                    io::stdout().flush().ok();
                }
                let mut line = String::new();
                io::stdin().read_line(&mut line).ok();
                Ok(Value::Str(
                    line.trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string(),
                ))
            }
            "exit" => {
                let code = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
                std::process::exit(code);
            }

            // ── Agent management (stubs that now work) ────────────────────────
            "spawn_agent" | "spawn_helper" => {
                let n = args
                    .first()
                    .cloned()
                    .unwrap_or(Value::Str("unknown".into()));
                eprintln!("[gx] spawning agent: {}", n);
                Ok(Value::Str(format!("agent:{}", n)))
            }

            // ── Legacy stubs ──────────────────────────────────────────────────
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
            // ── Array methods ─────────────────────────────────────────────────
            (Value::Array(arr), "push") | (Value::Array(arr), "append") => {
                let mut a = arr.clone();
                for v in args {
                    a.push(v);
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "pop") => {
                let mut a = arr.clone();
                Ok(a.pop().unwrap_or(Value::Null))
            }
            (Value::Array(arr), "shift") => {
                if arr.is_empty() {
                    return Ok(Value::Null);
                }
                Ok(arr[0].clone())
            }
            (Value::Array(arr), "unshift") => {
                let mut a = arr.clone();
                if let Some(v) = args.into_iter().next() {
                    a.insert(0, v);
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "length")
            | (Value::Array(arr), "len")
            | (Value::Array(arr), "count") => Ok(Value::Number(arr.len() as f64)),
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
            (Value::Array(arr), "contains") | (Value::Array(arr), "includes") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Bool(arr.contains(&needle)))
            }
            (Value::Array(arr), "index_of") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Number(
                    arr.iter()
                        .position(|v| v == &needle)
                        .map(|i| i as f64)
                        .unwrap_or(-1.0),
                ))
            }
            (Value::Array(arr), "reverse") => {
                let mut a = arr.clone();
                a.reverse();
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "sort") => {
                let mut a = arr.clone();
                a.sort_by(|x, y| match (x, y) {
                    (Value::Number(a), Value::Number(b)) => {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    _ => x.to_string().cmp(&y.to_string()),
                });
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "sort_by") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut a = arr.clone();
                a.sort_by(|x, y| {
                    let xk = x.get_field(&key);
                    let yk = y.get_field(&key);
                    match (xk, yk) {
                        (Value::Number(a), Value::Number(b)) => {
                            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (a, b) => a.to_string().cmp(&b.to_string()),
                    }
                });
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "slice") => {
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let end = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .map(|n| n as usize)
                    .unwrap_or(arr.len());
                let end = end.min(arr.len());
                let start = start.min(end);
                Ok(Value::Array(arr[start..end].to_vec()))
            }
            (Value::Array(arr), "concat") => {
                let mut a = arr.clone();
                for arg in args {
                    match arg {
                        Value::Array(b) => a.extend(b),
                        v => a.push(v),
                    }
                }
                Ok(Value::Array(a))
            }
            (Value::Array(arr), "flat") | (Value::Array(arr), "flatten") => {
                let mut flat = Vec::new();
                for v in arr {
                    match v {
                        Value::Array(inner) => flat.extend(inner.clone()),
                        other => flat.push(other.clone()),
                    }
                }
                Ok(Value::Array(flat))
            }
            (Value::Array(arr), "unique") | (Value::Array(arr), "distinct") => {
                let mut seen = Vec::new();
                for v in arr {
                    if !seen.contains(v) {
                        seen.push(v.clone());
                    }
                }
                Ok(Value::Array(seen))
            }
            (Value::Array(arr), "sum") => {
                let s: f64 = arr.iter().filter_map(|v| v.as_number()).sum();
                Ok(Value::Number(s))
            }
            (Value::Array(arr), "min") => {
                let m = arr
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::INFINITY, f64::min);
                Ok(Value::Number(m))
            }
            (Value::Array(arr), "max") => {
                let m = arr
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Value::Number(m))
            }
            (Value::Array(arr), "average") | (Value::Array(arr), "mean") => {
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_number()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                Ok(Value::Number(nums.iter().sum::<f64>() / nums.len() as f64))
            }
            (Value::Array(arr), "find") => {
                let needle = args.first().cloned().unwrap_or(Value::Null);
                Ok(arr
                    .iter()
                    .find(|v| *v == &needle)
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            (Value::Array(arr), "filter_by") => {
                // filter_by("field", value) — filter objects by field value
                let field = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let value = args.get(1).cloned().unwrap_or(Value::Null);
                let filtered: Vec<Value> = arr
                    .iter()
                    .filter(|v| v.get_field(&field) == value)
                    .cloned()
                    .collect();
                Ok(Value::Array(filtered))
            }
            (Value::Array(arr), "map_field") => {
                // map_field("field") — extract a field from each object
                let field = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Array(
                    arr.iter().map(|v| v.get_field(&field)).collect(),
                ))
            }
            (Value::Array(arr), "take") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                Ok(Value::Array(arr.iter().take(n).cloned().collect()))
            }
            (Value::Array(arr), "skip") | (Value::Array(arr), "drop") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                Ok(Value::Array(arr.iter().skip(n).cloned().collect()))
            }
            (Value::Array(arr), "to_json") => {
                let json =
                    serde_json::to_string(&arr.iter().map(gx_value_to_json).collect::<Vec<_>>())
                        .unwrap_or_default();
                Ok(Value::Str(json))
            }

            // ── String methods ────────────────────────────────────────────────
            (Value::Str(s), "length") | (Value::Str(s), "len") | (Value::Str(s), "count") => {
                Ok(Value::Number(s.chars().count() as f64))
            }
            (Value::Str(s), "to_upper") | (Value::Str(s), "upper") => {
                Ok(Value::Str(s.to_uppercase()))
            }
            (Value::Str(s), "to_lower") | (Value::Str(s), "lower") => {
                Ok(Value::Str(s.to_lowercase()))
            }
            (Value::Str(s), "trim") => Ok(Value::Str(s.trim().to_string())),
            (Value::Str(s), "trim_start") | (Value::Str(s), "ltrim") => {
                Ok(Value::Str(s.trim_start().to_string()))
            }
            (Value::Str(s), "trim_end") | (Value::Str(s), "rtrim") => {
                Ok(Value::Str(s.trim_end().to_string()))
            }
            (Value::Str(s), "split") => {
                let sep = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or(" ".into());
                Ok(Value::Array(
                    s.split(&*sep).map(|p| Value::Str(p.to_string())).collect(),
                ))
            }
            (Value::Str(s), "split_lines") | (Value::Str(s), "lines") => Ok(Value::Array(
                s.lines().map(|l| Value::Str(l.to_string())).collect(),
            )),
            (Value::Str(s), "contains") | (Value::Str(s), "includes") => {
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
            (Value::Str(s), "replace_first") => {
                let from = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let to = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Str(s.replacen(&*from, &to, 1)))
            }
            (Value::Str(s), "index_of") => {
                let needle = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Number(
                    s.find(&*needle).map(|i| i as f64).unwrap_or(-1.0),
                ))
            }
            (Value::Str(s), "char_at") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
                let chars: Vec<char> = s.chars().collect();
                let i = if n < 0.0 {
                    (chars.len() as i64 + n as i64).max(0) as usize
                } else {
                    n as usize
                };
                Ok(chars
                    .get(i)
                    .map(|c| Value::Str(c.to_string()))
                    .unwrap_or(Value::Null))
            }
            (Value::Str(s), "slice") | (Value::Str(s), "substring") => {
                let chars: Vec<char> = s.chars().collect();
                let start = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let end = args
                    .get(1)
                    .and_then(|v| v.as_number())
                    .map(|n| n as usize)
                    .unwrap_or(chars.len());
                let end = end.min(chars.len());
                let start = start.min(end);
                Ok(Value::Str(chars[start..end].iter().collect()))
            }
            (Value::Str(s), "repeat") => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                Ok(Value::Str(s.repeat(n)))
            }
            (Value::Str(s), "pad_left") | (Value::Str(s), "pad_start") => {
                let width = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let pad_char = args
                    .get(1)
                    .and_then(|v| v.as_str().map(|s| s.chars().next().unwrap_or(' ')))
                    .unwrap_or(' ');
                let chars: Vec<char> = s.chars().collect();
                if chars.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let padding: String = std::iter::repeat_n(pad_char, width - chars.len()).collect();
                Ok(Value::Str(format!("{}{}", padding, s)))
            }
            (Value::Str(s), "pad_right") | (Value::Str(s), "pad_end") => {
                let width = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                let pad_char = args
                    .get(1)
                    .and_then(|v| v.as_str().map(|s| s.chars().next().unwrap_or(' ')))
                    .unwrap_or(' ');
                let chars: Vec<char> = s.chars().collect();
                if chars.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let padding: String = std::iter::repeat_n(pad_char, width - chars.len()).collect();
                Ok(Value::Str(format!("{}{}", s, padding)))
            }
            (Value::Str(s), "to_number") | (Value::Str(s), "parse_number") => s
                .trim()
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| Signal::Error(format!("Cannot parse '{}' as number", s))),
            (Value::Str(s), "to_json") | (Value::Str(s), "parse_json") => {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(json) => Ok(json_to_gx_value(&json)),
                    Err(e) => Err(Signal::Error(format!("parse_json: {}", e))),
                }
            }
            (Value::Str(s), "to_array") => Ok(Value::Array(
                s.chars().map(|c| Value::Str(c.to_string())).collect(),
            )),
            (Value::Str(s), "is_empty") => Ok(Value::Bool(s.is_empty())),
            (Value::Str(s), "to_upper_first") => {
                let mut chars = s.chars();
                match chars.next() {
                    None => Ok(Value::Str(String::new())),
                    Some(c) => Ok(Value::Str(
                        c.to_uppercase().collect::<String>() + chars.as_str(),
                    )),
                }
            }

            // ── Object methods ────────────────────────────────────────────────
            (Value::Object(m), "has") | (Value::Object(m), "has_key") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(Value::Bool(m.contains_key(&*key)))
            }
            (Value::Object(m), "keys") => {
                let mut ks: Vec<Value> = m.keys().map(|k| Value::Str(k.clone())).collect();
                ks.sort_by_key(|a| a.to_string());
                Ok(Value::Array(ks))
            }
            (Value::Object(m), "values") => Ok(Value::Array(m.values().cloned().collect())),
            (Value::Object(m), "entries") => {
                let pairs: Vec<Value> = m
                    .iter()
                    .map(|(k, v)| Value::Array(vec![Value::Str(k.clone()), v.clone()]))
                    .collect();
                Ok(Value::Array(pairs))
            }
            (Value::Object(m), "get") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                Ok(m.get(&*key).cloned().unwrap_or(default))
            }
            (Value::Object(m), "merge") => {
                let mut result = m.clone();
                if let Some(Value::Object(other)) = args.first() {
                    for (k, v) in other {
                        result.insert(k.clone(), v.clone());
                    }
                }
                Ok(Value::Object(result))
            }
            (Value::Object(m), "delete") | (Value::Object(m), "remove") => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut result = m.clone();
                result.remove(&*key);
                Ok(Value::Object(result))
            }
            (Value::Object(m), "len")
            | (Value::Object(m), "length")
            | (Value::Object(m), "count") => Ok(Value::Number(m.len() as f64)),
            (Value::Object(m), "to_json") => {
                let json = gx_value_to_json(&Value::Object(m.clone()));
                Ok(Value::Str(serde_json::to_string(&json).unwrap_or_default()))
            }
            (Value::Object(m), "is_empty") => Ok(Value::Bool(m.is_empty())),

            // ── Number methods ────────────────────────────────────────────────
            (Value::Number(n), "floor") => Ok(Value::Number(n.floor())),
            (Value::Number(n), "ceil") => Ok(Value::Number(n.ceil())),
            (Value::Number(n), "round") => Ok(Value::Number(n.round())),
            (Value::Number(n), "abs") => Ok(Value::Number(n.abs())),
            (Value::Number(n), "sqrt") => Ok(Value::Number(n.sqrt())),
            (Value::Number(n), "pow") => {
                let exp = args.first().and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(Value::Number(n.powf(exp)))
            }
            (Value::Number(n), "to_string") | (Value::Number(n), "str") => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    Ok(Value::Str(format!("{}", *n as i64)))
                } else {
                    Ok(Value::Str(format!("{}", n)))
                }
            }

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

// ── Bridge calls ──────────────────────────────────────────────────────────────

impl Interpreter {
    pub fn call_js(&mut self, module: &str, method: &str, args: &[Value]) -> Result<Value, Signal> {
        use crate::bridge::{json_to_value, value_to_json};
        use std::process::Command;

        let json_args = serde_json::to_string(&args.iter().map(value_to_json).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".into());

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
                "Unknown namespace '{}'. Use: js, py",
                other
            ))),
        }
    }
}

// ── Internal parse helper ─────────────────────────────────────────────────────

fn parse_gx_source(source: &str, path: &str) -> Result<crate::ast::Program, String> {
    if crate::indent_parser::is_indent_syntax(source) {
        crate::indent_parser::parse(source).map_err(|e| format!("{}: {}", path, e))
    } else {
        let tokens = crate::lexer::Lexer::new(source)
            .tokenize()
            .map_err(|e| format!("{}: {}", path, e))?;
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| format!("{}: {}", path, e))
    }
}

// ── JSON conversion helpers ───────────────────────────────────────────────────

pub fn json_to_gx_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_gx_value).collect()),
        serde_json::Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_gx_value(v)))
                .collect(),
        ),
    }
}

pub fn gx_value_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => serde_json::json!(n),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(gx_value_to_json).collect()),
        Value::Object(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), gx_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

// ── Base64 implementation (no external dep) ───────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        result.push(CHARS[b0 >> 2] as char);
        result.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=');
    let decode_char = |c: char| -> Result<u8, String> {
        match c {
            'A'..='Z' => Ok(c as u8 - b'A'),
            'a'..='z' => Ok(c as u8 - b'a' + 26),
            '0'..='9' => Ok(c as u8 - b'0' + 52),
            '+' => Ok(62),
            '/' => Ok(63),
            _ => Err(format!("Invalid base64 character: {}", c)),
        }
    };
    let chars: Vec<char> = s.chars().collect();
    let mut result = Vec::new();
    for chunk in chars.chunks(4) {
        let b0 = decode_char(chunk[0])?;
        let b1 = if chunk.len() > 1 {
            decode_char(chunk[1])?
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            decode_char(chunk[2])?
        } else {
            0
        };
        let b3 = if chunk.len() > 3 {
            decode_char(chunk[3])?
        } else {
            0
        };
        result.push((b0 << 2) | (b1 >> 4));
        if chunk.len() > 2 {
            result.push(((b1 & 0xf) << 4) | (b2 >> 2));
        }
        if chunk.len() > 3 {
            result.push(((b2 & 3) << 6) | b3);
        }
    }
    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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
    fn test_while_loop() {
        run(r#"
helper "wh" {
  brain {
    plan { }
    execute {
      i = 0
      while i < 3 {
        i += 1
      }
      log(i)
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_break_continue() {
        run(r#"
helper "bc" {
  brain {
    plan { }
    execute {
      total = 0
      for each i in [1, 2, 3, 4, 5] {
        if i == 4 { break }
        if i == 2 { continue }
        total += i
      }
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert() {
        run(r#"
helper "asserttest" {
  brain {
    plan { }
    execute {
      x = 2 + 2
      assert x == 4 "math must work"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_null_coalesce() {
        run(r#"
helper "nc" {
  brain {
    plan { }
    execute {
      a = null
      b = a ?? "default"
      log(b)
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
    fn test_array_push_mutates() {
        run(r#"
helper "arrays" {
  brain {
    plan { }
    execute {
      items = ["a", "b"]
      items.push("c")
      log(items.length)
      assert items.length == 3 "push should mutate in place"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_array_methods_extended() {
        run(r#"
helper "arrmethods" {
  brain {
    plan { }
    execute {
      nums = [3, 1, 4, 1, 5, 9, 2, 6]
      sorted = nums.sort()
      assert sorted.first() == 1 "sort: first should be 1"
      assert sorted.last() == 9 "sort: last should be 9"
      assert nums.sum() == 31 "sum should be 31"
      assert nums.min() == 1 "min should be 1"
      assert nums.max() == 9 "max should be 9"
      uniq = [1, 2, 2, 3, 3, 3].unique()
      assert uniq.length == 3 "unique should remove duplicates"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_string_methods_extended() {
        run(r#"
helper "strmethods" {
  brain {
    plan { }
    execute {
      s = "  Hello World  "
      assert s.trim() == "Hello World" "trim"
      assert s.trim().to_lower() == "hello world" "to_lower"
      assert "abc".repeat(3) == "abcabcabc" "repeat"
      assert "hello".index_of("ll") == 2 "index_of"
      assert "hi".pad_left(5, "0") == "000hi" "pad_left"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_json_builtins() {
        run(r#"
helper "jsontest" {
  brain {
    plan { }
    execute {
      data = { name: "GX", version: 1 }
      s = json_stringify(data)
      parsed = json_parse(s)
      assert parsed.name == "GX" "json round-trip name"
      assert parsed.version == 1 "json round-trip version"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_math_builtins() {
        run(r#"
helper "mathtest" {
  brain {
    plan { }
    execute {
      assert sqrt(9) == 3 "sqrt(9)"
      assert pow(2, 10) == 1024 "pow(2,10)"
      assert abs(-5) == 5 "abs(-5)"
      assert floor(3.9) == 3 "floor"
      assert ceil(3.1) == 4 "ceil"
      assert clamp(15, 0, 10) == 10 "clamp high"
      assert clamp(-5, 0, 10) == 0 "clamp low"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_file_io() {
        let tmp = std::env::temp_dir().join("gx_test_file.txt");
        let path = tmp.to_string_lossy().replace('\\', "/");
        let src = format!(
            r#"
helper "filetest" {{
  brain {{
    plan {{ }}
    execute {{
      write_file("{path}", "hello gx")
      content = read_file("{path}")
      assert content == "hello gx" "file round-trip"
      assert file_exists("{path}") "file exists"
      delete_file("{path}")
      assert not file_exists("{path}") "file deleted"
    }}
    remember {{ }}
    communicate {{ }}
  }}
}}"#
        );
        run(&src).unwrap();
    }

    #[test]
    fn test_negative_indexing() {
        run(r#"
helper "negidx" {
  brain {
    plan { }
    execute {
      arr = [1, 2, 3, 4, 5]
      assert arr[-1] == 5 "last element"
      assert arr[-2] == 4 "second to last"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }
}
