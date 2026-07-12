//! `gx check`'s static analysis pass — diagnostics that are fully
//! detectable from the AST alone, with a deliberately low false-positive
//! bar (see each check's own doc for why it's safe to flag).
//!
//! This walks the *whole project* (the entry file plus everything it
//! transitively `import`s — see `Interpreter::build_project_index`), not
//! just the one file passed on the command line, so an agent declared in
//! one file and spawned from another is still recognized as referenced,
//! and a spawn target declared anywhere in the project is still checked
//! against its real capabilities.

use crate::ast::{
    Expr, FunctionDef, HelperDef, InterpolatedPart, Program, RouteDecl, Stmt, ToolDef, WhenTrigger,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
    /// Best-effort source line. `Expr` nodes don't carry their own line
    /// number in this AST — a finding anchored inside an expression uses
    /// the line of its nearest enclosing statement.
    pub line: usize,
}

/// Lightweight, internal-only metadata describing what an agent exposes —
/// not a reflection API for scripts to introspect, just enough for the
/// runtime and `gx check` to agree on what "callable" means for a given
/// agent. Mirrors the exact routing rule `Interpreter::call_agent` uses at
/// runtime (see its doc comment), so a finding here and the error a script
/// would actually hit at runtime never disagree.
#[derive(Debug, Clone, Default)]
pub struct AgentMeta {
    pub has_brain: bool,
    pub message_actions: Vec<String>,
    pub has_started: bool,
    pub has_cron: bool,
    /// Whether this agent runs standalone at program start (a `brain { }`
    /// that never references `input`, or a `when started` block) rather
    /// than existing to be `spawn`ed/`send`-ed to by something else — the
    /// same rule `run_program` uses (via `helper_is_callable_only`) to
    /// decide whether to auto-run an agent. The dead-agent lint must not
    /// flag these: a standalone agent auto-runs by design, never by being
    /// referenced elsewhere.
    pub runs_standalone: bool,
    pub line: usize,
}

impl AgentMeta {
    pub fn of(helper: &HelperDef) -> Self {
        let mut m = AgentMeta {
            has_brain: helper.brain.is_some(),
            runs_standalone: helper.brain.is_some()
                && !crate::interpreter::helper_is_callable_only(helper),
            line: helper.line,
            ..Default::default()
        };
        for wb in &helper.when_blocks {
            match &wb.trigger {
                WhenTrigger::Message(name) => m.message_actions.push(name.clone()),
                WhenTrigger::Started => m.has_started = true,
                WhenTrigger::Cron(_) => m.has_cron = true,
                WhenTrigger::Expr(_) | WhenTrigger::Changes(_) => {}
            }
        }
        m
    }

    /// Whether `spawn agent "x" with { ... }` can get a real value back.
    /// Only a `brain { }` (via `communicate`) can — `when message` stays
    /// deliberately async-only regardless of what `action` is passed; see
    /// `Interpreter::call_agent`'s doc comment for why the two are never
    /// unified.
    pub fn is_sync_callable(&self) -> bool {
        self.has_brain
    }
}

struct Analysis<'a> {
    metas: &'a HashMap<String, AgentMeta>,
    findings: Vec<Finding>,
    referenced: HashSet<String>,
    current_line: usize,
}

impl<'a> Analysis<'a> {
    fn walk_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.walk_stmt(s);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        self.current_line = stmt_line(stmt);
        match stmt {
            Stmt::Assign { target, value, .. }
            | Stmt::PlusAssign { target, value, .. }
            | Stmt::MinusAssign { target, value, .. }
            | Stmt::MulAssign { target, value, .. }
            | Stmt::DivAssign { target, value, .. } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for (cond, body) in branches {
                    self.walk_expr(cond);
                    self.walk_stmts(body);
                }
                if let Some(b) = else_body {
                    self.walk_stmts(b);
                }
            }
            Stmt::ForEach { iter, body, .. } => {
                self.walk_expr(iter);
                self.walk_stmts(body);
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.walk_expr(condition);
                self.walk_stmts(body);
            }
            Stmt::TryCatch {
                try_body,
                catch_body,
                ..
            } => {
                self.walk_stmts(try_body);
                self.walk_stmts(catch_body);
            }
            Stmt::Assert {
                condition, message, ..
            } => {
                self.walk_expr(condition);
                if let Some(m) = message {
                    self.walk_expr(m);
                }
            }
            Stmt::Emit { payload, .. } => {
                for (_, e) in payload {
                    self.walk_expr(e);
                }
            }
            Stmt::Broadcast { .. } => {}
            Stmt::Log { value, .. } | Stmt::Output { value, .. } | Stmt::Say { value, .. } => {
                self.walk_expr(value);
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.walk_expr(v);
                }
            }
            Stmt::Expr { expr, .. } => self.walk_expr(expr),
            Stmt::Wait { ms, .. } => self.walk_expr(ms),
            Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::ReRun { .. }
            | Stmt::EscalateToHuman { .. } => {}
            Stmt::Serve { routes, .. } => {
                for r in routes {
                    self.walk_route(r);
                }
            }
            Stmt::SendMessage {
                agent_name,
                event,
                data,
                ..
            } => {
                self.walk_expr(agent_name);
                for (_, e) in data {
                    self.walk_expr(e);
                }
                if let Expr::Str(name) = agent_name {
                    self.referenced.insert(name.clone());
                }
                self.check_send_message(agent_name, event);
            }
            Stmt::Think {
                prompt,
                temperature,
                min_confidence,
                ..
            } => {
                self.walk_expr(prompt);
                if let Some(t) = temperature {
                    self.walk_expr(t);
                }
                if let Some(m) = min_confidence {
                    self.walk_expr(m);
                }
            }
            Stmt::Observe { bindings, .. } => {
                for (_, e) in bindings {
                    self.walk_expr(e);
                }
            }
            Stmt::Act { body, .. } => self.walk_stmts(body),
            Stmt::LoopUntil {
                condition, body, ..
            } => {
                self.walk_expr(condition);
                self.walk_stmts(body);
            }
            Stmt::RepeatTimes { count, body, .. } => {
                self.walk_expr(count);
                self.walk_stmts(body);
            }
            Stmt::Parallel { branches, .. } => {
                for b in branches {
                    self.walk_stmts(b);
                }
            }
            Stmt::Respond { value, .. } => self.walk_expr(value),
            Stmt::RespondStream { body, .. } => self.walk_stmts(body),
            Stmt::Span { name, body, .. } => {
                self.walk_expr(name);
                self.walk_stmts(body);
            }
            Stmt::Await { bindings, .. } => {
                for (_, e) in bindings {
                    self.walk_expr(e);
                }
            }
            Stmt::DbTransaction { path, body, .. } => {
                self.walk_expr(path);
                self.walk_stmts(body);
            }
        }
    }

    fn walk_route(&mut self, route: &RouteDecl) {
        self.walk_stmts(&route.body);
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Str(_) | Expr::Num(_) | Expr::Bool(_) | Expr::Null | Expr::Ident(_) => {}
            Expr::FieldAccess { object, .. } => self.walk_expr(object),
            Expr::Index { object, index } => {
                self.walk_expr(object);
                self.walk_expr(index);
            }
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
                if let Expr::Ident(name) = callee.as_ref() {
                    if (name == "db_exec" || name == "db_query") && args.len() > 1 {
                        self.check_sql_argument(name, &args[1]);
                    }
                }
            }
            Expr::Object(pairs) => {
                for (_, e) in pairs {
                    self.walk_expr(e);
                }
            }
            Expr::Array(items) => {
                for e in items {
                    self.walk_expr(e);
                }
            }
            Expr::BinOp { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Not(e) => self.walk_expr(e),
            Expr::Interpolated(parts) => {
                for p in parts {
                    if let InterpolatedPart::Expr(e) = p {
                        self.walk_expr(e);
                    }
                }
            }
            Expr::AskAI { params, .. } => {
                for (_, e) in params {
                    self.walk_expr(e);
                }
            }
            Expr::Embed { text } => self.walk_expr(text),
            Expr::InferClassifier { input, classes } => {
                self.walk_expr(input);
                self.walk_expr(classes);
            }
            Expr::BridgeCall { args, .. } => {
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::CallAgent {
                name,
                input,
                timeout_ms,
            } => {
                self.walk_expr(name);
                for (_, e) in input {
                    self.walk_expr(e);
                }
                if let Some(t) = timeout_ms {
                    self.walk_expr(t);
                }
                self.check_call_agent(name);
            }
            Expr::Lambda { body, .. } => self.walk_stmts(body),
            Expr::Range { start, end } => {
                self.walk_expr(start);
                self.walk_expr(end);
            }
            Expr::ParallelMap(pairs) => {
                for (_, e) in pairs {
                    self.walk_expr(e);
                }
            }
        }
    }

    /// `spawn agent "name" with { ... }` targeting an agent with no
    /// synchronous return path — the exact class of bug this milestone's
    /// report identified as its single most damaging finding (a
    /// `when message`-only agent silently returning null to every caller).
    ///
    /// `brain{}` and `when message` stay genuinely distinct: `when message`
    /// never satisfies `spawn agent`'s "return a value" contract, no matter
    /// what `action` is passed — so this only needs one static fact, not a
    /// literal-`action`-matching heuristic: does the target have a
    /// `brain {}` block, yes or no. Only flags what's statically certain —
    /// a literal target name naming a *known* agent (one this project
    /// actually declares; an unknown name might be built dynamically, or
    /// belong to a file this check didn't see) — the same "very low
    /// false-positive rate" bar every check here holds to.
    fn check_call_agent(&mut self, name: &Expr) {
        let Expr::Str(target) = name else { return };
        self.referenced.insert(target.clone());
        let Some(meta) = self.metas.get(target) else {
            return;
        };
        if meta.has_brain {
            return;
        }
        let message = if meta.message_actions.is_empty() {
            format!(
                "spawn agent \"{target}\" with {{ ... }} expects a synchronous return value, \
                 but \"{target}\" has no `brain {{ }}` block — this fails at runtime with \
                 \"cannot be called synchronously\". Use `spawn \"action\" to \"{target}\" with \
                 {{ ... }}` instead, or give \"{target}\" a `brain {{ }}` implementation."
            )
        } else {
            format!(
                "spawn agent \"{target}\" with {{ ... }} expects a synchronous return value, but \
                 \"{target}\" only exposes asynchronous `when message` handlers ({}) — no \
                 `brain {{ }}` block. This fails at runtime regardless of the `action` passed. \
                 Use `spawn \"action\" to \"{target}\" with {{ ... }}` instead, or give \
                 \"{target}\" a `brain {{ }}` implementation.",
                meta.message_actions.join(", ")
            )
        };
        self.findings.push(Finding {
            severity: Severity::Error,
            message,
            line: self.current_line,
        });
    }

    /// `spawn "event" to "agent"` (fire-and-forget) targeting an agent that
    /// isn't declared anywhere in this project — a literal target name
    /// this statically certain about is caught the same way an unknown
    /// `spawn agent` target is: a typo'd or removed agent name here used to
    /// silently queue the message forever (see
    /// `Interpreter::run_stmt`'s `Stmt::SendMessage` arm), with nothing to
    /// ever deliver it and no signal anything went wrong. A non-literal
    /// target is skipped rather than guessed at, same as every other check
    /// here — it might be built dynamically, or name an agent declared in
    /// a file this check didn't see.
    fn check_send_message(&mut self, agent_name: &Expr, event: &str) {
        let Expr::Str(target) = agent_name else {
            return;
        };
        if self.metas.contains_key(target) {
            return;
        }
        self.findings.push(Finding {
            severity: Severity::Error,
            message: format!(
                "spawn \"{event}\" to \"{target}\" with {{ ... }} — \"{target}\" is not declared \
                 anywhere in this project. This message can never be delivered; check the agent \
                 name for a typo."
            ),
            line: self.current_line,
        });
    }

    /// Flags a `db_exec`/`db_query` SQL argument built by splicing a
    /// non-literal value directly into the query string — via `+`
    /// concatenation or (GX's own, arguably more tempting version of the
    /// same mistake) string interpolation, e.g. `"SELECT * FROM t WHERE
    /// id = {user_id}"` — instead of a `?` placeholder with the value
    /// passed as a separate parameter. Both forms bypass SQLite's own
    /// escaping and are the textbook SQL-injection shape; `?` placeholders
    /// already exist in GX (`db_exec(path, "... WHERE id = ?", [user_id])`)
    /// specifically so this never has to happen. A SQL string built
    /// entirely from literals (no interpolated *expression*, no `+` with a
    /// non-literal operand) is fine and not flagged.
    fn check_sql_argument(&mut self, fn_name: &str, sql_arg: &Expr) {
        if expr_is_dynamic_string(sql_arg) {
            self.findings.push(Finding {
                severity: Severity::Warning,
                message: format!(
                    "{fn_name}'s SQL argument is built by splicing a value directly into the \
                     query string (via `+` or string interpolation) instead of a `?` \
                     placeholder — this is the textbook SQL-injection shape. Use \
                     `{fn_name}(path, \"... WHERE col = ?\", [value])` instead."
                ),
                line: self.current_line,
            });
        }
    }
}

/// Whether every part of `expr` is a compile-time-constant string — plain
/// literals, and/or `+`/`{interpolation}` combining only other constant
/// strings. Only `Add`/`Concat`/`Interpolated` are examined; anything else
/// (a bare variable, a function call, ...) is "not a literal" but also not
/// itself a concatenation/interpolation *shape*, so it's handled by the
/// caller rather than here.
fn is_literal_string_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Str(_) => true,
        Expr::BinOp {
            left,
            op: crate::ast::BinOp::Add | crate::ast::BinOp::Concat,
            right,
        } => is_literal_string_expr(left) && is_literal_string_expr(right),
        Expr::Interpolated(parts) => parts
            .iter()
            .all(|p| matches!(p, InterpolatedPart::Literal(_))),
        _ => false,
    }
}

/// Whether `expr` is a string built by splicing in at least one non-literal
/// part — `+`/string-interpolation shape, but not provably constant per
/// `is_literal_string_expr`. A bare identifier/call used as the whole SQL
/// argument (no concatenation/interpolation visible here at all) is
/// deliberately not flagged: this check can't see whether it was built
/// safely elsewhere, and guessing would raise the false-positive rate this
/// milestone specifically asked to keep low.
fn expr_is_dynamic_string(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::BinOp {
            op: crate::ast::BinOp::Add | crate::ast::BinOp::Concat,
            ..
        } | Expr::Interpolated(_)
    ) && !is_literal_string_expr(expr)
}

/// Extracts every `Stmt` variant's own `line` field — needed because
/// `Expr` nodes don't carry one in this AST, so a finding anchored inside
/// an expression uses its nearest enclosing statement's line instead.
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
        | Stmt::Break { line, .. }
        | Stmt::Continue { line, .. }
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
        | Stmt::ReRun { line, .. }
        | Stmt::EscalateToHuman { line, .. }
        | Stmt::Serve { line, .. }
        | Stmt::SendMessage { line, .. }
        | Stmt::Think { line, .. }
        | Stmt::Observe { line, .. }
        | Stmt::Act { line, .. }
        | Stmt::LoopUntil { line, .. }
        | Stmt::RepeatTimes { line, .. }
        | Stmt::Parallel { line, .. }
        | Stmt::Respond { line, .. }
        | Stmt::RespondStream { line, .. }
        | Stmt::Span { line, .. }
        | Stmt::Await { line, .. }
        | Stmt::DbTransaction { line, .. } => *line,
    }
}

/// Runs every static check over the whole project (`helpers`/`functions`/
/// `tools` already merged across every transitively-`import`ed file — see
/// `Interpreter::build_project_index` — plus `entry`'s own top-level
/// statements, which aren't part of that merged index).
pub fn check_program(
    entry: &Program,
    helpers: &HashMap<String, HelperDef>,
    functions: &HashMap<String, FunctionDef>,
    tools: &HashMap<String, ToolDef>,
) -> Vec<Finding> {
    let metas: HashMap<String, AgentMeta> = helpers
        .iter()
        .map(|(k, v)| (k.clone(), AgentMeta::of(v)))
        .collect();

    let mut analysis = Analysis {
        metas: &metas,
        findings: Vec::new(),
        referenced: HashSet::new(),
        current_line: 0,
    };

    analysis.walk_stmts(&entry.top_level_stmts);
    for f in functions.values() {
        analysis.walk_stmts(&f.body);
    }
    for t in tools.values() {
        analysis.walk_stmts(&t.body);
    }
    for h in helpers.values() {
        if let Some(brain) = &h.brain {
            analysis.walk_stmts(&brain.plan);
            analysis.walk_stmts(&brain.execute);
            analysis.walk_stmts(&brain.remember);
            analysis.walk_stmts(&brain.communicate);
        }
        for wb in &h.when_blocks {
            analysis.walk_stmts(&wb.body);
        }
    }

    // "Declared but never spawned" — a fully static dead-code check: no GX
    // program can construct an agent name dynamically in a way this can't
    // see (agent names are always string literals, at every call site that
    // targets one), so an agent whose name never appears as a literal
    // `spawn`/`send` target anywhere in the project is either genuinely
    // unused or was left half-wired during a refactor. A `when started`
    // agent still runs on its own (auto-run at program start, not spawned)
    // so it's deliberately excluded — only agents whose *entire* purpose is
    // being called by something else qualify.
    for (name, meta) in &metas {
        if !analysis.referenced.contains(name)
            && !meta.has_started
            && !meta.has_cron
            && !meta.runs_standalone
        {
            analysis.findings.push(Finding {
                severity: Severity::Warning,
                message: format!(
                    "agent \"{name}\" is declared but never spawned or sent a message from \
                     anywhere in this project — dead code, unless its name is constructed \
                     dynamically somewhere this check can't see."
                ),
                line: meta.line,
            });
        }
    }

    analysis.findings.sort_by_key(|f| f.line);
    analysis.findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check(src: &str) -> Vec<Finding> {
        let tokens = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let helpers: HashMap<String, HelperDef> = program
            .helpers
            .iter()
            .map(|h| (h.name.clone(), h.clone()))
            .collect();
        let functions: HashMap<String, FunctionDef> = program
            .functions
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        let tools: HashMap<String, ToolDef> = program
            .tools
            .iter()
            .map(|t| (t.name.clone(), t.clone()))
            .collect();
        check_program(&program, &helpers, &functions, &tools)
    }

    fn messages(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.message.as_str()).collect()
    }

    #[test]
    fn flags_spawn_agent_targeting_a_message_only_agent_even_with_a_mismatched_action() {
        let findings = check(
            r#"
agent "worker" {
  when message "do_thing" { { ok: true } }
}
agent "caller" {
  when started {
    x = spawn agent "worker" with { action: "nope" }
  }
}
"#,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("only exposes asynchronous"));
        assert!(findings[0].message.contains("do_thing"));
    }

    #[test]
    fn flags_spawn_agent_targeting_a_message_only_agent_even_with_a_matching_action() {
        // `brain{}` and `when message` stay genuinely distinct: `when
        // message` never satisfies `spawn agent`'s synchronous contract,
        // no matter what `action` is passed — including one that happens
        // to name a real handler. This is the corrected design after
        // reconsidering the alternative (auto-routing a matching `action`
        // into the handler): that would make a handler's contract depend
        // on its caller instead of its own declaration.
        let findings = check(
            r#"
agent "worker" {
  when message "do_thing" { { ok: true } }
}
agent "caller" {
  when started {
    x = spawn agent "worker" with { action: "do_thing" }
  }
}
"#,
        );
        assert_eq!(findings.len(), 1, "unexpected findings: {:?}", findings);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("only exposes asynchronous"));
    }

    #[test]
    fn flags_spawn_agent_targeting_a_fully_async_only_agent() {
        let findings = check(
            r#"
agent "worker" {
  when started { log("hi") }
}
agent "caller" {
  when started {
    x = spawn agent "worker" with { action: "whatever" }
  }
}
"#,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0]
            .message
            .contains("cannot be called synchronously"));
    }

    #[test]
    fn does_not_flag_a_brain_agent_regardless_of_action_field() {
        let findings = check(
            r#"
agent "worker" {
  brain {
    plan {}
    execute {}
    remember {}
    communicate { true }
  }
}
agent "caller" {
  when started {
    x = spawn agent "worker" with { action: "whatever" }
  }
}
"#,
        );
        assert!(findings.is_empty(), "unexpected findings: {:?}", findings);
    }

    #[test]
    fn does_not_flag_a_dynamic_spawn_target() {
        // Low false-positive rate is the whole point: a non-literal target
        // can't be verified statically (it might be a `brain{}` agent this
        // check simply can't resolve by name), so it must be skipped
        // rather than guessed at. Unlike the target, the `action` field's
        // literalness no longer matters to this check at all — a
        // message-only agent is flagged regardless of what `action` was
        // passed — so only the dynamic-*target* case needs covering here.
        let findings = check(
            r#"
agent "worker" {
  when message "do_thing" { { ok: true } }
}
agent "caller" {
  when started {
    target_name = "worker"
    x = spawn agent target_name with { action: "do_thing" }
  }
}
"#,
        );
        // A dynamic target is also invisible to the dead-agent lint (it
        // can't tell "worker" was actually referenced) — that's a separate,
        // pre-existing, and correct limitation of *that* check, not
        // something the spawn-target check under test here should produce.
        assert!(
            findings.iter().all(|f| f.severity != Severity::Error),
            "unexpected error findings: {:?}",
            findings
        );
    }

    #[test]
    fn flags_a_fire_and_forget_send_to_an_unknown_agent() {
        // Regression test: a literal `spawn "event" to "agent"` target
        // that names no declared agent anywhere in the project used to be
        // fully invisible to `gx check` — the message it sends can never
        // be delivered (see the matching runtime fix in
        // `Interpreter::run_stmt`'s `Stmt::SendMessage` arm).
        let findings = check(
            r#"
agent "caller" {
  when started {
    spawn "some_event" to "totally_nonexistent_agent" with { x: 1 }
  }
}
"#,
        );
        assert_eq!(findings.len(), 1, "unexpected findings: {:?}", findings);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("totally_nonexistent_agent"));
        assert!(findings[0].message.contains("never be delivered"));
    }

    #[test]
    fn does_not_flag_a_fire_and_forget_send_to_a_known_agent() {
        let findings = check(
            r#"
agent "worker" {
  when message "greet" { log("hi") }
}
agent "caller" {
  when started {
    spawn "greet" to "worker" with { }
  }
}
"#,
        );
        assert!(
            findings.iter().all(|f| f.severity != Severity::Error),
            "unexpected error findings: {:?}",
            findings
        );
    }

    #[test]
    fn does_not_flag_a_dynamic_fire_and_forget_target() {
        // Same low-false-positive bar as the spawn-agent check: a
        // non-literal target can't be verified statically, so it's skipped
        // rather than guessed at.
        let findings = check(
            r#"
agent "caller" {
  when started {
    target_name = "some_agent_built_elsewhere"
    spawn "greet" to target_name with { }
  }
}
"#,
        );
        assert!(
            findings.iter().all(|f| f.severity != Severity::Error),
            "unexpected error findings: {:?}",
            findings
        );
    }

    #[test]
    fn flags_a_never_spawned_agent() {
        let findings = check(
            r#"
agent "unused" {
  when message "hi" { { ok: true } }
}
"#,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].message.contains("never spawned"));
    }

    #[test]
    fn does_not_flag_a_standalone_brain_agent_that_never_references_input() {
        // The single most common GX pattern: one agent, `brain { }`, no
        // `input` reference — auto-run once at program start, never
        // spawned by anything, and that's by design.
        let findings = check(
            r#"
agent "standalone" {
  brain {
    plan {}
    execute {}
    remember {}
    communicate {}
  }
}
"#,
        );
        assert!(findings.is_empty(), "unexpected findings: {:?}", findings);
    }

    #[test]
    fn does_not_flag_a_when_started_agent_that_is_never_spawned() {
        let findings = check(
            r#"
agent "boot" {
  when started { log("hi") }
}
"#,
        );
        assert!(findings.is_empty(), "unexpected findings: {:?}", findings);
    }

    #[test]
    fn flags_sql_built_by_string_concatenation() {
        let findings = check(
            r#"
agent "t" {
  when started {
    user_id = "5"
    x = db_query("app.db", "SELECT * FROM users WHERE id = " + user_id)
  }
}
"#,
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("SQL-injection"));
    }

    #[test]
    fn flags_sql_built_by_interpolation() {
        let findings = check(
            r#"
agent "t" {
  when started {
    user_id = "5"
    x = db_query("app.db", "SELECT * FROM users WHERE id = {user_id}")
  }
}
"#,
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("SQL-injection"));
    }

    #[test]
    fn does_not_flag_parameterized_sql_or_a_plain_literal() {
        let findings = check(
            r#"
agent "t" {
  when started {
    user_id = "5"
    x = db_query("app.db", "SELECT * FROM users WHERE id = ?", [user_id])
    y = db_query("app.db", "SELECT * FROM users")
  }
}
"#,
        );
        assert!(messages(&findings)
            .iter()
            .all(|m| !m.contains("SQL-injection")));
    }

    #[test]
    fn does_not_flag_sql_built_from_multiple_literal_strings() {
        // A rare style, but not a real injection risk: nothing non-literal
        // is ever spliced in, even though it's still a `+` chain.
        let findings = check(
            r#"
agent "t" {
  when started {
    x = db_query("app.db", "SELECT * FROM users" + " WHERE active = 1")
  }
}
"#,
        );
        assert!(messages(&findings)
            .iter()
            .all(|m| !m.contains("SQL-injection")));
    }
}
