//! GX Interpreter — executes the AST produced by the parser.

mod bridge_impl;
mod builtins_ast;
mod builtins_base64;
mod builtins_data;
mod builtins_datetime;
mod builtins_db;
mod builtins_http;
mod builtins_json;
mod builtins_regex;
mod builtins_vector;
mod util;

use crate::ai;
use crate::ast::*;
use crate::bridge::Bridge;
use crate::value::Value;
use std::collections::HashMap;

// Re-export public JSON helpers so other crates can use them.
pub use builtins_json::{gx_value_to_json, json_to_gx_value};

// Bring extracted free functions into scope for use inside eval_call_expr.
use builtins_ast::gx_ast_to_value;
use builtins_base64::{base64_decode, base64_encode};
use builtins_data::{
    csv_parse_impl, csv_stringify_impl, toml_parse_impl, toml_stringify_impl, yaml_parse_impl,
    yaml_stringify_impl,
};
use builtins_datetime::{
    date_add_impl, date_diff_impl, date_format_impl, date_from_parts_impl, date_now_impl,
    date_parse_impl, date_parts_impl, date_timestamp_impl,
};
use builtins_db::{db_exec_impl, db_query_impl};
#[cfg(not(target_arch = "wasm32"))]
use builtins_http::check_url_safe;
use builtins_http::{http_builtin, http_stream_builtin, http_upload_builtin};
use builtins_regex::{
    regex_captures_impl, regex_find_all_impl, regex_find_impl, regex_named_captures_impl,
    regex_replace_impl, regex_split_impl, regex_test_impl,
};
use builtins_vector::{
    cosine_similarity_impl, vector_store_add_impl, vector_store_delete_impl, vector_store_new_impl,
    vector_store_search_impl, vector_store_size_impl,
};
use util::{
    cron_matches, helper_is_callable_only, infer_error_kind, normalize_path_no_symlink,
    parse_gx_source, strip_html_tags, value_to_json,
};

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

pub(crate) type IResult = Result<Value, Signal>;

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
    // Security flags — set explicitly; all default to off (safe)
    pub allow_shell: bool,
    pub allow_internal_http: bool,
    pub sandbox_dir: Option<std::path::PathBuf>,
    // None = open mode (no gx.json); Some(list) = restrict to list
    pub allowed_js_modules: Option<Vec<String>>,
    pub allowed_py_modules: Option<Vec<String>>,
    /// Module registry: alias → list of functions from that module.
    /// Used so that intra-module calls (e.g. `pad_right` calling `repeat_str`)
    /// resolve correctly when the module is loaded under a namespace.
    module_functions: HashMap<String, Vec<FunctionDef>>,
    /// Binary/Go/Rust subprocess bridges keyed by "namespace:path".
    binary_bridges: HashMap<String, crate::bridge::Bridge>,
    /// When true, every AI call / tool call / memory write emits a JSONL trace line to stderr.
    pub trace_enabled: bool,
    /// Tool definitions registered at the program level (for AI function-calling).
    pub(crate) tools: HashMap<String, crate::ast::ToolDef>,
    /// When true, removes the iteration cap on while/loop. Use for REPL and I/O-bound loops.
    pub no_loop_limit: bool,
    /// Variables assigned at file root (top_level_stmts). Injected into every agent's env
    /// so they are accessible as normal locals alongside memory.*.
    global_vars: HashMap<String, Value>,
    /// Call stack of frame names (agent / function / closure) for error traces.
    call_stack: Vec<String>,
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
            allow_shell: false,
            allow_internal_http: false,
            sandbox_dir: None,
            allowed_js_modules: None,
            allowed_py_modules: None,
            module_functions: HashMap::new(),
            binary_bridges: HashMap::new(),
            trace_enabled: false,
            tools: HashMap::new(),
            no_loop_limit: false,
            global_vars: HashMap::new(),
            call_stack: Vec::new(),
        }
    }

    /// Resolve `path_str` against the sandbox directory and verify the
    /// resolved path is still inside it. Returns the absolute resolved path.
    /// When `sandbox_dir` is None (open mode) the path is returned as-is.
    fn safe_path(&self, path_str: &str) -> Result<std::path::PathBuf, Signal> {
        let Some(ref base) = self.sandbox_dir else {
            return Ok(std::path::PathBuf::from(path_str));
        };
        let raw = std::path::Path::new(path_str);
        let resolved = if raw.is_absolute() {
            normalize_path_no_symlink(raw)
        } else {
            normalize_path_no_symlink(&base.join(raw))
        };
        if !resolved.starts_with(base) {
            return Err(Signal::Error(format!(
                "Access denied: '{}' is outside the allowed directory '{}'. \
                 Run with --no-sandbox to disable path restrictions.",
                path_str,
                base.display()
            )));
        }
        Ok(resolved)
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
        for t in &program.tools {
            self.tools.insert(t.name.clone(), t.clone());
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

            if let Some(ref alias) = fi.alias {
                // Namespaced import: register functions as `alias.funcname`
                // Also store the originals in module_functions so intra-module
                // calls (e.g. pad_right calling repeat_str) resolve correctly.
                let originals: Vec<FunctionDef> = sub.functions.clone();
                for f in &sub.functions {
                    let mut namespaced = f.clone();
                    namespaced.name = format!("{}.{}", alias, f.name);
                    self.functions.insert(namespaced.name.clone(), namespaced);
                }
                self.module_functions.insert(alias.clone(), originals);
                // Agents from the module are also callable via alias namespace
                for h in &sub.helpers {
                    self.helpers.insert(h.name.clone(), h.clone());
                }
            } else {
                // Flat import: inline everything into global scope
                for f in &sub.functions {
                    self.functions.insert(f.name.clone(), f.clone());
                }
                for h in &sub.helpers {
                    self.helpers.insert(h.name.clone(), h.clone());
                }
            }
            self.imports.extend(sub.imports.clone());
        }

        for h in &program.helpers {
            self.helpers.insert(h.name.clone(), h.clone());
            // Register agent-level function declarations globally
            for f in &h.functions {
                self.functions.insert(f.name.clone(), f.clone());
            }
        }

        // Execute top-level statements (x = 1, load_env(".env"), config = yaml_parse(...))
        // before any agent runs. Results are stored as global_vars and injected into
        // every agent's env so they are accessible as ordinary locals.
        if !program.top_level_stmts.is_empty() {
            let mut global_env = Env::new();
            for stmt in &program.top_level_stmts.clone() {
                self.run_stmt(stmt, &mut global_env).map_err(|e| match e {
                    Signal::Error(m) => format!("top-level statement: {}", m),
                    Signal::AssertFail(m) => format!("Assertion failed: {}", m),
                    Signal::Return(_) => "return outside function".into(),
                    other => format!("top-level error: {:?}", other),
                })?;
            }
            // Store everything that was assigned (excluding memory object itself)
            for (k, v) in &global_env.vars {
                if k != "memory" {
                    self.global_vars.insert(k.clone(), v.clone());
                }
            }
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
        // Reset the call stack at the top of each agent run, then push this agent's frame.
        // (An error aborts the whole program, so stale frames never leak across agents.)
        self.call_stack.clear();
        self.call_stack.push(format!("agent \"{}\"", helper.name));

        // Enable trace if the agent declared trace: true
        // (Currently set via helper.goal == "trace" as a convention; full attr support later)

        let mut memory: HashMap<String, Value> = HashMap::new();
        memory.insert("ai_trace".into(), Value::Array(Vec::new()));

        // v0.2.0: store goal in memory if declared
        if let Some(ref goal) = helper.goal {
            memory.insert("goal".into(), Value::Str(goal.clone()));
        }

        // Persistent memory: load from SQLite if the agent has any `persistent` entries
        #[cfg(not(target_arch = "wasm32"))]
        let persistent_db_path = {
            let has_persistent = self.has_persistent_memory(&helper.name);
            if has_persistent {
                let db_path = persistent_db_path_for(&helper.name);
                // Load existing memory from SQLite (overlay on top of default values)
                if let Ok(loaded) = load_persistent_memory(&db_path) {
                    for (k, v) in loaded {
                        memory.insert(k, v);
                    }
                }
                Some(db_path)
            } else {
                None
            }
        };

        let mut env = Env::new();
        for entry in &helper.memory {
            // Only set default if not already loaded from persistent store
            if !memory.contains_key(&entry.key)
                || matches!(memory.get(&entry.key), Some(Value::Null))
            {
                let val = self.eval_expr(&entry.value, &mut env)?;
                memory.insert(entry.key.clone(), val);
            }
        }

        // Inject file-root globals into the agent's env as ordinary locals.
        // This makes `url = "https://..."` at file root accessible inside agents.
        let global_vars = self.global_vars.clone();
        for (k, v) in global_vars {
            env.set(&k, v);
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

        // Persist memory to SQLite if this agent uses persistent storage
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref db_path) = persistent_db_path {
            let final_memory = env.get_memory();
            let _ = save_persistent_memory(db_path, &final_memory);
        }

        Ok(())
    }

    /// Returns true if the agent has any persistent memory entries (checked by name convention).
    #[allow(unused_variables)]
    fn has_persistent_memory(&self, agent_name: &str) -> bool {
        // Persistent memory is opt-in: set `gx_persistent = true` in memory block
        // or detected by looking at agent's memory entries. For now, we check if a
        // special file exists at the expected path.
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::path::Path::new(&persistent_db_path_for(agent_name)).exists()
        }
        #[cfg(target_arch = "wasm32")]
        false
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

    /// Wrapper that attaches source-line and call-stack context to runtime errors.
    /// Only the innermost statement attaches the line (outer frames see " at line "
    /// already present and pass the error through unchanged).
    fn run_stmt(&mut self, stmt: &Stmt, env: &mut Env) -> IResult {
        match self.run_stmt_inner(stmt, env) {
            Err(Signal::Error(m)) if !m.contains(" at line ") => {
                let mut full = format!("{} at line {}", m, stmt_line(stmt));
                if !self.call_stack.is_empty() {
                    full.push_str(&format!("\n  in {}", self.call_stack.join(" → ")));
                }
                Err(Signal::Error(full))
            }
            other => other,
        }
    }

    fn run_stmt_inner(&mut self, stmt: &Stmt, env: &mut Env) -> IResult {
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
                // Default cap: 10 million iterations — enough for any real program,
                // prevents accidental infinite spin. Disabled by --no-limit.
                // Note: wall-clock timeout is intentionally removed — blocking I/O
                // (readline, http_stream) would trigger it incorrectly.
                const MAX_WHILE: usize = 10_000_000;
                let limit = if self.no_loop_limit {
                    usize::MAX
                } else {
                    MAX_WHILE
                };
                loop {
                    if iterations >= limit {
                        return Err(Signal::Error(
                            "while loop exceeded 10,000,000 iterations. \
                             If this is an intentional infinite loop (e.g. a REPL using readline()), \
                             run with: gx run --no-limit"
                                .into(),
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
                let loop_limit = if self.no_loop_limit {
                    usize::MAX
                } else {
                    10_000_000
                };
                loop {
                    if iterations > loop_limit {
                        return Err(Signal::Error(
                            "loop until exceeded iteration limit. Use --no-limit if intentional."
                                .into(),
                        ));
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

            // await { key: expr, ... } — concurrent parallel execution
            Stmt::Await {
                bindings, into_var, ..
            } => {
                let result = self.eval_await_block(bindings, env)?;
                env.set(into_var, result.clone());
                Ok(result)
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
                // Range slice: expr[start..end]
                if let Expr::Range { start, end } = index.as_ref() {
                    let obj = self.eval_expr(object, env)?;
                    let s = self.eval_expr(start, env)?.as_number().unwrap_or(0.0) as usize;
                    let e = self.eval_expr(end, env)?.as_number().unwrap_or(0.0) as usize;
                    return match obj {
                        Value::Str(ref st) => {
                            let chars: Vec<char> = st.chars().collect();
                            let end = e.min(chars.len());
                            let start = s.min(end);
                            Ok(Value::Str(chars[start..end].iter().collect()))
                        }
                        Value::Array(ref arr) => {
                            let end = e.min(arr.len());
                            let start = s.min(end);
                            Ok(Value::Array(arr[start..end].to_vec()))
                        }
                        other => Err(Signal::Error(format!(
                            "Range slicing requires a string or array, got {}",
                            other.type_name()
                        ))),
                    };
                }
                let obj = self.eval_expr(object, env)?;
                let idx = self.eval_expr(index, env)?;
                Ok(obj.get_index(&idx))
            }

            Expr::Range { start, end } => {
                // Standalone range expression evaluates to an array (like range(start, end))
                let s = self.eval_expr(start, env)?.as_number().unwrap_or(0.0) as i64;
                let e = self.eval_expr(end, env)?.as_number().unwrap_or(0.0) as i64;
                let arr: Vec<Value> = (s..e).map(|n| Value::Number(n as f64)).collect();
                Ok(Value::Array(arr))
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

            Expr::Lambda { params, body } => {
                // Capture a snapshot of the current local scope so the closure can
                // reference enclosing variables (url, body, headers, etc.) at call time.
                // memory.* is propagated separately; only plain locals are captured here.
                let captured = env.vars.clone();
                Ok(Value::Closure(params.clone(), body.clone(), captured))
            }

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
                        Err(Signal::Error(format!(
                            "Division by zero ({} / 0). Guard with: if divisor != 0 {{ ... }}",
                            a
                        )))
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
                        Err(Signal::Error(format!(
                            "Modulo by zero ({} % 0). Guard with: if divisor != 0 {{ ... }}",
                            a
                        )))
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
            // Module call: `utils.greet(args)` where utils is an imported namespace.
            // Check self.functions for a "namespace.funcname" entry before falling
            // through to the generic method-call path.
            if let Expr::Ident(namespace) = object.as_ref() {
                let full_name = format!("{}.{}", namespace, field);
                if let Some(func) = self.functions.get(&full_name).cloned() {
                    let mem = env.get_memory();
                    // Temporarily inject this module's functions under their short
                    // names so intra-module calls (e.g. pad_right → repeat_str)
                    // resolve correctly without requiring the namespace prefix.
                    let siblings: Vec<FunctionDef> = self
                        .module_functions
                        .get(namespace)
                        .cloned()
                        .unwrap_or_default();
                    let mut restored: Vec<(String, Option<FunctionDef>)> = Vec::new();
                    for sf in &siblings {
                        let prev = self.functions.insert(sf.name.clone(), sf.clone());
                        restored.push((sf.name.clone(), prev));
                    }
                    let result = self.call_user_function(&func, args, Some(mem));
                    for (name, prev) in restored {
                        match prev {
                            Some(f) => {
                                self.functions.insert(name, f);
                            }
                            None => {
                                self.functions.remove(&name);
                            }
                        }
                    }
                    return result;
                }
            }
            let obj = self.eval_expr(object, env)?;
            // If the field holds a closure, call it directly instead of treating
            // it as a built-in method. Enables: tools[i].handler(args), dispatch["post"](req)
            if let Value::Object(ref map) = obj {
                if let Some(Value::Closure(params, body, captured)) = map.get(field.as_str()) {
                    let (p, b, c) = (params.clone(), body.clone(), captured.clone());
                    return self.call_closure_with_capture(&p, &b, &c, args, env);
                }
            }
            return self.eval_method(obj, field, args, env);
        }

        // Direct lambda call: fn(x) { x + 1 }(5)
        // Capture current scope so the body can reference enclosing locals.
        if let Expr::Lambda { params, body } = callee {
            let captured = env.vars.clone();
            return self.call_closure_with_capture(params, body, &captured, args, env);
        }

        if let Expr::Ident(name) = callee {
            // Check if a closure value is stored under this name
            let val = env.get(name);
            if let Value::Closure(params, body, captured) = val {
                return self.call_closure_with_capture(&params, &body, &captured, args, env);
            }
            // Also check memory
            let mem = env.get_memory();
            if let Some(Value::Closure(params, body, captured)) = mem.get(name).cloned() {
                return self.call_closure_with_capture(&params, &body, &captured, args, env);
            }
            if let Some(func) = self.functions.get(name).cloned() {
                // Named functions (file-root, agent-level) also propagate memory
                return self.call_user_function_propagating(&func, args, env);
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

    // ── await { key: expr, ... } — concurrent IO block ────────────────────────

    fn eval_await_block(&mut self, bindings: &[(String, Expr)], env: &mut Env) -> IResult {
        use std::sync::mpsc;

        let mut handles: Vec<(String, mpsc::Receiver<Result<serde_json::Value, String>>)> =
            Vec::new();

        for (key, expr) in bindings {
            let val = self.eval_expr(expr, env)?;
            let json = gx_value_to_json(&val);
            // For HTTP calls and other IO we've already executed them synchronously above.
            // For agent calls, spawn threads like parallel {}.
            // This gives true concurrency for agent calls; for HTTP builtins the eval above
            // is already non-blocking from GX's perspective.
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(Ok(json));
            handles.push((key.clone(), rx));
        }

        let mut result_map = HashMap::new();
        for (key, rx) in handles {
            let entry = match rx.recv() {
                Ok(Ok(json)) => json_to_gx_value(&json),
                Ok(Err(e)) => {
                    let mut em = HashMap::new();
                    em.insert("error".into(), Value::Str(e));
                    Value::Object(em)
                }
                Err(_) => Value::Null,
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
            Value::Closure(params, body, captured) => {
                self.call_closure_with_capture(params, body, captured, args, env)
            }
            Value::Str(name) => {
                let name = name.clone();
                if let Some(func) = self.functions.get(&name).cloned() {
                    return self.call_user_function_propagating(&func, args, env);
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

    /// Core closure execution: seeds env with captured locals, overlays memory,
    /// then binds params. Changes to memory.* propagate back to the caller.
    fn call_closure_with_capture(
        &mut self,
        params: &[String],
        body: &[Stmt],
        captured: &HashMap<String, Value>,
        args: Vec<Value>,
        caller_env: &mut Env,
    ) -> IResult {
        let mut env = Env::new();
        // 1. Seed with captured locals (variables from the enclosing scope at definition time)
        for (k, v) in captured {
            env.set(k, v.clone());
        }
        // 2. Copy current memory into the closure env so memory.* reads work
        let initial_mem = caller_env.get_memory();
        env.set_memory(initial_mem.clone());
        // 3. Bind params — these shadow any captured variable with the same name
        for (i, param) in params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = body.to_vec();
        self.call_stack.push("<closure>".to_string());
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        let result = match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => return Err(e),
        };
        // 4. Propagate memory changes back to the caller
        let new_mem = env.get_memory();
        if new_mem != initial_mem {
            caller_env.set_memory(new_mem);
        }
        result
    }

    // ── Observability trace emit ──────────────────────────────────────────────

    pub fn emit_trace(&self, event: &str, data: &Value) {
        if !self.trace_enabled {
            return;
        }
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let agent = self.current_agent.as_deref().unwrap_or("unknown");
        let payload = serde_json::json!({
            "ts": ts,
            "agent": agent,
            "event": event,
            "data": gx_value_to_json(data)
        });
        eprintln!("[trace] {}", payload);
    }

    // ── Retry with exponential/linear/fixed backoff ───────────────────────────

    fn builtin_retry(&mut self, args: Vec<Value>, env: &mut Env) -> IResult {
        // retry(fn, max?, { delay?, backoff? })
        // backoff: "exponential" | "linear" | "fixed"
        let callable = args.first().cloned().unwrap_or(Value::Null);
        let max_attempts = args.get(1).and_then(|v| v.as_number()).unwrap_or(3.0) as u32;
        let opts = args.get(2).cloned().unwrap_or(Value::Null);

        let initial_delay_ms = match &opts {
            Value::Object(m) => m.get("delay").and_then(|v| v.as_number()).unwrap_or(1000.0) as u64,
            Value::Number(n) => *n as u64,
            _ => 1000,
        };
        let backoff_strategy = match &opts {
            Value::Object(m) => m
                .get("backoff")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "exponential".to_string()),
            _ => "exponential".to_string(),
        };

        // Keep the full Closure (with captured env) so retry lambdas can reference
        // enclosing variables like url, headers, body without workarounds.
        let closure = match callable {
            Value::Closure(..) => callable,
            _ => {
                return Err(Signal::Error(
                    "retry: first argument must be a function (fn() { ... })".into(),
                ))
            }
        };

        let mut last_error = Signal::Error("retry: no attempts made".into());

        for attempt in 0..max_attempts {
            match self.call_closure(&closure, vec![], env) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_error = e;
                    if attempt + 1 < max_attempts {
                        let delay = match backoff_strategy.as_str() {
                            "exponential" => initial_delay_ms * (2u64.pow(attempt)),
                            "linear" => initial_delay_ms * (attempt as u64 + 1),
                            _ => initial_delay_ms, // fixed
                        };
                        let delay = delay.min(30_000); // cap at 30 seconds
                        eprintln!(
                            "[gx] retry: attempt {}/{} failed, waiting {}ms",
                            attempt + 1,
                            max_attempts,
                            delay
                        );
                        #[cfg(not(target_arch = "wasm32"))]
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                    }
                }
            }
        }
        Err(last_error)
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
        self.call_stack.push(format!("{}()", func.name));
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Like call_user_function but propagates memory changes back to the caller's env.
    /// Used when agent-level or inline functions need to mutate shared `memory.*` state.
    fn call_user_function_propagating(
        &mut self,
        func: &FunctionDef,
        args: Vec<Value>,
        caller_env: &mut Env,
    ) -> IResult {
        let initial_mem = caller_env.get_memory();
        let mut env = Env::new();
        env.set_memory(initial_mem.clone());
        for (i, param) in func.params.iter().enumerate() {
            env.set(param, args.get(i).cloned().unwrap_or(Value::Null));
        }
        let body = func.body.clone();
        self.call_stack.push(format!("{}()", func.name));
        let raw = self.run_stmts(&body, &mut env);
        self.call_stack.pop();
        let result = match raw {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => return Err(e),
        };
        // Propagate any memory changes back to the caller
        let new_mem = env.get_memory();
        if new_mem != initial_mem {
            caller_env.set_memory(new_mem);
        }
        result
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

            // ── Stdin ─────────────────────────────────────────────────────────
            "is_tty" | "stdin_is_tty" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::IsTerminal;
                    Ok(Value::Bool(std::io::stdin().is_terminal()))
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Bool(false))
            }
            "readline" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::BufRead;
                    let mut line = String::new();
                    match std::io::stdin().lock().read_line(&mut line) {
                        Ok(0) => Ok(Value::Null), // EOF
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                                if line.ends_with('\r') {
                                    line.pop();
                                }
                            }
                            Ok(Value::Str(line))
                        }
                        Err(e) => Err(Signal::Error(format!("readline: {}", e))),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Null)
            }
            "read_all" | "read_stdin" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    use std::io::{IsTerminal, Read};
                    // Never block waiting for user to type — return null on TTY.
                    // Callers that need TTY input should use readline() in a loop.
                    if std::io::stdin().is_terminal() {
                        return Ok(Value::Null);
                    }
                    let mut buf = String::new();
                    std::io::stdin()
                        .lock()
                        .read_to_string(&mut buf)
                        .map(|_| Value::Str(buf))
                        .map_err(|e| Signal::Error(format!("read_stdin: {}", e)))
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Value::Null)
            }

            // ── Standalone slice / merge (also available as methods) ──────────
            "slice" => {
                let target = args.first().cloned().unwrap_or(Value::Null);
                let start = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                match target {
                    Value::Array(arr) => {
                        let end = args
                            .get(2)
                            .and_then(|v| v.as_number())
                            .map(|n| (n as usize).min(arr.len()))
                            .unwrap_or(arr.len());
                        let start = start.min(end);
                        Ok(Value::Array(arr[start..end].to_vec()))
                    }
                    Value::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let end = args
                            .get(2)
                            .and_then(|v| v.as_number())
                            .map(|n| (n as usize).min(chars.len()))
                            .unwrap_or(chars.len());
                        let start = start.min(end);
                        Ok(Value::Str(chars[start..end].iter().collect()))
                    }
                    other => Err(Signal::Error(format!(
                        "slice() expects a string or array as first argument, got {}",
                        other.type_name()
                    ))),
                }
            }
            "merge" => {
                // merge(obj1, obj2, ...) → shallow-merge all objects left to right
                let mut result = HashMap::new();
                for arg in &args {
                    if let Value::Object(m) = arg {
                        for (k, v) in m {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                }
                Ok(Value::Object(result))
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

            // ── Date / Time (production) ──────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "date_now" => date_now_impl(),
            #[cfg(not(target_arch = "wasm32"))]
            "date_timestamp" => date_timestamp_impl(),
            #[cfg(not(target_arch = "wasm32"))]
            "date_parse" => date_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_format" => date_format_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_diff" => date_diff_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_add" => date_add_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_parts" => date_parts_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "date_from_parts" => date_from_parts_impl(&args),

            // ── Regex ─────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "regex_test" | "re_test" => regex_test_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_find" | "re_find" => regex_find_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_find_all" | "re_find_all" | "regex_findall" => regex_find_all_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_replace" | "re_replace" => regex_replace_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_split" | "re_split" => regex_split_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_captures" | "re_captures" => regex_captures_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "regex_named_captures" | "re_named" => regex_named_captures_impl(&args),

            // ── CSV / YAML / TOML ─────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "csv_parse" => csv_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "csv_stringify" | "csv_encode" => csv_stringify_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "yaml_parse" => yaml_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "yaml_stringify" | "yaml_encode" => yaml_stringify_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "toml_parse" => toml_parse_impl(&args),
            #[cfg(not(target_arch = "wasm32"))]
            "toml_stringify" | "toml_encode" => toml_stringify_impl(&args),

            // ── Environment / .env ────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "load_env" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| ".env".to_string());
                load_env_file(&path)
            }
            // get_env with optional default
            "get_env" | "env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                match std::env::var(&key) {
                    Ok(v) => Ok(Value::Str(v)),
                    Err(_) => Ok(default),
                }
            }
            "set_env" => {
                let key = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let val = args.get(1).map(|v| v.to_string()).unwrap_or_default();
                std::env::set_var(&key, &val);
                Ok(Value::Null)
            }

            // ── Retry with backoff ────────────────────────────────────────────
            "retry" => self.builtin_retry(args, env),

            // ── Vector store ──────────────────────────────────────────────────
            "vector_store_new" | "vs_new" => vector_store_new_impl(&args),
            "vector_store_add" | "vs_add" => vector_store_add_impl(&args),
            "vector_store_search" | "vs_search" => vector_store_search_impl(&args),
            "vector_store_delete" | "vs_delete" => vector_store_delete_impl(&args),
            "vector_store_size" | "vs_size" => vector_store_size_impl(&args),
            "cosine_similarity" => cosine_similarity_impl(&args),

            // ── Observability ─────────────────────────────────────────────────
            "trace_log" => {
                let event = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "event".to_string());
                let data = args.get(1).cloned().unwrap_or(Value::Null);
                self.emit_trace(&event, &data);
                Ok(Value::Null)
            }

            // ── Test assertions (integrate with `gx test` pass/fail tracking) ──
            "assert_eq" | "assert_equal" => {
                self.assert_count += 1;
                let a = args.first().cloned().unwrap_or(Value::Null);
                let b = args.get(1).cloned().unwrap_or(Value::Null);
                if a == b {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(2)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_eq".to_string());
                    let msg = format!("{}: expected {} == {}", label, a, b);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }
            "assert_true" | "assert_that" => {
                self.assert_count += 1;
                let cond = args.first().map(|v| v.is_truthy()).unwrap_or(false);
                if cond {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(1)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_true".to_string());
                    let msg = format!("{}: expected true, got false", label);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }
            "assert_contains" => {
                self.assert_count += 1;
                let haystack = args.first().cloned().unwrap_or(Value::Null);
                let needle = args.get(1).cloned().unwrap_or(Value::Null);
                let contained = match &haystack {
                    Value::Str(s) => needle.as_str().map(|n| s.contains(n)).unwrap_or(false),
                    Value::Array(arr) => arr.contains(&needle),
                    _ => false,
                };
                if contained {
                    Ok(Value::Bool(true))
                } else {
                    let label = args
                        .get(2)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "assert_contains".to_string());
                    let msg = format!("{}: {} does not contain {}", label, haystack, needle);
                    self.assert_failures.push(msg.clone());
                    Err(Signal::AssertFail(msg))
                }
            }

            // ── Schema validation ─────────────────────────────────────────────
            "schema_validate" => schema_validate_impl(&args),
            "schema_check" => schema_validate_impl(&args),

            // ── Persistent memory ─────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "persist_memory" | "save_memory" => self.builtin_persist_memory(env),
            #[cfg(not(target_arch = "wasm32"))]
            "load_memory" | "restore_memory" => self.builtin_load_memory(env),

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
                // max(array) or max(a, b, c, ...) — handles any number of args
                if let Some(Value::Array(arr)) = args.first() {
                    let m = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::NEG_INFINITY, f64::max);
                    return Ok(Value::Number(m));
                }
                let m = args
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Value::Number(m))
            }
            "min" => {
                // min(array) or min(a, b, c, ...) — handles any number of args
                if let Some(Value::Array(arr)) = args.first() {
                    let m = arr
                        .iter()
                        .filter_map(|v| v.as_number())
                        .fold(f64::INFINITY, f64::min);
                    return Ok(Value::Number(m));
                }
                let m = args
                    .iter()
                    .filter_map(|v| v.as_number())
                    .fold(f64::INFINITY, f64::min);
                Ok(Value::Number(m))
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
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(url) = args.first().and_then(|v| v.as_str()) {
                    check_url_safe(url, self.allow_internal_http)?;
                }
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
                let headers: Vec<(String, String)> = match &opts {
                    Value::Object(m) => match m.get("headers") {
                        Some(Value::Object(hm)) => hm
                            .iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                #[cfg(not(target_arch = "wasm32"))]
                check_url_safe(&url, self.allow_internal_http)?;
                let method_name = match method.to_uppercase().as_str() {
                    "POST" => "http_post",
                    "PUT" => "http_put",
                    "DELETE" => "http_delete",
                    _ => "http_get",
                };
                let headers_val = if headers.is_empty() {
                    None
                } else {
                    Some(Value::Object(
                        headers
                            .into_iter()
                            .map(|(k, v)| (k, Value::Str(v)))
                            .collect::<HashMap<_, _>>(),
                    ))
                };
                // arg layout: [url, body?, headers?]
                // For GET/DELETE: [url, headers?]
                // For POST/PUT:   [url, body, headers?]
                let mut builtin_args = vec![Value::Str(url)];
                match method_name {
                    "http_post" | "http_put" => {
                        builtin_args.push(body_val);
                        if let Some(h) = headers_val {
                            builtin_args.push(h);
                        }
                    }
                    _ => {
                        if let Some(h) = headers_val {
                            builtin_args.push(h);
                        }
                    }
                }
                http_builtin(method_name, &builtin_args)
            }

            // ── #17 HTTP streaming ────────────────────────────────────────────
            "http_stream" => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let url = match args.first() {
                        Some(Value::Object(m)) => m
                            .get("url")
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default(),
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    if !url.is_empty() {
                        check_url_safe(&url, self.allow_internal_http)?;
                    }
                }
                http_stream_builtin(&args)
            }

            // ── #8 HTTP multipart upload ──────────────────────────────────────
            "http_upload" => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(Value::Object(m)) = args.first() {
                    if let Some(url) = m.get("url").and_then(|v| v.as_str()) {
                        check_url_safe(url, self.allow_internal_http)?;
                    }
                }
                http_upload_builtin(&args)
            }

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
                    let mut w = std::io::BufWriter::new(stream.try_clone().map_err(|e| {
                        Signal::Error(format!("send_email: failed to clone socket: {}", e))
                    })?);
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
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("read_file requires a path string".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::read_to_string(&path)
                    .map(Value::Str)
                    .map_err(|e| Signal::Error(format!("read_file '{}': {}", raw, e)))
            }
            "read_file_lines" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| {
                        Signal::Error("read_file_lines requires a path string".into())
                    })?;
                let path = self.safe_path(&raw)?;
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Signal::Error(format!("read_file_lines '{}': {}", raw, e)))?;
                Ok(Value::Array(
                    content.lines().map(|l| Value::Str(l.to_string())).collect(),
                ))
            }
            "write_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("write_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                let content = args
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()))
                    .to_string();
                std::fs::write(&path, &content)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("write_file '{}': {}", raw, e)))
            }
            "append_file" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("append_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
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
                    .map_err(|e| Signal::Error(format!("append_file '{}': {}", raw, e)))?;
                f.write_all(content.as_bytes())
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("append_file write '{}': {}", raw, e)))
            }
            "file_exists" | "exists" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let path = self.safe_path(&raw)?;
                Ok(Value::Bool(path.exists()))
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
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("delete_file requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::remove_file(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("delete_file '{}': {}", raw, e)))
            }
            "list_dir" | "read_dir" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| ".".into());
                let path = self.safe_path(&raw)?;
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let files: Vec<Value> = entries
                            .flatten()
                            .map(|e| Value::Str(e.file_name().to_string_lossy().into_owned()))
                            .collect();
                        Ok(Value::Array(files))
                    }
                    Err(e) => Err(Signal::Error(format!("list_dir '{}': {}", raw, e))),
                }
            }
            "make_dir" | "mkdir" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("make_dir requires a path".into()))?;
                let path = self.safe_path(&raw)?;
                std::fs::create_dir_all(&path)
                    .map(|_| Value::Null)
                    .map_err(|e| Signal::Error(format!("make_dir '{}': {}", raw, e)))
            }
            "path_join" => {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                let mut path = std::path::PathBuf::new();
                for p in parts {
                    path.push(p);
                }
                Ok(Value::Str(path.to_string_lossy().into_owned()))
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
                if !self.allow_shell {
                    return Err(Signal::Error(
                        "shell() is disabled by default. \
                         Run with --allow-shell to enable OS command execution."
                            .into(),
                    ));
                }
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

            // ── Agent management ──────────────────────────────────────────────
            "spawn_agent" | "spawn_helper" => {
                let n = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "unknown".into());
                Err(Signal::Error(format!(
                    "spawn_agent('{}') is not yet implemented. \
                     Agents currently run sequentially in one process. \
                     Use function calls or direct helper invocation for sub-tasks.",
                    n
                )))
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
            // ── SQLite ────────────────────────────────────────────────────────
            #[cfg(not(target_arch = "wasm32"))]
            "db_query" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected db path".into()))?;
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
                let sql = args
                    .get(1)
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_query: expected SQL string".into()))?;
                let params: Vec<Value> = args.into_iter().skip(2).collect();
                db_query_impl(&path, &sql, params)
            }
            #[cfg(not(target_arch = "wasm32"))]
            "db_exec" => {
                let raw = args
                    .first()
                    .and_then(|v| v.as_str().map(String::from))
                    .ok_or_else(|| Signal::Error("db_exec: expected db path".into()))?;
                let safe = self.safe_path(&raw)?;
                let path = safe.to_string_lossy().into_owned();
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
                let suggestion = closest_builtin(name);
                let hint = match suggestion {
                    Some(s) => format!(" — did you mean '{}'?", s),
                    None => " — returning null".to_string(),
                };
                eprintln!("[gx] warning: unknown function '{}'{}", name, hint);
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
            (Value::Str(s), "reverse") => Ok(Value::Str(s.chars().rev().collect())),
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

// ── Free helper functions ─────────────────────────────────────────────────────

/// Extract the source line number from any statement (for error context).
fn stmt_line(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Assign { line, .. }
        | Stmt::PlusAssign { line, .. }
        | Stmt::MinusAssign { line, .. }
        | Stmt::MulAssign { line, .. }
        | Stmt::DivAssign { line, .. }
        | Stmt::If { line, .. }
        | Stmt::ForEach { line, .. }
        | Stmt::While { line, .. }
        | Stmt::Break { line }
        | Stmt::Continue { line }
        | Stmt::TryCatch { line, .. }
        | Stmt::Assert { line, .. }
        | Stmt::Emit { line, .. }
        | Stmt::Broadcast { line, .. }
        | Stmt::Log { line, .. }
        | Stmt::Output { line, .. }
        | Stmt::Say { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::Expr { line, .. }
        | Stmt::Wait { line, .. }
        | Stmt::ReRun { line }
        | Stmt::EscalateToHuman { line }
        | Stmt::Serve { line, .. }
        | Stmt::SendMessage { line, .. }
        | Stmt::Think { line, .. }
        | Stmt::Observe { line, .. }
        | Stmt::Act { line, .. }
        | Stmt::LoopUntil { line, .. }
        | Stmt::RepeatTimes { line, .. }
        | Stmt::Parallel { line, .. }
        | Stmt::Respond { line, .. }
        | Stmt::Await { line, .. } => *line,
    }
}

/// Common builtins for "did you mean" suggestions on unknown function calls.
const KNOWN_BUILTINS: &[&str] = &[
    "log",
    "print",
    "say",
    "readline",
    "read_all",
    "is_tty",
    "slice",
    "merge",
    "len",
    "range",
    "to_string",
    "to_number",
    "type_of",
    "is_null",
    "abs",
    "floor",
    "ceil",
    "round",
    "sqrt",
    "pow",
    "min",
    "max",
    "random",
    "json_stringify",
    "json_parse",
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "read_file",
    "write_file",
    "delete_file",
    "file_exists",
    "list_dir",
    "make_dir",
    "regex_test",
    "regex_find",
    "regex_find_all",
    "regex_replace",
    "regex_split",
    "regex_captures",
    "date_now",
    "date_parse",
    "date_format",
    "date_diff",
    "date_add",
    "date_parts",
    "csv_parse",
    "csv_stringify",
    "yaml_parse",
    "yaml_stringify",
    "toml_parse",
    "toml_stringify",
    "load_env",
    "get_env",
    "set_env",
    "retry",
    "vector_store_new",
    "vector_store_add",
    "vector_store_search",
    "cosine_similarity",
    "schema_validate",
    "persist_memory",
    "load_memory",
    "trace_log",
    "base64_encode",
    "base64_decode",
    "embed",
    "sleep",
    "now",
    "now_ms",
    "get_timestamp",
];

/// Returns the closest known builtin within edit distance 2, if any.
fn closest_builtin(name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for &candidate in KNOWN_BUILTINS {
        let d = levenshtein(name, candidate);
        if d <= 2 && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((candidate, d));
        }
    }
    best.map(|(c, _)| c)
}

/// Standard Levenshtein edit distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Parse and load a .env file into the current process environment.
/// Lines of the form KEY=VALUE or KEY="VALUE" are supported.
/// Lines starting with # are comments. Existing env vars are NOT overwritten.
#[cfg(not(target_arch = "wasm32"))]
fn load_env_file(path: &str) -> Result<Value, Signal> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Signal::Error(format!("load_env: cannot read '{}': {}", path, e)))?;
    let mut loaded = 0usize;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let raw_val = line[eq + 1..].trim();
            let val = raw_val.trim_matches('"').trim_matches('\'').to_string();
            // Only set if not already present
            if std::env::var(&key).is_err() {
                std::env::set_var(&key, &val);
            }
            loaded += 1;
        }
    }
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("loaded".into(), Value::Number(loaded as f64));
    result.insert("path".into(), Value::Str(path.to_string()));
    Ok(Value::Object(result))
}

/// schema_validate(value, schema_object) → { ok: bool, errors: array<string> }
/// Schema object format: { field_name: "type" } or { field_name: { type: "string", required: true } }
fn schema_validate_impl(args: &[Value]) -> Result<Value, Signal> {
    let value = args.first().cloned().unwrap_or(Value::Null);
    let schema = match args.get(1).cloned().unwrap_or(Value::Null) {
        Value::Object(m) => m,
        _ => {
            return Err(Signal::Error(
                "schema_validate(value, schema) — schema must be an object".into(),
            ))
        }
    };

    let obj = match &value {
        Value::Object(m) => m.clone(),
        _ => {
            let mut r = HashMap::new();
            r.insert("ok".into(), Value::Bool(false));
            r.insert(
                "errors".into(),
                Value::Array(vec![Value::Str("value must be an object".into())]),
            );
            return Ok(Value::Object(r));
        }
    };

    let mut errors: Vec<Value> = Vec::new();

    for (field, rule) in &schema {
        let (expected_type, required) = match rule {
            Value::Str(t) => (t.as_str().to_string(), true),
            Value::Object(opts) => {
                let t = opts
                    .get("type")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "any".to_string());
                let req = opts.get("required").map(|v| v.is_truthy()).unwrap_or(true);
                (t, req)
            }
            _ => continue,
        };

        match obj.get(field) {
            None | Some(Value::Null) => {
                if required {
                    errors.push(Value::Str(format!("field '{}' is required", field)));
                }
            }
            Some(field_val) => {
                let actual = field_val.type_name();
                let ok = match expected_type.as_str() {
                    "any" => true,
                    "string" | "str" => matches!(field_val, Value::Str(_)),
                    "number" | "num" | "float" | "int" => {
                        matches!(field_val, Value::Number(_))
                    }
                    "boolean" | "bool" => matches!(field_val, Value::Bool(_)),
                    "array" | "list" => matches!(field_val, Value::Array(_)),
                    "object" | "map" => matches!(field_val, Value::Object(_)),
                    "null" => matches!(field_val, Value::Null),
                    _ => true,
                };
                if !ok {
                    errors.push(Value::Str(format!(
                        "field '{}' must be {}, got {}",
                        field, expected_type, actual
                    )));
                }
            }
        }
    }

    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(errors.is_empty()));
    result.insert("errors".into(), Value::Array(errors));
    Ok(Value::Object(result))
}

// ── Persistent memory (SQLite-backed) ────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn persistent_db_path_for(agent_name: &str) -> String {
    let safe_name: String = agent_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = std::env::var("GX_STATE_DIR")
        .unwrap_or_else(|_| dirs_home().unwrap_or_else(|| ".".to_string()) + "/.gx/state");
    format!("{}/{}.db", base, safe_name)
}

#[cfg(not(target_arch = "wasm32"))]
fn dirs_home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn load_persistent_memory(db_path: &str) -> Result<HashMap<String, Value>, String> {
    use rusqlite::Connection;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT key, value FROM memory")
        .map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    for row in rows.flatten() {
        let (k, v_json) = row;
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&v_json) {
            map.insert(k, json_to_gx_value(&json));
        }
    }
    Ok(map)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_persistent_memory(db_path: &str, memory: &HashMap<String, Value>) -> Result<(), String> {
    use rusqlite::Connection;
    // Ensure the parent directory exists
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    // Skip ai_trace (ephemeral) and closures
    for (k, v) in memory {
        if k == "ai_trace" {
            continue;
        }
        if matches!(v, Value::Closure(..)) {
            continue;
        }
        let json_str = serde_json::to_string(&gx_value_to_json(v)).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO memory (key, value) VALUES (?1, ?2)",
            rusqlite::params![k, json_str],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── builtin: persist_memory / load_memory ────────────────────────────────────

impl Interpreter {
    /// Force-save current memory to SQLite for the current agent.
    /// Called by the `persist_memory()` builtin.
    #[cfg(not(target_arch = "wasm32"))]
    fn builtin_persist_memory(&self, env: &Env) -> IResult {
        let agent = self.current_agent.as_deref().unwrap_or("default");
        let db_path = persistent_db_path_for(agent);
        let memory = env.get_memory();
        save_persistent_memory(&db_path, &memory)
            .map(|_| Value::Bool(true))
            .map_err(|e| Signal::Error(format!("persist_memory: {}", e)))
    }

    /// Load memory from SQLite into the current env. Called by `load_memory()` builtin.
    #[cfg(not(target_arch = "wasm32"))]
    fn builtin_load_memory(&self, env: &mut Env) -> IResult {
        let agent = self.current_agent.as_deref().unwrap_or("default");
        let db_path = persistent_db_path_for(agent);
        let loaded = load_persistent_memory(&db_path)
            .map_err(|e| Signal::Error(format!("load_memory: {}", e)))?;
        let count = loaded.len();
        let mut mem = env.get_memory();
        for (k, v) in loaded {
            mem.insert(k, v);
        }
        env.set_memory(mem);
        Ok(Value::Number(count as f64))
    }
}

// ── Bridge + serve: see bridge_impl.rs ────────────────────────────────────────
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

    // ── v0.4.0 feature tests ──────────────────────────────────────────────────

    #[test]
    fn test_bang_not_operator() {
        run(r#"
helper "bang" {
  brain {
    plan { }
    execute {
      assert !false "!false should be true"
      assert !(!true) "double negation"
      ok = false
      assert !ok "!variable"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_integer_json_serialization() {
        run(r#"
helper "intjson" {
  brain {
    plan { }
    execute {
      s = json_stringify({ n: 50, rate: 0.5, count: 100 })
      // Must NOT contain 50.0 or 100.0 for integer values
      assert not s.contains("50.0") "integer must not be float"
      assert not s.contains("100.0") "integer must not be float"
      // Float must remain
      assert s.contains("0.5") "float preserved"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_range_slicing_string() {
        run(r#"
helper "rngstr" {
  brain {
    plan { }
    execute {
      s = "hello world"
      assert s[0..5] == "hello" "string range [0..5]"
      assert s[6..11] == "world" "string range [6..11]"
      assert s[0..0] == "" "empty range"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_range_slicing_array() {
        run(r#"
helper "rngarr" {
  brain {
    plan { }
    execute {
      arr = [10, 20, 30, 40, 50]
      sub = arr[1..4]
      assert sub[0] == 20 "range arr[0]"
      assert sub[1] == 30 "range arr[1]"
      assert sub[2] == 40 "range arr[2]"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_output_as_variable() {
        run(r#"
helper "outvar" {
  brain {
    plan { }
    execute {
      output = "hello"
      output += " world"
      assert output == "hello world" "output as variable"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_standalone_slice() {
        run(r#"
helper "slicefn" {
  brain {
    plan { }
    execute {
      assert slice("foobar", 0, 3) == "foo" "slice string"
      sub = slice([1, 2, 3, 4, 5], 1, 4)
      assert sub[0] == 2 "slice array start"
      assert sub[2] == 4 "slice array end"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_standalone_merge() {
        run(r#"
helper "mergefn" {
  brain {
    plan { }
    execute {
      m = merge({ a: 1, b: 2 }, { b: 99, c: 3 })
      assert m.a == 1 "merge preserves left"
      assert m.b == 99 "merge right wins"
      assert m.c == 3 "merge adds right"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_interpolation_brace_escape() {
        run(r#"
helper "interpescape" {
  brain {
    plan { }
    execute {
      // {{ in a GX string produces a literal {
      // Both the value and the comparison use {{ so both resolve to "{literal}"
      s = "{{literal}}"
      assert s == "{{literal}}" "brace escape"
      // Mixing: {{ escape + real interpolation
      name = "GX"
      msg = "{{tag}}: {name}"
      assert msg == "{{tag}}: GX" "mixed escape and interp"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_inline_function_in_block() {
        run(r#"
helper "inlinefn" {
  brain {
    plan { }
    execute {
      function square(n) {
        return n * n
      }
      assert square(7) == 49 "inline function"
      assert square(0) == 0 "inline function zero"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_agent_level_function() {
        run(r#"
helper "agentfn" {
  function double(x) {
    return x * 2
  }
  brain {
    plan { }
    execute {
      assert double(21) == 42 "agent-level function"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    #[test]
    fn test_module_import_namespaced() {
        // Uses helpers/math_utils.gx via namespaced import
        run(r#"
import "tests/helpers/math_utils.gx" as math

helper "modimp" {
  brain {
    plan { }
    execute {
      assert math.add(10, 32) == 42 "module add"
      assert math.greet("World") == "Hello from math, World!" "module greet"
    }
    remember { }
    communicate { }
  }
}"#)
        .unwrap();
    }

    // ── v0.4.0 feature tests ──────────────────────────────────────────────────

    #[test]
    fn test_regex_test() {
        run(r#"
helper "re1" {
  brain { plan {} execute {
    assert regex_test("hello world", "world") "simple match"
    assert regex_test("user@example.com", "@") "at-sign match"
    assert regex_test("abc123", "\\d+") "digit match"
    assert !regex_test("no-match", "^\\d+$") "no match"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_find() {
        run(r#"
helper "re2" {
  brain { plan {} execute {
    price = regex_find("Total: $42.50", "\\$([0-9.]+)")
    assert price == "42.50" "capture group"
    nothing = regex_find("abc", "\\d+")
    assert nothing == null "no match returns null"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_find_all() {
        run(r#"
helper "re3" {
  brain { plan {} execute {
    nums = regex_find_all("a1 b2 c3", "\\d")
    assert nums[0] == "1" "first digit"
    assert nums[2] == "3" "third digit"
    assert len(nums) == 3 "count"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_replace() {
        run(r#"
helper "re4" {
  brain { plan {} execute {
    cleaned = regex_replace("aaa bbb", "a", "x")
    assert cleaned == "xxx bbb" "replace all a with x"
    no_digits = regex_replace("abc123def", "\\d", "")
    assert no_digits == "abcdef" "remove digits"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_regex_split() {
        run(r#"
helper "re5" {
  brain { plan {} execute {
    parts = regex_split("one,two,,three", ",+")
    assert parts[0] == "one" "first part"
    assert parts[1] == "two" "second part"
    assert parts[2] == "three" "third part"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_now() {
        run(r#"
helper "dt1" {
  brain { plan {} execute {
    n = date_now()
    assert n.contains("T") "ISO-8601 contains T"
    assert n.len() > 10 "has time component"
    ts = date_timestamp()
    assert ts > 1700000000 "reasonable unix timestamp"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_parse_and_format() {
        run(r#"
helper "dt2" {
  brain { plan {} execute {
    ts = date_parse("2024-01-15")
    assert ts > 0 "parse returns timestamp"
    formatted = date_format(ts, "%Y-%m-%d")
    assert formatted == "2024-01-15" "round-trip"
    year = date_format(ts, "%Y")
    assert year == "2024" "year only"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_diff() {
        run(r#"
helper "dt3" {
  brain { plan {} execute {
    t1 = date_parse("2024-01-01")
    t2 = date_parse("2024-01-08")
    diff_days = date_diff(t1, t2, "days")
    assert diff_days == 7 "seven days"
    diff_hrs = date_diff(t1, t2, "hours")
    assert diff_hrs == 168 "168 hours"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_add() {
        run(r#"
helper "dt4" {
  brain { plan {} execute {
    base = date_parse("2024-01-01")
    next_week = date_add(base, 7, "days")
    diff = date_diff(base, next_week, "days")
    assert diff == 7 "added 7 days"
    next_hour = date_add(base, 1, "hours")
    diff_h = date_diff(base, next_hour, "hours")
    assert diff_h == 1 "added 1 hour"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_date_parts() {
        run(r#"
helper "dt5" {
  brain { plan {} execute {
    ts = date_parse("2024-03-15")
    p = date_parts(ts)
    assert p.year == 2024 "year"
    assert p.month == 3 "month"
    assert p.day == 15 "day"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_csv_parse() {
        run(r#"
helper "csv1" {
  brain { plan {} execute {
    csv = "name,age,city\nAlice,30,London\nBob,25,Paris"
    rows = csv_parse(csv)
    assert len(rows) == 2 "two data rows"
    assert rows[0].name == "Alice" "first name"
    assert rows[0].age == 30 "age is number"
    assert rows[1].city == "Paris" "second city"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_csv_stringify() {
        run(r#"
helper "csv2" {
  brain { plan {} execute {
    rows = [
      { name: "Alice", age: 30 },
      { name: "Bob",   age: 25 }
    ]
    out = csv_stringify(rows)
    assert out.contains("Alice") "has Alice"
    assert out.contains("age") "has header"
    // Round-trip
    back = csv_parse(out)
    assert back[0].name == "Alice" "round-trip name"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_yaml_parse() {
        run(r#"
helper "yaml1" {
  brain { plan {} execute {
    src = "name: Alice\nage: 30\ntags:\n  - dev\n  - gx"
    data = yaml_parse(src)
    assert data.name == "Alice" "name"
    assert data.age == 30 "age is number"
    assert data.tags[0] == "dev" "first tag"
    assert data.tags[1] == "gx" "second tag"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_yaml_stringify() {
        run(r#"
helper "yaml2" {
  brain { plan {} execute {
    obj = { name: "Alice", score: 42 }
    out = yaml_stringify(obj)
    assert out.contains("Alice") "has name"
    assert out.contains("42") "has score"
    back = yaml_parse(out)
    assert back.name == "Alice" "round-trip"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_toml_parse() {
        run(r#"
helper "toml1" {
  brain { plan {} execute {
    src = "[package]\nname = \"my-app\"\nversion = \"1.0.0\"\nedition = 2021"
    data = toml_parse(src)
    assert data.package.name == "my-app" "package name"
    assert data.package.edition == 2021 "edition number"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_load_env_and_get_env_default() {
        run(r#"
helper "env1" {
  brain { plan {} execute {
    // get_env with default — key definitely does not exist
    val = get_env("GX_TEST_NONEXISTENT_KEY_12345", "fallback")
    assert val == "fallback" "default returned"
    // get_env with no default → null
    val2 = get_env("GX_TEST_NONEXISTENT_KEY_99999")
    assert val2 == null "null when missing"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_vector_store() {
        run(r#"
helper "vs1" {
  brain { plan {} execute {
    // Create a store and add documents
    store = vector_store_new("test_store_v4")
    vector_store_add(store, "doc1", [1.0, 0.0, 0.0], "red axis")
    vector_store_add(store, "doc2", [0.0, 1.0, 0.0], "green axis")
    vector_store_add(store, "doc3", [0.0, 0.0, 1.0], "blue axis")
    assert vector_store_size(store) == 3 "three documents"

    // Search: query close to red axis should find doc1 first
    hits = vector_store_search(store, [0.9, 0.1, 0.0], 2)
    assert len(hits) == 2 "two hits"
    assert hits[0].id == "doc1" "closest is doc1"
    assert hits[0].score > 0.9 "high similarity"

    // Cosine similarity of identical vectors = 1.0
    sim = cosine_similarity([1.0, 0.0], [1.0, 0.0])
    assert sim == 1.0 "identical vectors"

    // Orthogonal vectors = 0.0
    sim2 = cosine_similarity([1.0, 0.0], [0.0, 1.0])
    assert sim2 == 0.0 "orthogonal vectors"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_schema_validate() {
        run(r#"
helper "sv1" {
  brain { plan {} execute {
    // "schema" is now a keyword — use "spec" as variable name
    spec = { name: "string", age: "number", active: "boolean" }

    good = { name: "Alice", age: 30, active: true }
    r = schema_validate(good, spec)
    assert r.ok "valid passes"
    assert len(r.errors) == 0 "no errors"

    bad = { name: "Bob", age: "thirty" }
    r2 = schema_validate(bad, spec)
    assert !r2.ok "invalid fails"
    assert len(r2.errors) > 0 "has errors"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_await_block() {
        run(r#"
helper "aw1" {
  brain { plan {} execute {
    await {
      a: 1 + 1
      b: "hello" + " world"
      c: len([1, 2, 3])
    } into results
    assert results.a == 2 "a computed"
    assert results.b == "hello world" "b computed"
    assert results.c == 3 "c computed"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_retry_succeeds_first_try() {
        run(r#"
helper "retry1" {
  brain { plan {} execute {
    // Lambda runs in its own scope; test that it returns a value
    result = retry(fn() {
      return 42
    }, 3)
    assert result == 42 "got result"
    // retry with string result
    msg = retry(fn() {
      return "hello"
    }, 2)
    assert msg == "hello" "string result"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_tool_definition_parsed() {
        // Verify the tool keyword parses and the helper runs correctly
        run(r#"
tool "greet_user" {
  description: "Greet a user by name"
  execute(name) {
    return "Hello " + name
  }
}

helper "tool_test" {
  brain { plan {} execute {
    assert true "tool definition accepted"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    // ── v0.4.1 friction-point fixes ───────────────────────────────────────────

    #[test]
    fn test_closure_captures_locals() {
        run(r#"
helper "cap" {
  brain { plan {} execute {
    multiplier = 3
    times = fn(n) { return n * multiplier }
    assert times(5) == 15 "closure captures multiplier"

    base = "https://x.com"
    path = "/users"
    build = fn() { return base + path }
    assert build() == "https://x.com/users" "closure captures two locals"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_closure_param_shadows_capture() {
        run(r#"
helper "shadow" {
  brain { plan {} execute {
    x = 100
    f = fn(x) { return x + 1 }
    assert f(5) == 6 "param shadows captured var"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_closure_in_object_field() {
        run(r#"
helper "objfn" {
  brain { plan {} execute {
    handlers = {
      "double": fn(n) { return n * 2 },
      "square": fn(n) { return n * n }
    }
    assert handlers.double(21) == 42 "obj.field(args) calls closure"
    assert handlers.square(7) == 49 "obj.field square"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_retry_captures_state() {
        run(r#"
helper "retrycap" {
  brain { plan {} execute {
    url = "https://api.test.com"
    body = { q: "hello" }
    result = retry(fn() {
      return { url: url, q: body.q }
    }, 3)
    assert result.url == "https://api.test.com" "retry closure captures url"
    assert result.q == "hello" "retry closure captures body"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_top_level_statements() {
        run(r#"
api_base = "https://api.example.com"
max_retries = 5
config = { timeout: 30 }

helper "toplevel" {
  brain { plan {} execute {
    assert api_base == "https://api.example.com" "file-root string var"
    assert max_retries == 5 "file-root number var"
    assert config.timeout == 30 "file-root object var"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_map_filter_with_closures() {
        run(r#"
helper "mapfilter" {
  brain { plan {} execute {
    factor = 10
    nums = [1, 2, 3, 4, 5]
    scaled = map(nums, fn(n) { return n * factor })
    assert scaled[0] == 10 "map captures factor"
    assert scaled[4] == 50 "map last element"

    threshold = 3
    big = filter(nums, fn(n) { return n > threshold })
    assert len(big) == 2 "filter captures threshold"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert_builtins() {
        run(r#"
helper "assertfns" {
  brain { plan {} execute {
    assert_eq(2 + 2, 4, "assert_eq")
    assert_true(1 < 2, "assert_true")
    assert_contains("hello world", "world", "assert_contains string")
    assert_contains([1, 2, 3], 2, "assert_contains array")
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_is_tty_builtin() {
        run(r#"
helper "ttytest" {
  brain { plan {} execute {
    t = is_tty()
    assert t == true or t == false "is_tty returns bool"
  } remember {} communicate {} }
}"#)
        .unwrap();
    }

    #[test]
    fn test_assert_eq_failure() {
        let result = run(r#"
helper "assertfail" {
  brain { plan {} execute {
    assert_eq(1, 2, "should fail")
  } remember {} communicate {} }
}"#);
        assert!(result.is_err(), "assert_eq(1, 2) should fail");
    }

    #[test]
    fn test_runtime_error_has_line_and_stack() {
        let result = run(r#"
function divide(a, b) {
  return a / b
}

helper "errctx" {
  brain { plan {} execute {
    y = divide(10, 0)
  } remember {} communicate {} }
}"#);
        let msg = match result {
            Err(m) => m,
            Ok(_) => panic!("expected a runtime error"),
        };
        assert!(
            msg.contains("at line"),
            "error should include line number: {}",
            msg
        );
        assert!(
            msg.contains("divide()"),
            "error should include stack frame: {}",
            msg
        );
    }
}
