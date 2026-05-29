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
    Respond(String, String, u16), // (content_type, body, status_code)
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
    event_bus: HashMap<String, Vec<Value>>,
    #[allow(dead_code)]
    js_bridge: Option<Bridge>,
    py_bridge: Option<Bridge>,
    pub base_path: Option<String>,
    pub assert_count: usize,
    pub assert_failures: Vec<String>,
    /// When set, output goes here instead of stdout (used by WASM playground)
    pub output_capture: Option<Vec<String>>,
    // #14 readiness: agents call ready() to signal they accept messages
    ready_agents: std::collections::HashSet<String>,
    // messages queued for not-yet-ready agents: agent_name -> [(event, payload)]
    queued_messages: HashMap<String, Vec<(String, Value)>>,
    // name of the currently-running helper (set by run_helper)
    current_agent: Option<String>,
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
            output_capture: None,
            ready_agents: std::collections::HashSet::new(),
            queued_messages: HashMap::new(),
            current_agent: None,
        }
    }

    /// Route output to a capture buffer instead of stdout.
    pub fn enable_capture(&mut self) {
        self.output_capture = Some(Vec::new());
    }

    /// Flush captured output as a single string.
    pub fn captured_output(&self) -> String {
        self.output_capture
            .as_ref()
            .map(|v| v.join("\n"))
            .unwrap_or_default()
    }

    fn emit_output(&mut self, line: &str) {
        if let Some(buf) = &mut self.output_capture {
            buf.push(line.to_string());
        } else {
            println!("{}", line);
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
            // Skip auto-running callable agents (those whose brain uses `input`).
            // They are only executed when called via `spawn agent`.
            if helper_is_callable_only(h) {
                continue;
            }
            self.run_helper(h).map_err(|e| match e {
                Signal::Error(m) => m,
                Signal::AssertFail(m) => format!("Assertion failed: {}", m),
                Signal::Return(_) => "Unexpected return at top level".into(),
                Signal::ReRun => "Unexpected re-run at top level".into(),
                Signal::Break => "Unexpected break at top level".into(),
                Signal::Continue => "Unexpected continue at top level".into(),
                Signal::EscalateToHuman => "Escalated to human".into(),
                Signal::Respond(_, _, _) => "Unexpected respond outside of route handler".into(),
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
        self.current_agent = Some(helper.name.clone());
        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));

        // v0.2.0: store goal in memory if declared
        if let Some(ref goal) = helper.goal {
            memory.insert("goal".into(), Value::Str(goal.clone()));
        }

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
                match self.run_stmts(&wb.body, &mut env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
                memory = env.get_memory();
            }
        }

        // Brain cycle (with v0.2.0 retry/on_error support)
        if let Some(brain) = &helper.brain.clone() {
            let max_retries = helper.retry.unwrap_or(0) as usize;
            let on_error_policy = helper.on_error.as_deref().unwrap_or("escalate");
            let mut retry_count = 0;
            let mut cycles = 0;
            const MAX_CYCLES: usize = 100;
            'outer: loop {
                env.set_memory(memory.clone());
                env.set("plan", Value::Null);
                env.set("result", Value::Null);

                match self.run_brain(brain, &mut env) {
                    Ok(_) => {}
                    Err(Signal::ReRun) if cycles < MAX_CYCLES => {
                        cycles += 1;
                        memory = env.get_memory();
                        continue 'outer;
                    }
                    Err(Signal::ReRun) => {
                        return Err(Signal::Error(format!(
                            "Helper '{}' exceeded {} re-run cycles",
                            helper.name, MAX_CYCLES
                        )));
                    }
                    Err(Signal::Error(ref msg)) if retry_count < max_retries => {
                        retry_count += 1;
                        eprintln!(
                            "[gx] Helper '{}' error (retry {}/{}): {}",
                            helper.name, retry_count, max_retries, msg
                        );
                        continue 'outer;
                    }
                    Err(e @ Signal::Error(_)) => match on_error_policy {
                        "continue" => {
                            eprintln!(
                                "[gx] on_error: continue — ignoring error in '{}'",
                                helper.name
                            );
                        }
                        _ => return Err(e),
                    },
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
                .filter(|wb| !matches!(wb.trigger, WhenTrigger::Started | WhenTrigger::Cron(_))),
        );
        for wb in &extra_when {
            env.set_memory(memory.clone());
            match &wb.trigger {
                WhenTrigger::Started => {}
                WhenTrigger::Expr(cond) => {
                    let v = self.eval_expr(cond, &mut env)?;
                    if v.is_truthy() {
                        match self.run_stmts(&wb.body, &mut env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
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
                        match self.run_stmts(&wb.body, &mut env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
                        memory = env.get_memory();
                    }
                }
                // Phase 5: when message "event" { ... }
                WhenTrigger::Message(event) => {
                    let bus_key = format!("{}:{}", helper.name, event);
                    if let Some(messages) = self.event_bus.remove(&bus_key) {
                        for msg in messages {
                            env.set_memory(memory.clone());
                            env.set("message", msg);
                            match self.run_stmts(&wb.body, &mut env) {
                                Ok(_) | Err(Signal::Return(_)) => {}
                                Err(e) => return Err(e),
                            }
                            memory = env.get_memory();
                        }
                    }
                }
                // when cron "*/5 * * * *" { ... } — fires if expr matches current time
                WhenTrigger::Cron(_expr) => {
                    // Cron agents are run via run_cron_helper; skip here
                }
            }
        }

        // Cron daemon: if any when-cron blocks exist, enter blocking loop
        let cron_blocks: Vec<_> = helper
            .when_blocks
            .iter()
            .filter(|wb| matches!(&wb.trigger, WhenTrigger::Cron(_)))
            .cloned()
            .collect();
        if !cron_blocks.is_empty() {
            env.set_memory(memory.clone());
            self.run_cron_daemon(&cron_blocks, &mut env)?;
        }

        Ok(())
    }

    /// Phase 5: call another agent by name, inject `input`, return communicate value.
    pub fn call_agent(&mut self, name: &str, input: Value) -> IResult {
        let helper = self.helpers.get(name).cloned().ok_or_else(|| {
            Signal::Error(format!(
                "Agent '{}' not defined. Make sure it is declared before use.",
                name
            ))
        })?;

        let mut env = Env::new();
        env.set("input", input);

        // Initialize memory
        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));
        for entry in &helper.memory {
            let val = self.eval_expr(&entry.value.clone(), &mut env)?;
            memory.insert(entry.key.clone(), val);
        }
        env.set_memory(memory);

        // Run brain if present — collect communicate return value
        if let Some(brain) = &helper.brain.clone() {
            macro_rules! run_phase {
                ($stmts:expr) => {
                    match self.run_stmts($stmts, &mut env) {
                        Ok(_) | Err(Signal::Return(_)) => {}
                        Err(e) => return Err(e),
                    }
                };
            }
            run_phase!(&brain.plan);
            run_phase!(&brain.execute);
            run_phase!(&brain.remember);
            return self.eval_communicate(&brain.communicate, &mut env);
        }

        // Run when started block
        for wb in &helper.when_blocks.clone() {
            if matches!(wb.trigger, WhenTrigger::Started) {
                match self.run_stmts(&wb.body, &mut env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        // No brain — return the env's last meaningful value or Null
        Ok(Value::Null)
    }

    /// Run communicate stmts and return the last expression value (for call_agent).
    fn eval_communicate(&mut self, stmts: &[Stmt], env: &mut Env) -> IResult {
        let mut last = Value::Null;
        for stmt in stmts {
            match stmt {
                Stmt::Expr { expr, .. } => {
                    last = self.eval_expr(expr, env)?;
                    // Don't auto-print — caller uses log() if they want output
                }
                Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                    let v = self.eval_expr(value, env)?;
                    self.emit_output(&v.to_string());
                    last = v;
                }
                Stmt::Return { value, .. } => {
                    return Ok(match value {
                        Some(e) => self.eval_expr(e, env)?,
                        None => Value::Null,
                    });
                }
                other => {
                    self.run_stmt(other, env)?;
                }
            }
        }
        Ok(last)
    }

    fn run_brain(&mut self, brain: &BrainBlock, env: &mut Env) -> IResult {
        macro_rules! run_phase {
            ($stmts:expr) => {
                match self.run_stmts($stmts, env) {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
            };
        }
        run_phase!(&brain.plan);
        run_phase!(&brain.execute);
        run_phase!(&brain.remember);
        run_phase!(&brain.communicate);
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
                catch_kind,
                catch_var,
                catch_body,
                ..
            } => match self.run_stmts(try_body, env) {
                Ok(v) => Ok(v),
                Err(signal @ (Signal::Error(_) | Signal::AssertFail(_))) => {
                    // #18: build typed error object — AssertFail always maps to AssertionError
                    let (msg, kind) = match signal {
                        Signal::AssertFail(m) => (m, "AssertionError"),
                        Signal::Error(m) => {
                            let k = infer_error_kind(&m);
                            (m, k)
                        }
                        _ => unreachable!(),
                    };
                    // If a type filter is set, only catch matching kinds
                    if let Some(required_kind) = catch_kind {
                        if required_kind != kind {
                            return Err(Signal::Error(msg));
                        }
                    }
                    let mut err_map = HashMap::new();
                    err_map.insert("message".into(), Value::Str(msg));
                    err_map.insert("kind".into(), Value::Str(kind.into()));
                    err_map.insert("code".into(), Value::Null);
                    env.set(catch_var, Value::Object(err_map));
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

            // Phase 5: send "event" to "agent" with { key: val }
            Stmt::SendMessage {
                agent_name,
                event,
                data,
                ..
            } => {
                let name_val = self.eval_expr(agent_name, env)?;
                let target = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "send: agent name must be a string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                map.insert("event".into(), Value::Str(event.clone()));
                for (k, v) in data {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let msg = Value::Object(map);
                // Deliver synchronously to the target agent's when message handlers
                let helper = self.helpers.get(&target).cloned();
                if let Some(h) = helper {
                    let handlers: Vec<_> = h
                        .when_blocks
                        .iter()
                        .filter(|wb| matches!(&wb.trigger, WhenTrigger::Message(e) if e == event))
                        .cloned()
                        .collect();
                    if !handlers.is_empty() {
                        let mut msg_env = Env::new();
                        msg_env.set("message", msg.clone());
                        for wb in &handlers {
                            self.run_stmts(&wb.body, &mut msg_env)?;
                        }
                        return Ok(Value::Null);
                    }
                }
                // No handler found — queue for deferred processing
                let bus_key = format!("{}:{}", target, event);
                self.event_bus.entry(bus_key).or_default().push(msg);
                Ok(Value::Null)
            }

            Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                let v = self.eval_expr(value, env)?;
                self.emit_output(&v.to_string());
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

            Stmt::Respond {
                format,
                value,
                status,
                ..
            } => {
                let v = self.eval_expr(value, env)?;
                let body = match format.as_str() {
                    "json" => value_to_json(&v),
                    _ => v.to_string(),
                };
                let content_type = match format.as_str() {
                    "json" => "application/json; charset=utf-8".to_string(),
                    "html" => "text/html; charset=utf-8".to_string(),
                    _ => "text/plain; charset=utf-8".to_string(),
                };
                Err(Signal::Respond(content_type, body, *status))
            }

            Stmt::Serve { port, routes, .. } => {
                #[cfg(not(target_arch = "wasm32"))]
                return self.run_serve(port, routes, env);
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (port, routes);
                    return Err(Signal::Error(
                        "HTTP server not available in playground".into(),
                    ));
                }
            }

            // ── v0.2.0 opinionated sugar ──────────────────────────────────────
            Stmt::Think {
                prompt,
                model,
                temperature,
                min_confidence,
                into_var,
                ..
            } => {
                let prompt_val = self.eval_expr(prompt, env)?;
                let provider = model.as_deref().unwrap_or("openai");
                let temp_val = match temperature {
                    Some(t) => self.eval_expr(t, env)?,
                    None => Value::Number(0.7),
                };
                let min_conf = match min_confidence {
                    Some(mc) => self.eval_expr(mc, env)?.as_number().unwrap_or(0.0),
                    None => 0.0,
                };
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mut params: HashMap<String, Value> = HashMap::new();
                    params.insert("prompt".into(), prompt_val);
                    params.insert("temperature".into(), temp_val);
                    let result = ai::ask_ai(provider, None, &params);
                    if min_conf > 0.0 {
                        let conf = if let Value::Object(ref m) = result {
                            m.get("confidence")
                                .and_then(|v| v.as_number())
                                .unwrap_or(1.0)
                        } else {
                            1.0
                        };
                        if conf < min_conf {
                            eprintln!(
                                "[gx think] confidence {:.2} below threshold {:.2} — escalating",
                                conf, min_conf
                            );
                            self.events.push(("escalate_to_human".into(), Vec::new()));
                            return Err(Signal::EscalateToHuman);
                        }
                    }
                    env.set(into_var, result);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (temp_val, min_conf);
                    let mut m = HashMap::new();
                    m.insert(
                        "text".into(),
                        Value::Str("[AI not available in playground]".into()),
                    );
                    m.insert("confidence".into(), Value::Number(1.0));
                    m.insert("ok".into(), Value::Bool(false));
                    env.set(into_var, Value::Object(m));
                }
                Ok(Value::Null)
            }

            Stmt::Observe { bindings, .. } => {
                for (key, val_expr) in bindings {
                    let v = self.eval_expr(val_expr, env)?;
                    env.set(key, v);
                }
                Ok(Value::Null)
            }

            Stmt::Act { body, .. } => self.run_stmts(body, env),

            Stmt::LoopUntil {
                condition, body, ..
            } => {
                let mut iterations = 0usize;
                loop {
                    if iterations > 10_000 {
                        return Err(Signal::Error("loop until exceeded 10000 iterations".into()));
                    }
                    let cond_val = self.eval_expr(condition, env)?;
                    if cond_val.is_truthy() {
                        break;
                    }
                    match self.run_stmts(body, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => {}
                        Err(e) => return Err(e),
                    }
                    iterations += 1;
                }
                Ok(Value::Null)
            }

            Stmt::RepeatTimes {
                count, var, body, ..
            } => {
                let n = self.eval_expr(count, env)?.as_number().unwrap_or(0.0) as usize;
                for i in 0..n {
                    if let Some(v) = var {
                        env.set(v, Value::Number(i as f64));
                    }
                    match self.run_stmts(body, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(Signal::Continue) => {}
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }

            Stmt::Parallel { branches, .. } => {
                // Sequential fallback — true parallelism deferred to Phase 8
                for branch in branches {
                    match self.run_stmts(branch, env) {
                        Ok(_) => {}
                        Err(Signal::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
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
                // Pipeline: lv |> spawn agent "name" → call_agent(name, lv)
                if matches!(op, BinOp::Pipe) {
                    let input = self.eval_expr(left, env)?;
                    // RHS must resolve to a CallAgent expr
                    match right.as_ref() {
                        Expr::CallAgent {
                            name, input: extra, ..
                        } => {
                            let name_val = self.eval_expr(name, env)?;
                            let agent_name = match &name_val {
                                Value::Str(s) => s.clone(),
                                _ => {
                                    return Err(Signal::Error(format!(
                                        "Pipeline: agent name must be a string, got {}",
                                        name_val.type_name()
                                    )))
                                }
                            };
                            // Merge input value + extra { } fields.
                            // Non-object scalars are wrapped as { value: X } for clean chaining.
                            let mut map = match input {
                                Value::Object(m) => m,
                                other => {
                                    let mut m = HashMap::new();
                                    m.insert("value".into(), other);
                                    m
                                }
                            };
                            for (k, v) in extra {
                                map.insert(k.clone(), self.eval_expr(v, env)?);
                            }
                            return self.call_agent(&agent_name, Value::Object(map));
                        }
                        other => {
                            // Evaluate RHS as a function/value and apply
                            let _ = other;
                            return Err(Signal::Error(
                                "|> right side must be `spawn agent` or `call agent`".into(),
                            ));
                        }
                    }
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
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let (ns, mo, me) = (namespace.clone(), module.clone(), method.clone());
                    self.bridge_call(&ns, &mo, &me, &resolved)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (namespace, module, method, resolved);
                    Err(Signal::Error(
                        "JS/Python bridge not available in playground".into(),
                    ))
                }
            }

            // Phase 5: spawn agent "name" with { key: val } [timeout Nms]
            Expr::CallAgent {
                name,
                input,
                timeout_ms,
            } => {
                let name_val = self.eval_expr(name, env)?;
                let agent_name = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "Agent name must be a string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                for (k, v) in input {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let input_val = Value::Object(map);

                if let Some(t_expr) = timeout_ms {
                    let ms = self.eval_expr(t_expr, env)?.as_number().unwrap_or(5000.0) as u64;
                    self.call_agent_with_timeout(&agent_name, input_val, ms)
                } else {
                    self.call_agent(&agent_name, input_val)
                }
            }

            Expr::Lambda { params, body } => Ok(Value::Closure(params.clone(), body.clone())),

            Expr::ParallelMap(branches) => {
                let mut named_branches = Vec::new();
                for (k, expr) in branches {
                    named_branches.push((k.clone(), expr.clone()));
                }
                self.eval_parallel_map(named_branches, env)
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
            BinOp::Pipe => Err(Signal::Error(
                "|> pipeline: right side must be `spawn agent` or `call agent`".into(),
            )),
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

        // Direct lambda call: fn(x) { x + 1 }(5)
        if let Expr::Lambda { params, body } = callee {
            let func = FunctionDef {
                name: "<lambda>".into(),
                params: params.clone(),
                body: body.clone(),
                line: 0,
            };
            let mem = env.get_memory();
            return self.call_user_function(&func, args, Some(mem));
        }

        if let Expr::Ident(name) = callee {
            // Check if a closure value is stored under this name
            let val = env.get(name);
            if let Value::Closure(params, body) = val {
                let func = FunctionDef {
                    name: name.clone(),
                    params,
                    body,
                    line: 0,
                };
                let mem = env.get_memory();
                return self.call_user_function(&func, args, Some(mem));
            }
            // Also check memory
            let mem = env.get_memory();
            if let Some(Value::Closure(params, body)) = mem.get(name).cloned() {
                let func = FunctionDef {
                    name: name.clone(),
                    params,
                    body,
                    line: 0,
                };
                return self.call_user_function(&func, args, Some(mem));
            }
            if let Some(func) = self.functions.get(name).cloned() {
                return self.call_user_function(&func, args, Some(mem));
            }
            return self.eval_builtin(name, args, env);
        }

        Err(Signal::Error(format!("Cannot call {:?}", callee)))
    }

    // ── #6 spawn agent with timeout ───────────────────────────────────────────

    fn call_agent_with_timeout(
        &mut self,
        agent_name: &str,
        input_val: Value,
        timeout_ms: u64,
    ) -> IResult {
        use std::sync::mpsc;
        use std::time::Duration;

        let helpers = self.helpers.clone();
        let functions = self.functions.clone();
        let agent = agent_name.to_string();
        let input_json = crate::interpreter::gx_value_to_json(&input_val);

        let (tx, rx) = mpsc::channel::<Result<serde_json::Value, String>>();

        std::thread::spawn(move || {
            let mut child = Interpreter::new();
            child.helpers = helpers;
            child.functions = functions;
            let input = crate::interpreter::json_to_gx_value(&input_json);
            let result = child
                .call_agent(&agent, input)
                .map(|v| crate::interpreter::gx_value_to_json(&v))
                .map_err(|e| format!("{:?}", e));
            let _ = tx.send(result);
        });

        match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(Ok(json)) => Ok(json_to_gx_value(&json)),
            Ok(Err(e)) => Err(Signal::Error(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut map = HashMap::new();
                map.insert("timed_out".into(), Value::Bool(true));
                map.insert("agent".into(), Value::Str(agent_name.into()));
                map.insert("timeout_ms".into(), Value::Number(timeout_ms as f64));
                Ok(Value::Object(map))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Signal::Error(format!(
                "Agent '{}' thread panicked",
                agent_name
            ))),
        }
    }

    // ── #7 parallel named results ─────────────────────────────────────────────

    fn eval_parallel_map(&mut self, branches: Vec<(String, Expr)>, env: &mut Env) -> IResult {
        use std::sync::mpsc;

        let mut handles: Vec<(String, mpsc::Receiver<Result<serde_json::Value, String>>)> =
            Vec::new();

        for (key, expr) in &branches {
            // Only parallelize CallAgent exprs; evaluate others synchronously
            if let Expr::CallAgent {
                name,
                input,
                timeout_ms,
            } = expr
            {
                let name_val = self.eval_expr(name, env)?;
                let agent_name = match &name_val {
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(format!(
                            "parallel: agent name must be string, got {}",
                            name_val.type_name()
                        )))
                    }
                };
                let mut map = HashMap::new();
                for (k, v) in input {
                    map.insert(k.clone(), self.eval_expr(v, env)?);
                }
                let input_val = Value::Object(map);
                let input_json = gx_value_to_json(&input_val);
                let helpers = self.helpers.clone();
                let functions = self.functions.clone();
                let agent = agent_name.clone();
                let timeout = timeout_ms
                    .as_ref()
                    .and_then(|e| self.eval_expr(e, env).ok())
                    .and_then(|v| v.as_number())
                    .map(|n| n as u64);

                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut child = Interpreter::new();
                    child.helpers = helpers;
                    child.functions = functions;
                    let input = json_to_gx_value(&input_json);
                    let result = child
                        .call_agent(&agent, input)
                        .map(|v| gx_value_to_json(&v))
                        .map_err(|e| format!("{:?}", e));
                    if let Some(ms) = timeout {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                    let _ = tx.send(result);
                });
                handles.push((key.clone(), rx));
            } else {
                // Non-agent expr: evaluate synchronously, wrap in a pseudo-handle
                let val = self.eval_expr(expr, env)?;
                let json = gx_value_to_json(&val);
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Ok(json));
                handles.push((key.clone(), rx));
            }
        }

        let mut result_map = HashMap::new();
        for (key, rx) in handles {
            use std::time::Duration;
            let entry = match rx.recv_timeout(Duration::from_secs(300)) {
                Ok(Ok(json)) => json_to_gx_value(&json),
                Ok(Err(e)) => {
                    let mut em = HashMap::new();
                    em.insert("error".into(), Value::Str(e));
                    Value::Object(em)
                }
                Err(_) => {
                    let mut em = HashMap::new();
                    em.insert("timed_out".into(), Value::Bool(true));
                    Value::Object(em)
                }
            };
            result_map.insert(key, entry);
        }
        Ok(Value::Object(result_map))
    }

    // ── #15 cron daemon ───────────────────────────────────────────────────────

    fn run_cron_daemon(&mut self, blocks: &[WhenBlock], env: &mut Env) -> Result<(), Signal> {
        eprintln!("[gx] cron daemon started ({} schedule(s))", blocks.len());
        loop {
            let now = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            };
            for wb in blocks {
                if let WhenTrigger::Cron(expr) = &wb.trigger {
                    if cron_matches(expr, now) {
                        match self.run_stmts(&wb.body, env) {
                            Ok(_) | Err(Signal::Return(_)) => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            // Sleep until the start of the next minute
            let secs_into_minute = now % 60;
            let sleep_secs = 60 - secs_into_minute;
            std::thread::sleep(std::time::Duration::from_secs(sleep_secs));
        }
    }

    fn call_closure(&mut self, callee: &Value, args: Vec<Value>, env: &mut Env) -> IResult {
        match callee {
            Value::Closure(params, body) => {
                let func = FunctionDef {
                    name: "<closure>".into(),
                    params: params.clone(),
                    body: body.clone(),
                    line: 0,
                };
                let mem = env.get_memory();
                self.call_user_function(&func, args, Some(mem))
            }
            Value::Str(name) => {
                let name = name.clone();
                if let Some(func) = self.functions.get(&name).cloned() {
                    let mem = env.get_memory();
                    return self.call_user_function(&func, args, Some(mem));
                }
                Err(Signal::Error(format!(
                    "'{}' is not a defined function",
                    name
                )))
            }
            _ => Err(Signal::Error(format!(
                "Expected function, got {}",
                callee.type_name()
            ))),
        }
    }

    fn call_behavior(&mut self, func: &FunctionDef, caller_env: &mut Env) -> IResult {
        let mut env = Env::new();
        let mem = caller_env.get_memory();
        env.set_memory(mem.clone());
        // Flatten memory into local vars so bare names work in progressive syntax:
        // `count += 1` inside a behavior acts like `memory.count += 1`
        let initial: HashMap<String, Value> = mem.clone();
        for (k, v) in &mem {
            env.set(k, v.clone());
        }
        let body = func.body.clone();
        let result = match self.run_stmts(&body, &mut env) {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        // memory.X assignments are captured in env.get_memory()
        let mut new_mem = env.get_memory();
        // Local var wins only if it actually changed — this lets memory.X = ... and bare x = ...
        // coexist: whichever was actually modified wins, with local var taking priority on conflict
        for k in initial.keys() {
            let local_val = env.get(k);
            let was = initial.get(k).cloned().unwrap_or(Value::Null);
            if local_val != was && !matches!(local_val, Value::Null) {
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

    fn eval_builtin(&mut self, name: &str, args: Vec<Value>, env: &mut Env) -> IResult {
        match name {
            // ── Output ────────────────────────────────────────────────────────
            "log" | "output" | "print" | "say" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                self.emit_output(&parts.join(" "));
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
            // Character primitives for self-hosting
            "ord" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Number(
                    c.chars().next().map(|ch| ch as u32 as f64).unwrap_or(0.0),
                ))
            }
            "chr" => {
                let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
                Ok(char::from_u32(n)
                    .map(|c| Value::Str(c.to_string()))
                    .unwrap_or(Value::Null))
            }
            "is_digit" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_digit())
                        .unwrap_or(false),
                ))
            }
            "is_alpha" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_alphabetic())
                        .unwrap_or(false),
                ))
            }
            "is_alnum" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_ascii_alphanumeric())
                        .unwrap_or(false),
                ))
            }
            "is_whitespace" => {
                let c = args
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Ok(Value::Bool(
                    c.chars()
                        .next()
                        .map(|ch| ch.is_whitespace())
                        .unwrap_or(false),
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
            "http_get" | "fetch" | "http_post" | "http_put" | "http_delete" => {
                http_builtin(name, &args)
            }

            // http_request { url, method, body, headers } — unified form
            "http_request" => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let url = match &opts {
                    Value::Object(m) => m
                        .get("url")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default(),
                    Value::Str(s) => s.clone(),
                    _ => {
                        return Err(Signal::Error(
                            "http_request: expected object with url field".into(),
                        ))
                    }
                };
                let method = match &opts {
                    Value::Object(m) => m
                        .get("method")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "GET".into()),
                    _ => "GET".into(),
                };
                let body_val = match &opts {
                    Value::Object(m) => m.get("body").cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                let _headers: Vec<(String, String)> = match &opts {
                    Value::Object(m) => match m.get("headers") {
                        Some(Value::Object(hm)) => hm
                            .iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                // Delegate to http_builtin with synthesised args
                let method_name = match method.to_uppercase().as_str() {
                    "POST" => "http_post",
                    "PUT" => "http_put",
                    "DELETE" => "http_delete",
                    _ => "http_get",
                };
                let mut builtin_args = vec![Value::Str(url)];
                if !matches!(body_val, Value::Null) {
                    builtin_args.push(body_val);
                }
                http_builtin(method_name, &builtin_args)
            }

            // ── #17 HTTP streaming ────────────────────────────────────────────
            "http_stream" => http_stream_builtin(&args),

            // ── #8 HTTP multipart upload ──────────────────────────────────────
            "http_upload" => http_upload_builtin(&args),

            // send_email { to, subject, body, smtp_host?, smtp_port?, from?, username?, password? }
            "send_email" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let opts = args.first().cloned().unwrap_or(Value::Null);
                    let get_str = |key: &str| match &opts {
                        Value::Object(m) => m
                            .get(key)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    let to = get_str("to");
                    let subject = get_str("subject");
                    let body_s = get_str("body");
                    let from = get_str("from");
                    let smtp_host = get_str("smtp_host");
                    let smtp_port = match &opts {
                        Value::Object(m) => m
                            .get("smtp_port")
                            .and_then(|v| v.as_number())
                            .unwrap_or(587.0) as u16,
                        _ => 587,
                    };
                    let username = get_str("username");
                    let password = get_str("password");

                    // Use SMTP env vars as fallback
                    let smtp_host = if smtp_host.is_empty() {
                        std::env::var("SMTP_HOST").unwrap_or_else(|_| "localhost".into())
                    } else {
                        smtp_host
                    };
                    let from = if from.is_empty() {
                        std::env::var("SMTP_FROM").unwrap_or_else(|_| "gx@localhost".into())
                    } else {
                        from
                    };
                    let username = if username.is_empty() {
                        std::env::var("SMTP_USER").unwrap_or_default()
                    } else {
                        username
                    };
                    let password = if password.is_empty() {
                        std::env::var("SMTP_PASS").unwrap_or_default()
                    } else {
                        password
                    };

                    let raw = format!(
                        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body_s}"
                    );

                    use std::io::{BufRead, BufReader, Write};
                    use std::net::TcpStream;
                    let stream = TcpStream::connect(format!("{}:{}", smtp_host, smtp_port))
                        .map_err(|e| Signal::Error(format!("send_email: connect failed: {}", e)))?;
                    let mut w = std::io::BufWriter::new(stream.try_clone().unwrap());
                    let mut r = BufReader::new(stream);
                    let mut line = String::new();
                    let smtp_read =
                        |r: &mut BufReader<TcpStream>, line: &mut String| -> Result<(), Signal> {
                            line.clear();
                            r.read_line(line)
                                .map_err(|e| Signal::Error(format!("send_email: read: {}", e)))?;
                            Ok(())
                        };
                    smtp_read(&mut r, &mut line)?;
                    let cmd =
                        |w: &mut std::io::BufWriter<TcpStream>, s: &str| -> Result<(), Signal> {
                            w.write_all(format!("{}\r\n", s).as_bytes())
                                .map_err(|e| Signal::Error(format!("send_email: write: {}", e)))?;
                            w.flush()
                                .map_err(|e| Signal::Error(format!("send_email: flush: {}", e)))
                        };
                    cmd(&mut w, "EHLO localhost")?;
                    smtp_read(&mut r, &mut line)?;
                    while line.contains('-') {
                        smtp_read(&mut r, &mut line)?;
                    }
                    if !username.is_empty() {
                        cmd(&mut w, "AUTH LOGIN")?;
                        smtp_read(&mut r, &mut line)?;
                        cmd(&mut w, &base64_encode(username.as_bytes()))?;
                        smtp_read(&mut r, &mut line)?;
                        cmd(&mut w, &base64_encode(password.as_bytes()))?;
                        smtp_read(&mut r, &mut line)?;
                    }
                    cmd(&mut w, &format!("MAIL FROM:<{}>", from))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, &format!("RCPT TO:<{}>", to))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, "DATA")?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, &format!("{}\r\n.", raw))?;
                    smtp_read(&mut r, &mut line)?;
                    cmd(&mut w, "QUIT")?;
                    Ok(Value::Bool(true))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error(
                    "send_email not available in playground".into(),
                ))
            }

            // scrape "url" → cleaned plain text
            "scrape" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let url = args
                        .first()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    let html = ureq::get(&url)
                        .call()
                        .map_err(|e| Signal::Error(format!("scrape: fetch failed: {}", e)))?
                        .into_string()
                        .map_err(|e| Signal::Error(format!("scrape: read failed: {}", e)))?;
                    // Strip HTML tags naively
                    let text = strip_html_tags(&html);
                    Ok(Value::Str(text))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error("scrape not available in playground".into()))
            }

            // notify { channel: "slack"|"webhook", url, message }
            "notify" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let opts = args.first().cloned().unwrap_or(Value::Null);
                    let get_str = |key: &str| match &opts {
                        Value::Object(m) => m
                            .get(key)
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    let channel = get_str("channel");
                    let message = get_str("message");
                    let url = if channel == "slack" {
                        std::env::var("SLACK_WEBHOOK_URL").unwrap_or_else(|_| get_str("url"))
                    } else {
                        get_str("url")
                    };
                    if url.is_empty() {
                        return Err(Signal::Error(
                            "notify: url or SLACK_WEBHOOK_URL required".into(),
                        ));
                    }
                    let payload = format!("{{\"text\":\"{}\"}}", message.replace('"', "\\\""));
                    ureq::post(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&payload)
                        .map_err(|e| Signal::Error(format!("notify: failed: {}", e)))?;
                    Ok(Value::Bool(true))
                }
                #[cfg(target_arch = "wasm32")]
                Err(Signal::Error("notify not available in playground".into()))
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
            // Parse a GX source string using the Rust parser and return the AST
            // as a GX value tree (same format as parser.gx output) for use with
            // js_codegen.gx. This bypasses the slow GX-written parser.
            "parse_gx" => {
                let src = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let parse_result = if crate::indent_parser::is_indent_syntax(&src) {
                    crate::indent_parser::parse(&src)
                } else {
                    crate::lexer::Lexer::new(&src)
                        .tokenize()
                        .and_then(|toks| crate::parser::Parser::new(toks).parse())
                };
                match parse_result {
                    Ok(program) => Ok(gx_ast_to_value(&program)),
                    Err(e) => {
                        eprintln!("[gx] parse_gx error: {}", e);
                        Ok(Value::Null)
                    }
                }
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
                    .stdin(std::process::Stdio::inherit())
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

            // ── Readiness (#14) ───────────────────────────────────────────────
            "ready" => {
                if let Some(name) = self.current_agent.clone() {
                    self.ready_agents.insert(name.clone());
                    // Flush any messages queued before ready() was called
                    if let Some(queued) = self.queued_messages.remove(&name) {
                        for (event, payload) in queued {
                            let bus_key = format!("{}:{}", name, event);
                            self.event_bus.entry(bus_key).or_default().push(payload);
                        }
                    }
                }
                Ok(Value::Null)
            }

            // ── Env ───────────────────────────────────────────────────────────
            "env_require" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("env_require: expected string key".into()))?;
                match std::env::var(&key) {
                    Ok(val) => Ok(Value::Str(val)),
                    Err(_) => Err(Signal::Error(format!(
                        "Missing required environment variable: {}",
                        key
                    ))),
                }
            }

            // ── Structured logging ────────────────────────────────────────────
            "log_debug" | "log_info" | "log_warn" | "log_error" => {
                let level = match name {
                    "log_debug" => "DEBUG",
                    "log_info" => "INFO",
                    "log_warn" => "WARN",
                    _ => "ERROR",
                };
                let min_level = std::env::var("GX_LOG_LEVEL")
                    .unwrap_or_else(|_| "INFO".into())
                    .to_uppercase();
                let levels = ["DEBUG", "INFO", "WARN", "ERROR"];
                let min_idx = levels
                    .iter()
                    .position(|&l| l == min_level.as_str())
                    .unwrap_or(1);
                let cur_idx = levels.iter().position(|&l| l == level).unwrap_or(1);
                if cur_idx >= min_idx {
                    let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                    eprintln!("[{}] {}", level, parts.join(" "));
                }
                Ok(Value::Null)
            }

            // ── Object utilities ──────────────────────────────────────────────
            "object_merge" => {
                let mut result: HashMap<String, Value> = HashMap::new();
                for arg in &args {
                    if let Value::Object(map) = arg {
                        for (k, v) in map {
                            result.insert(k.clone(), v.clone());
                        }
                    } else {
                        return Err(Signal::Error(format!(
                            "object_merge: expected object, got {}",
                            arg.type_name()
                        )));
                    }
                }
                Ok(Value::Object(result))
            }

            // ── Regex ─────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "regex_match" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_match: expected pattern string".into()))?;
                let text = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_match: expected text string".into()))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| Signal::Error(format!("regex_match: invalid pattern: {}", e)))?;
                Ok(Value::Bool(re.is_match(&text)))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "regex_find_all" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_find_all: expected pattern".into()))?;
                let text = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_find_all: expected text".into()))?;
                let re = regex::Regex::new(&pattern).map_err(|e| {
                    Signal::Error(format!("regex_find_all: invalid pattern: {}", e))
                })?;
                let matches: Vec<Value> = re
                    .find_iter(&text)
                    .map(|m| Value::Str(m.as_str().to_string()))
                    .collect();
                Ok(Value::Array(matches))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "regex_replace" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_replace: expected pattern".into()))?;
                let replacement = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_replace: expected replacement".into()))?;
                let text = args
                    .get(2)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_replace: expected text".into()))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| Signal::Error(format!("regex_replace: invalid pattern: {}", e)))?;
                Ok(Value::Str(
                    re.replace_all(&text, replacement.as_str()).to_string(),
                ))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "regex_split" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_split: expected pattern".into()))?;
                let text = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("regex_split: expected text".into()))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| Signal::Error(format!("regex_split: invalid pattern: {}", e)))?;
                let parts: Vec<Value> =
                    re.split(&text).map(|s| Value::Str(s.to_string())).collect();
                Ok(Value::Array(parts))
            }

            // ── SQLite ────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "db_query" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected db path".into()))?;
                let sql = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected SQL string".into()))?;
                let params: Vec<Value> = args.into_iter().skip(2).collect();
                db_query_impl(&path, &sql, params)
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_exec" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_exec: expected db path".into()))?;
                let sql = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_exec: expected SQL string".into()))?;
                let params: Vec<Value> = args.into_iter().skip(2).collect();
                db_exec_impl(&path, &sql, params)
            }

            // ── Functional: map / filter ──────────────────────────────────────
            "map" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("map: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("map: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                let mut result = Vec::new();
                for item in items {
                    let out = self.call_closure(&func, vec![item], env)?;
                    result.push(out);
                }
                Ok(Value::Array(result))
            }
            "filter" => {
                let arr = args
                    .first()
                    .cloned()
                    .ok_or_else(|| Signal::Error("filter: expected array".into()))?;
                let func = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| Signal::Error("filter: expected function".into()))?;
                let items = arr.iter().map_err(Signal::Error)?;
                let mut result = Vec::new();
                for item in items {
                    let keep = self.call_closure(&func, vec![item.clone()], env)?;
                    if keep.is_truthy() {
                        result.push(item);
                    }
                }
                Ok(Value::Array(result))
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

#[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(not(target_arch = "wasm32"))]
    fn run_serve(&mut self, port_expr: &Expr, routes: &[RouteDecl], env: &mut Env) -> IResult {
        let port = self
            .eval_expr(port_expr, env)?
            .as_number()
            .unwrap_or(3000.0) as u16;
        let addr = format!("0.0.0.0:{}", port);
        let server = tiny_http::Server::http(&addr)
            .map_err(|e| Signal::Error(format!("Cannot start server on port {}: {}", port, e)))?;
        println!("GX server listening on http://localhost:{}", port);
        println!("Press Ctrl+C to stop.");
        for mut request in server.incoming_requests() {
            let method = request.method().to_string().to_uppercase();
            let url = request.url().to_string();
            let (path, query) = if let Some(q) = url.find('?') {
                (url[..q].to_string(), url[q + 1..].to_string())
            } else {
                (url.clone(), String::new())
            };
            let mut body_str = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut body_str);
            // Build request object available inside handler
            let mut req_map = HashMap::new();
            req_map.insert("method".to_string(), Value::Str(method.clone()));
            req_map.insert("path".to_string(), Value::Str(path.clone()));
            req_map.insert("body".to_string(), Value::Str(body_str.clone()));
            req_map.insert("query".to_string(), Value::Str(query.clone()));
            // Parse body as JSON if possible
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                req_map.insert("json".to_string(), json_to_gx_value(&json));
            }
            // Match route
            let matched = routes
                .iter()
                .find(|r| (r.method == method || r.method == "ANY") && r.path == path);
            let (ct, body, status) = if let Some(route) = matched {
                let mut route_env = env.clone();
                route_env.set("request", Value::Object(req_map));
                match self.run_stmts(&route.body.clone(), &mut route_env) {
                    Ok(_) => ("text/plain; charset=utf-8".into(), "OK".into(), 200u16),
                    Err(Signal::Respond(ct, b, s)) => (ct, b, s),
                    Err(Signal::Error(e)) => (
                        "text/plain; charset=utf-8".into(),
                        format!("500 Internal Error: {}", e),
                        500,
                    ),
                    Err(_) => ("text/plain; charset=utf-8".into(), "OK".into(), 200),
                }
            } else {
                (
                    "text/plain; charset=utf-8".into(),
                    format!("404 Not Found: {} {}", method, path),
                    404,
                )
            };
            let ct_header =
                tiny_http::Header::from_bytes(b"Content-Type".as_ref(), ct.as_bytes()).unwrap();
            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(ct_header);
            let _ = request.respond(response);
        }
        Ok(Value::Null)
    }
}

fn value_to_json(v: &Value) -> String {
    serde_json::to_string(&gx_value_to_json(v)).unwrap_or_else(|_| "null".into())
}

/// Strip HTML tags, decode common entities, collapse whitespace
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Collapse whitespace
    let mut result = String::new();
    let mut prev_ws = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result.trim().to_string()
}

// ── Callable-agent detector ───────────────────────────────────────────────────

/// Returns true if the helper's brain accesses `input`, meaning it's designed
/// to be called via `spawn agent` rather than run standalone.
fn helper_is_callable_only(h: &HelperDef) -> bool {
    if let Some(brain) = &h.brain {
        stmts_use_input(&brain.plan)
            || stmts_use_input(&brain.execute)
            || stmts_use_input(&brain.remember)
            || stmts_use_input(&brain.communicate)
    } else {
        false
    }
}

fn stmts_use_input(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_uses_input)
}

fn stmt_uses_input(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Assign { value, .. } => expr_uses_input(value),
        Stmt::PlusAssign { value, .. }
        | Stmt::MinusAssign { value, .. }
        | Stmt::MulAssign { value, .. }
        | Stmt::DivAssign { value, .. } => expr_uses_input(value),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches
                .iter()
                .any(|(c, b)| expr_uses_input(c) || stmts_use_input(b))
                || else_body.as_deref().is_some_and(stmts_use_input)
        }
        Stmt::ForEach { iter, body, .. } => expr_uses_input(iter) || stmts_use_input(body),
        Stmt::While {
            condition, body, ..
        } => expr_uses_input(condition) || stmts_use_input(body),
        Stmt::TryCatch {
            try_body,
            catch_body,
            ..
        } => stmts_use_input(try_body) || stmts_use_input(catch_body),
        Stmt::Log { value, .. }
        | Stmt::Output { value, .. }
        | Stmt::Say { value, .. }
        | Stmt::Return {
            value: Some(value), ..
        }
        | Stmt::Assert {
            condition: value, ..
        }
        | Stmt::Wait { ms: value, .. } => expr_uses_input(value),
        Stmt::Expr { expr, .. } => expr_uses_input(expr),
        Stmt::Think {
            prompt,
            temperature,
            min_confidence,
            ..
        } => {
            expr_uses_input(prompt)
                || temperature.as_ref().is_some_and(expr_uses_input)
                || min_confidence.as_ref().is_some_and(expr_uses_input)
        }
        Stmt::Observe { bindings, .. } => bindings.iter().any(|(_, v)| expr_uses_input(v)),
        Stmt::Act { body, .. } => stmts_use_input(body),
        Stmt::LoopUntil {
            condition, body, ..
        } => expr_uses_input(condition) || stmts_use_input(body),
        Stmt::RepeatTimes { count, body, .. } => expr_uses_input(count) || stmts_use_input(body),
        Stmt::Parallel { branches, .. } => branches.iter().any(|b| stmts_use_input(b)),
        _ => false,
    }
}

fn expr_uses_input(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(n) => n == "input",
        Expr::FieldAccess { object, .. } => expr_uses_input(object),
        Expr::Index { object, index } => expr_uses_input(object) || expr_uses_input(index),
        Expr::Call { callee, args } => expr_uses_input(callee) || args.iter().any(expr_uses_input),
        Expr::BinOp { left, right, .. } => expr_uses_input(left) || expr_uses_input(right),
        Expr::Not(inner) => expr_uses_input(inner),
        Expr::Array(items) => items.iter().any(expr_uses_input),
        Expr::Object(pairs) => pairs.iter().any(|(_, v)| expr_uses_input(v)),
        Expr::Interpolated(parts) => parts.iter().any(|p| match p {
            InterpolatedPart::Expr(e) => expr_uses_input(e),
            _ => false,
        }),
        Expr::CallAgent { name, input, .. } => {
            expr_uses_input(name) || input.iter().any(|(_, v)| expr_uses_input(v))
        }
        Expr::ParallelMap(branches) => branches.iter().any(|(_, v)| expr_uses_input(v)),
        _ => false,
    }
}

// ── HTTP builtins (native only) ───────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn http_builtin(name: &str, _args: &[Value]) -> Result<Value, Signal> {
    let mut map = HashMap::new();
    map.insert("ok".into(), Value::Bool(false));
    map.insert(
        "error".into(),
        Value::Str(format!("{} not available in playground", name)),
    );
    Ok(Value::Object(map))
}

#[cfg(not(target_arch = "wasm32"))]
fn http_builtin(name: &str, args: &[Value]) -> Result<Value, Signal> {
    match name {
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
        _ => Err(Signal::Error(format!("Unknown HTTP builtin: {}", name))),
    }
}

// ── #17 HTTP streaming (native only) ─────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn http_stream_builtin(_args: &[Value]) -> Result<Value, Signal> {
    Err(Signal::Error(
        "http_stream not available in playground".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn http_stream_builtin(args: &[Value]) -> Result<Value, Signal> {
    use std::io::{BufRead, BufReader};

    let opts = args.first().cloned().unwrap_or(Value::Null);
    let url = match &opts {
        Value::Object(m) => m
            .get("url")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        Value::Str(s) => s.clone(),
        _ => {
            return Err(Signal::Error(
                "http_stream: expected {url, method?, body?}".into(),
            ))
        }
    };
    let method = match &opts {
        Value::Object(m) => m
            .get("method")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "GET".into()),
        _ => "GET".into(),
    };
    let body_val = match &opts {
        Value::Object(m) => m.get("body").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };

    let resp = match method.to_uppercase().as_str() {
        "POST" => {
            let json_body = gx_value_to_json(&body_val);
            ureq::post(&url)
                .set("Content-Type", "application/json")
                .send_json(&json_body)
                .map_err(|e| Signal::Error(format!("http_stream POST failed: {}", e)))?
        }
        _ => ureq::get(&url)
            .call()
            .map_err(|e| Signal::Error(format!("http_stream GET failed: {}", e)))?,
    };

    let reader = BufReader::new(resp.into_reader());
    let mut chunks: Vec<Value> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(l) => chunks.push(Value::Str(l)),
            Err(e) => return Err(Signal::Error(format!("http_stream read error: {}", e))),
        }
    }
    Ok(Value::Array(chunks))
}

// ── #8 HTTP multipart upload (native only) ────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn http_upload_builtin(_args: &[Value]) -> Result<Value, Signal> {
    Err(Signal::Error(
        "http_upload not available in playground".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn http_upload_builtin(args: &[Value]) -> Result<Value, Signal> {
    let opts = args
        .first()
        .ok_or_else(|| Signal::Error("http_upload: expected {url, fields?, files?}".into()))?;
    let map = match opts {
        Value::Object(m) => m,
        _ => {
            return Err(Signal::Error(
                "http_upload: expected object argument".into(),
            ))
        }
    };

    let url = map
        .get("url")
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("http_upload: 'url' field required".into()))?;

    // Build multipart/form-data body manually
    use std::time::{SystemTime, UNIX_EPOCH};
    let boundary = format!(
        "GXBoundary{:016x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );

    let mut body: Vec<u8> = Vec::new();

    // Text fields
    if let Some(Value::Object(fields)) = map.get("fields") {
        for (k, v) in fields {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", k).as_bytes(),
            );
            body.extend_from_slice(v.to_string().as_bytes());
            body.extend_from_slice(b"\r\n");
        }
    }

    // File fields
    if let Some(Value::Object(files)) = map.get("files") {
        for (field_name, path_val) in files {
            let path = path_val.as_str().ok_or_else(|| {
                Signal::Error(format!(
                    "http_upload: file path for '{}' must be a string",
                    field_name
                ))
            })?;
            let file_data = std::fs::read(path).map_err(|e| {
                Signal::Error(format!("http_upload: cannot read '{}': {}", path, e))
            })?;
            let filename = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                    field_name, filename
                )
                .as_bytes(),
            );
            body.extend_from_slice(&file_data);
            body.extend_from_slice(b"\r\n");
        }
    }

    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let content_type = format!("multipart/form-data; boundary={}", boundary);
    let resp = ureq::post(&url)
        .set("Content-Type", &content_type)
        .send_bytes(&body)
        .map_err(|e| Signal::Error(format!("http_upload failed: {}", e)))?;

    let status = resp.status() as f64;
    let resp_body = resp.into_string().unwrap_or_default();
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(status < 400.0));
    result.insert("status".into(), Value::Number(status));
    result.insert("body".into(), Value::Str(resp_body.clone()));
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp_body) {
        result.insert("data".into(), json_to_gx_value(&json));
    }
    Ok(Value::Object(result))
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
        Value::Closure(params, _) => {
            serde_json::Value::String(format!("<fn({})>", params.join(", ")))
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

// ── GX AST → GX Value conversion (for parse_gx builtin) ──────────────────────
// Converts the Rust AST produced by the native parser into GX Value objects
// with the same shape expected by js_codegen.gx (matching parser.gx output).

fn gx_ast_to_value(program: &crate::ast::Program) -> Value {
    let mut stmts: Vec<Value> = Vec::new();
    // Top-level functions
    for f in &program.functions {
        stmts.push(funcdef_to_value(&f.name, &f.params, &f.body));
    }
    // Top-level brain (top-level statements outside any agent/function)
    if let Some(brain) = &program.top_level_brain {
        for s in &brain.plan {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.execute {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.remember {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.communicate {
            stmts.push(stmt_to_value(s));
        }
    }
    // Helpers/agents: expose their functions and when-started blocks
    for h in &program.helpers {
        for f in &h.recipes {
            stmts.push(funcdef_to_value(&f.name, &[], &f.brain.plan));
        }
        for wb in &h.when_blocks {
            if matches!(wb.trigger, crate::ast::WhenTrigger::Started) {
                for s in &wb.body {
                    stmts.push(stmt_to_value(s));
                }
            }
        }
    }
    obj(&[
        ("tag", Value::Str("Program".into())),
        ("stmts", Value::Array(stmts)),
    ])
}

fn funcdef_to_value(name: &str, params: &[String], body: &[Stmt]) -> Value {
    obj(&[
        ("tag", Value::Str("FuncDef".into())),
        ("name", Value::Str(name.into())),
        (
            "params",
            Value::Array(params.iter().map(|p| Value::Str(p.clone())).collect()),
        ),
        ("body", stmts_to_value(body)),
    ])
}

fn stmts_to_value(stmts: &[Stmt]) -> Value {
    Value::Array(stmts.iter().map(stmt_to_value).collect())
}

fn stmt_to_value(s: &Stmt) -> Value {
    match s {
        Stmt::Assign { target, value, .. } => obj(&[
            ("tag", Value::Str("Assign".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::PlusAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("PlusEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::MinusAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("MinusEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::MulAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("MulEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::DivAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("DivEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            let br_vals: Vec<Value> = branches
                .iter()
                .map(|(cond, body)| {
                    obj(&[
                        ("cond", expr_to_value(cond)),
                        ("body", stmts_to_value(body)),
                    ])
                })
                .collect();
            obj(&[
                ("tag", Value::Str("If".into())),
                ("branches", Value::Array(br_vals)),
                (
                    "else_body",
                    else_body
                        .as_ref()
                        .map(|b| stmts_to_value(b))
                        .unwrap_or(Value::Null),
                ),
            ])
        }
        Stmt::While {
            condition, body, ..
        } => obj(&[
            ("tag", Value::Str("While".into())),
            ("cond", expr_to_value(condition)),
            ("body", stmts_to_value(body)),
        ]),
        Stmt::ForEach {
            var, iter, body, ..
        } => obj(&[
            ("tag", Value::Str("For".into())),
            ("var", Value::Str(var.clone())),
            ("iter", expr_to_value(iter)),
            ("body", stmts_to_value(body)),
        ]),
        Stmt::Return { value, .. } => obj(&[
            ("tag", Value::Str("Return".into())),
            (
                "value",
                value.as_ref().map(expr_to_value).unwrap_or(Value::Null),
            ),
        ]),
        Stmt::Break { .. } => obj(&[("tag", Value::Str("Break".into()))]),
        Stmt::Continue { .. } => obj(&[("tag", Value::Str("Continue".into()))]),
        Stmt::Log { value, .. } | Stmt::Output { value, .. } => obj(&[
            ("tag", Value::Str("Log".into())),
            ("value", expr_to_value(value)),
        ]),
        Stmt::Say { value, .. } => obj(&[
            ("tag", Value::Str("Say".into())),
            ("value", expr_to_value(value)),
        ]),
        Stmt::Assert {
            condition, message, ..
        } => obj(&[
            ("tag", Value::Str("Assert".into())),
            ("cond", expr_to_value(condition)),
            (
                "msg",
                message.as_ref().map(expr_to_value).unwrap_or(Value::Null),
            ),
        ]),
        Stmt::TryCatch {
            try_body,
            catch_var,
            catch_body,
            ..
        } => obj(&[
            ("tag", Value::Str("TryCatch".into())),
            ("try_body", stmts_to_value(try_body)),
            ("catch_var", Value::Str(catch_var.clone())),
            ("catch_body", stmts_to_value(catch_body)),
        ]),
        Stmt::Expr { expr, .. } => obj(&[
            ("tag", Value::Str("ExprStmt".into())),
            ("expr", expr_to_value(expr)),
        ]),
        _ => obj(&[
            ("tag", Value::Str("ExprStmt".into())),
            ("expr", Value::Null),
        ]),
    }
}

fn expr_to_value(e: &Expr) -> Value {
    match e {
        Expr::Num(n) => obj(&[
            ("tag", Value::Str("Num".into())),
            ("value", Value::Number(*n)),
        ]),
        Expr::Str(s) => obj(&[
            ("tag", Value::Str("Str".into())),
            ("value", Value::Str(s.clone())),
        ]),
        Expr::Bool(b) => obj(&[
            ("tag", Value::Str("Bool".into())),
            ("value", Value::Bool(*b)),
        ]),
        Expr::Null => obj(&[("tag", Value::Str("Null".into()))]),
        Expr::Ident(name) => obj(&[
            ("tag", Value::Str("Ident".into())),
            ("name", Value::Str(name.clone())),
        ]),
        Expr::FieldAccess { object, field } => obj(&[
            ("tag", Value::Str("Field".into())),
            ("obj", expr_to_value(object)),
            ("field", Value::Str(field.clone())),
        ]),
        Expr::Index { object, index } => obj(&[
            ("tag", Value::Str("Index".into())),
            ("obj", expr_to_value(object)),
            ("idx", expr_to_value(index)),
        ]),
        Expr::Call { callee, args } => {
            // Detect method call: callee is FieldAccess
            if let Expr::FieldAccess { object, field } = callee.as_ref() {
                let arg_vals: Vec<Value> = args.iter().map(expr_to_value).collect();
                return obj(&[
                    ("tag", Value::Str("MethodCall".into())),
                    ("obj", expr_to_value(object)),
                    ("method", Value::Str(field.clone())),
                    ("args", Value::Array(arg_vals)),
                ]);
            }
            let arg_vals: Vec<Value> = args.iter().map(expr_to_value).collect();
            obj(&[
                ("tag", Value::Str("Call".into())),
                ("callee", expr_to_value(callee)),
                ("args", Value::Array(arg_vals)),
            ])
        }
        Expr::Array(items) => obj(&[
            ("tag", Value::Str("Array".into())),
            (
                "items",
                Value::Array(items.iter().map(expr_to_value).collect()),
            ),
        ]),
        Expr::Object(pairs) => {
            let pair_vals: Vec<Value> = pairs
                .iter()
                .map(|(k, v)| obj(&[("key", Value::Str(k.clone())), ("value", expr_to_value(v))]))
                .collect();
            obj(&[
                ("tag", Value::Str("Object".into())),
                ("pairs", Value::Array(pair_vals)),
            ])
        }
        Expr::BinOp { left, op, right } => {
            if matches!(op, BinOp::NullCoalesce) {
                return obj(&[
                    ("tag", Value::Str("NullCoal".into())),
                    ("left", expr_to_value(left)),
                    ("right", expr_to_value(right)),
                ]);
            }
            let op_str = match op {
                BinOp::Add | BinOp::Concat => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::Eq => "==",
                BinOp::NotEq => "!=",
                BinOp::Lt => "<",
                BinOp::LtEq => "<=",
                BinOp::Gt => ">",
                BinOp::GtEq => ">=",
                BinOp::And => "and",
                BinOp::Or => "or",
                BinOp::Pipe => "|>",
                BinOp::NullCoalesce => "??",
            };
            obj(&[
                ("tag", Value::Str("BinOp".into())),
                ("op", Value::Str(op_str.into())),
                ("left", expr_to_value(left)),
                ("right", expr_to_value(right)),
            ])
        }
        Expr::Not(inner) => obj(&[
            ("tag", Value::Str("Unary".into())),
            ("op", Value::Str("not".into())),
            ("expr", expr_to_value(inner)),
        ]),
        Expr::Interpolated(parts) => {
            let part_vals: Vec<Value> = parts
                .iter()
                .map(|p| match p {
                    InterpolatedPart::Literal(s) => obj(&[
                        ("tag", Value::Str("Lit".into())),
                        ("v", Value::Str(s.clone())),
                    ]),
                    InterpolatedPart::Expr(e) => {
                        obj(&[("tag", Value::Str("Expr".into())), ("e", expr_to_value(e))])
                    }
                })
                .collect();
            obj(&[
                ("tag", Value::Str("Interp".into())),
                ("parts", Value::Array(part_vals)),
            ])
        }
        _ => obj(&[("tag", Value::Str("Null".into()))]),
    }
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let map: std::collections::HashMap<String, Value> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    Value::Object(map)
}

// ── SQLite helpers (native only) ──────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn gx_to_sqlite(val: &Value) -> rusqlite::types::ToSqlOutput<'static> {
    use rusqlite::types::{ToSqlOutput, Value as SqlValue};
    match val {
        Value::Null => ToSqlOutput::Owned(SqlValue::Null),
        Value::Bool(b) => ToSqlOutput::Owned(SqlValue::Integer(*b as i64)),
        Value::Number(n) => {
            if n.fract() == 0.0 {
                ToSqlOutput::Owned(SqlValue::Integer(*n as i64))
            } else {
                ToSqlOutput::Owned(SqlValue::Real(*n))
            }
        }
        Value::Str(s) => ToSqlOutput::Owned(SqlValue::Text(s.clone())),
        other => ToSqlOutput::Owned(SqlValue::Text(other.to_string())),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sqlite_to_gx(val: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match val {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(i as f64),
        ValueRef::Real(f) => Value::Number(f),
        ValueRef::Text(s) => Value::Str(String::from_utf8_lossy(s).to_string()),
        ValueRef::Blob(b) => Value::Str(base64_encode(b)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn db_query_impl(path: &str, sql: &str, params: Vec<Value>) -> Result<Value, Signal> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| Signal::Error(format!("db_query: cannot open '{}': {}", path, e)))?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| Signal::Error(format!("db_query: SQL error: {}", e)))?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let rows_iter = stmt
        .query(rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_query: query error: {}", e)))?;
    let mut rows_iter = rows_iter;
    let mut rows: Vec<Value> = Vec::new();
    loop {
        match rows_iter.next() {
            Ok(Some(row)) => {
                let mut map = HashMap::new();
                for (i, col) in col_names.iter().enumerate() {
                    let v = sqlite_to_gx(row.get_ref(i).unwrap_or(rusqlite::types::ValueRef::Null));
                    map.insert(col.clone(), v);
                }
                rows.push(Value::Object(map));
            }
            Ok(None) => break,
            Err(e) => return Err(Signal::Error(format!("db_query: row error: {}", e))),
        }
    }
    Ok(Value::Array(rows))
}

#[cfg(not(target_arch = "wasm32"))]
fn db_exec_impl(path: &str, sql: &str, params: Vec<Value>) -> Result<Value, Signal> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| Signal::Error(format!("db_exec: cannot open '{}': {}", path, e)))?;
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let affected = conn
        .execute(sql, rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_exec: SQL error: {}", e)))?;
    Ok(Value::Number(affected as f64))
}

// ── #18 Typed error helpers ───────────────────────────────────────────────────

fn infer_error_kind(msg: &str) -> &'static str {
    let lower = msg.to_lowercase();
    if lower.contains("json") || lower.contains("parse") || lower.contains("invalid") {
        "JsonParseError"
    } else if lower.contains("network")
        || lower.contains("connection")
        || lower.contains("timeout")
        || lower.contains("http")
    {
        "NetworkError"
    } else if lower.contains("permission") || lower.contains("access denied") {
        "PermissionError"
    } else if lower.contains("not found") || lower.contains("no such file") {
        "NotFoundError"
    } else if lower.contains("assert") {
        "AssertionError"
    } else {
        "RuntimeError"
    }
}

// ── Cron expression matcher ───────────────────────────────────────────────────
// Supports: *, n, */n, n-m  for each of the 5 fields (min hour dom mon dow)

fn cron_field_matches(field: &str, value: u64, min: u64, max: u64) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(step) = field.strip_prefix("*/") {
        let step: u64 = step.parse().unwrap_or(1);
        return step > 0 && value.is_multiple_of(step);
    }
    if field.contains('-') {
        let parts: Vec<&str> = field.splitn(2, '-').collect();
        if parts.len() == 2 {
            let lo: u64 = parts[0].parse().unwrap_or(min);
            let hi: u64 = parts[1].parse().unwrap_or(max);
            return value >= lo && value <= hi;
        }
    }
    field.parse::<u64>().map(|n| n == value).unwrap_or(false)
}

fn cron_matches(expr: &str, unix_secs: u64) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() < 5 {
        return false;
    }
    // Convert unix_secs to calendar fields (UTC)
    let total_minutes = unix_secs / 60;
    let minute = total_minutes % 60;
    let total_hours = total_minutes / 60;
    let hour = total_hours % 24;
    let total_days = total_hours / 24;
    // Day of week: 1970-01-01 was a Thursday (4)
    let dow = (total_days + 4) % 7;
    // Rough month/dom (close enough for scheduling)
    let year = 1970 + total_days / 365;
    let day_of_year = total_days % 365;
    let dom = day_of_year % 31 + 1;
    let month = day_of_year / 31 + 1;
    let _ = year;

    cron_field_matches(fields[0], minute, 0, 59)
        && cron_field_matches(fields[1], hour, 0, 23)
        && cron_field_matches(fields[2], dom, 1, 31)
        && cron_field_matches(fields[3], month, 1, 12)
        && cron_field_matches(fields[4], dow, 0, 6)
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
