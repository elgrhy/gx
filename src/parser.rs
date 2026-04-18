//! GX Parser — turns a token stream into an AST.

use crate::ast::*;
use crate::lexer::{Token, TokenKind};
use std::collections::HashSet;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Namespaces declared via `use js.X`, `use py.X`
    namespaces: HashSet<String>,
}

// ── Core helpers ──────────────────────────────────────────────────────────────

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            namespaces: HashSet::new(),
        }
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn line(&self) -> usize {
        self.tokens[self.pos].line
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<(), String> {
        self.skip_newlines();
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "Line {}: expected {:?}, got {:?}",
                self.line(),
                kind,
                self.peek_kind()
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        self.skip_newlines();
        let name = match self.peek_kind().clone() {
            TokenKind::Ident(s) => s,
            TokenKind::Plan => "plan".into(),
            TokenKind::Execute => "execute".into(),
            TokenKind::Remember => "remember".into(),
            TokenKind::Communicate => "communicate".into(),
            TokenKind::Type => "type".into(),
            TokenKind::Source => "source".into(),
            TokenKind::Bind => "bind".into(),
            TokenKind::On => "on".into(),
            TokenKind::Memory => "memory".into(),
            TokenKind::Count => "count".into(),
            TokenKind::Push => "push".into(),
            TokenKind::Log => "log".into(),
            TokenKind::Output => "output".into(),
            TokenKind::Assign => "assign".into(),
            TokenKind::Spawn => "spawn".into(),
            TokenKind::Wait => "wait".into(),
            TokenKind::Channel => "channel".into(),
            TokenKind::Receive => "receive".into(),
            TokenKind::Needs => "needs".into(),
            TokenKind::Gives => "gives".into(),
            TokenKind::From => "from".into(),
            TokenKind::As => "as".into(),
            TokenKind::Do => "do".into(),
            TokenKind::When => "when".into(),
            TokenKind::Then => "then".into(),
            TokenKind::In => "in".into(),
            TokenKind::And => "and".into(),
            TokenKind::Or => "or".into(),
            TokenKind::Not => "not".into(),
            TokenKind::Use => "use".into(),
            TokenKind::Started => "started".into(),
            TokenKind::Escalate => "escalate".into(),
            TokenKind::Human => "human".into(),
            TokenKind::Changes => "changes".into(),
            TokenKind::Ask => "ask".into(),
            TokenKind::Embed => "embed".into(),
            TokenKind::Infer => "infer".into(),
            TokenKind::Classifier => "classifier".into(),
            TokenKind::Broadcast => "broadcast".into(),
            TokenKind::Function => "function".into(),
            TokenKind::Import => "import".into(),
            TokenKind::Emit => "emit".into(),
            TokenKind::Recipe => "recipe".into(),
            TokenKind::Objective => "objective".into(),
            TokenKind::While => "while".into(),
            TokenKind::Break => "break".into(),
            TokenKind::Continue => "continue".into(),
            TokenKind::Assert => "assert".into(),
            TokenKind::Serve => "serve".into(),
            TokenKind::Route => "route".into(),
            TokenKind::Respond => "respond".into(),
            TokenKind::Port => "port".into(),
            other => {
                return Err(format!(
                    "Line {}: expected identifier, got {:?}",
                    self.line(),
                    other
                ))
            }
        };
        self.advance();
        Ok(name)
    }

    fn expect_string(&mut self) -> Result<String, String> {
        self.skip_newlines();
        match self.peek_kind().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!(
                "Line {}: expected string, got {:?}",
                self.line(),
                other
            )),
        }
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        self.skip_newlines();
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_block(&mut self) -> Result<(), String> {
        self.skip_newlines();
        if !self.eat(&TokenKind::LBrace) {
            return Ok(());
        }
        let mut depth = 1usize;
        loop {
            match self.peek_kind() {
                TokenKind::Eof => return Err("Unclosed block".into()),
                TokenKind::LBrace => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RBrace => {
                    self.advance();
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }
}

// ── Top-level ─────────────────────────────────────────────────────────────────

impl Parser {
    pub fn parse(&mut self) -> Result<Program, String> {
        let mut file_imports = Vec::new();
        let mut imports = Vec::new();
        let mut functions = Vec::new();
        let mut helpers = Vec::new();
        let mut top_level_brain = None;

        loop {
            self.skip_newlines();
            match self.peek_kind().clone() {
                TokenKind::Eof => break,
                TokenKind::Import => {
                    file_imports.push(self.parse_file_import()?);
                }
                TokenKind::Use => {
                    imports.push(self.parse_import()?);
                }
                TokenKind::Function => {
                    functions.push(self.parse_function()?);
                }
                TokenKind::Helper | TokenKind::Agent => {
                    helpers.push(self.parse_helper()?);
                }
                TokenKind::Brain => {
                    top_level_brain = Some(self.parse_brain_block()?);
                }
                other => {
                    return Err(format!(
                        "Line {}: unexpected top-level token {:?}",
                        self.line(),
                        other
                    ));
                }
            }
        }

        Ok(Program {
            file_imports,
            imports,
            functions,
            helpers,
            top_level_brain,
        })
    }

    fn parse_file_import(&mut self) -> Result<FileImport, String> {
        let line = self.line();
        self.advance(); // consume `import`
        let path = self.expect_string()?;
        Ok(FileImport { path, line })
    }

    fn parse_function(&mut self) -> Result<FunctionDef, String> {
        let line = self.line();
        self.advance(); // consume `function`
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RParen) {
                break;
            }
            params.push(self.expect_ident()?);
            if !self.eat(&TokenKind::Comma) {
                self.skip_newlines();
                self.expect(&TokenKind::RParen)?;
                break;
            }
        }
        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_stmts()?;
        Ok(FunctionDef {
            name,
            params,
            body,
            line,
        })
    }

    fn parse_import(&mut self) -> Result<ImportDecl, String> {
        let line = self.line();
        self.advance(); // consume `use`
        let namespace = self.expect_ident()?;
        self.expect(&TokenKind::Dot)?;
        let package = self.expect_ident()?;
        self.namespaces.insert(namespace.clone());
        Ok(ImportDecl {
            namespace,
            package,
            line,
        })
    }

    fn parse_helper(&mut self) -> Result<HelperDef, String> {
        let line = self.line();
        self.advance(); // consume `helper` or `agent`
        let name = self.expect_string()?;
        self.expect(&TokenKind::LBrace)?;

        let mut can_do = Vec::new();
        let mut memory = Vec::new();
        let mut receive_block = Vec::new();
        let mut brain = None;
        let mut recipes = Vec::new();
        let mut objectives = Vec::new();
        let mut when_blocks = Vec::new();

        loop {
            self.skip_newlines();
            match self.peek_kind().clone() {
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                TokenKind::Eof => return Err(format!("Line {}: unclosed helper '{}'", line, name)),

                TokenKind::Ident(ref s) if s == "can_do" || s == "capabilities" => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    can_do = self.parse_string_array()?;
                }
                TokenKind::CanDo => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    can_do = self.parse_string_array()?;
                }

                // `remember { ... }` block OR `remember key = value` single-line
                TokenKind::Remember | TokenKind::Memory => {
                    self.advance();
                    self.skip_newlines();
                    if matches!(self.peek_kind(), TokenKind::LBrace) {
                        self.advance();
                        memory.extend(self.parse_memory_entries()?);
                    } else {
                        // single-line: remember key = value
                        let entry_line = self.line();
                        let key = self.expect_ident()?;
                        self.expect(&TokenKind::Eq)?;
                        let value = self.parse_expr()?;
                        memory.push(MemoryEntry {
                            key,
                            value,
                            line: entry_line,
                        });
                    }
                }

                TokenKind::Receive => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    receive_block = self.parse_receive_block()?;
                }

                TokenKind::Brain => {
                    brain = Some(self.parse_brain_block()?);
                }
                TokenKind::Recipe => {
                    recipes.push(self.parse_recipe()?);
                }
                TokenKind::Objective => {
                    objectives.push(self.parse_objective()?);
                }

                // Phase 2: `when X { ... }` inside agent body
                TokenKind::When => {
                    when_blocks.push(self.parse_when_block()?);
                }

                // `message` blocks — skip (Phase 2)
                TokenKind::Ident(ref s) if s == "message" => {
                    self.advance();
                    let _ = self.expect_string();
                    self.skip_block()?;
                }

                other => {
                    return Err(format!(
                        "Line {}: unexpected token in helper '{}' body: {:?}",
                        self.line(),
                        name,
                        other
                    ));
                }
            }
        }

        Ok(HelperDef {
            name,
            can_do,
            memory,
            receive_block,
            brain,
            recipes,
            objectives,
            when_blocks,
            line,
        })
    }

    fn parse_string_array(&mut self) -> Result<Vec<String>, String> {
        self.expect(&TokenKind::LBracket)?;
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBracket) {
                break;
            }
            items.push(self.expect_string()?);
            self.eat(&TokenKind::Comma);
        }
        Ok(items)
    }

    fn parse_memory_entries(&mut self) -> Result<Vec<MemoryEntry>, String> {
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            let line = self.line();
            let key = self.expect_ident()?;
            self.expect(&TokenKind::Eq)?;
            let value = self.parse_expr()?;
            entries.push(MemoryEntry { key, value, line });
            self.eat(&TokenKind::Comma);
        }
        Ok(entries)
    }

    fn parse_receive_block(&mut self) -> Result<Vec<ChannelDef>, String> {
        let mut channels = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            let line = self.line();
            let name = if self.eat(&TokenKind::Channel) {
                self.expect_string()?
            } else if self.eat(&TokenKind::From) {
                let _source = self.expect_string()?;
                self.expect(&TokenKind::As)?;
                self.expect_string()?
            } else {
                return Err(format!("Line {}: expected channel or from", line));
            };

            self.expect(&TokenKind::LBrace)?;
            let mut source = None;
            let mut msg_type = None;
            let mut bind = None;
            let mut on_receive = None;

            loop {
                self.skip_newlines();
                if self.eat(&TokenKind::RBrace) {
                    break;
                }
                let key = self.expect_ident()?;
                self.expect(&TokenKind::Colon)?;
                match key.as_str() {
                    "source" => source = Some(self.expect_string()?),
                    "type" => msg_type = Some(self.expect_string()?),
                    "bind" => bind = Some(self.parse_expr()?),
                    "on_receive" => {
                        let v = self.parse_expr()?;
                        on_receive = Some(format!("{:?}", v));
                    }
                    _ => {
                        let _ = self.parse_expr();
                    }
                }
                self.eat(&TokenKind::Comma);
            }

            channels.push(ChannelDef {
                name,
                source,
                msg_type,
                bind,
                on_receive,
                line,
            });
        }
        Ok(channels)
    }

    // ── Brain block ───────────────────────────────────────────────────────────

    fn parse_brain_block(&mut self) -> Result<BrainBlock, String> {
        let line = self.line();
        self.expect(&TokenKind::Brain)?;
        self.expect(&TokenKind::LBrace)?;

        let mut plan = Vec::new();
        let mut execute = Vec::new();
        let mut remember = Vec::new();
        let mut communicate = Vec::new();

        loop {
            self.skip_newlines();
            match self.peek_kind().clone() {
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                TokenKind::Eof => return Err(format!("Line {}: unclosed brain block", line)),
                TokenKind::Plan => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    plan = self.parse_stmts()?;
                }
                TokenKind::Execute => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    execute = self.parse_stmts()?;
                }
                TokenKind::Remember => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    remember = self.parse_stmts()?;
                }
                TokenKind::Communicate => {
                    self.advance();
                    self.expect(&TokenKind::LBrace)?;
                    communicate = self.parse_stmts()?;
                }
                other => {
                    return Err(format!(
                        "Line {}: unexpected token in brain block: {:?}",
                        self.line(),
                        other
                    ))
                }
            }
        }

        Ok(BrainBlock {
            plan,
            execute,
            remember,
            communicate,
            line,
        })
    }

    // ── Recipe ────────────────────────────────────────────────────────────────

    fn parse_recipe(&mut self) -> Result<RecipeDef, String> {
        let line = self.line();
        self.advance();
        let name = self.expect_string()?;
        self.expect(&TokenKind::LBrace)?;

        let mut needs = Vec::new();
        let mut gives = None;
        let mut brain_opt = None;

        loop {
            self.skip_newlines();
            match self.peek_kind().clone() {
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                TokenKind::Eof => return Err(format!("Line {}: unclosed recipe '{}'", line, name)),
                TokenKind::Needs => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    loop {
                        self.skip_newlines();
                        needs.push(self.expect_ident()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                TokenKind::Gives => {
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    gives = Some(self.expect_ident()?);
                }
                TokenKind::Ident(ref s) if s == "receive" || s == "output" => {
                    let is_needs = self.peek_kind() == &TokenKind::Ident("receive".into());
                    self.advance();
                    self.expect(&TokenKind::Colon)?;
                    let v = self.expect_ident()?;
                    if is_needs {
                        needs.push(v);
                    } else {
                        gives = Some(v);
                    }
                }
                TokenKind::Brain => {
                    brain_opt = Some(self.parse_brain_block()?);
                }
                _ => {
                    let _ = self.expect_ident();
                    self.eat(&TokenKind::Colon);
                    let _ = self.parse_expr();
                    self.eat(&TokenKind::Comma);
                }
            }
        }

        let brain = brain_opt
            .ok_or_else(|| format!("Line {}: recipe '{}' missing brain block", line, name))?;
        Ok(RecipeDef {
            name,
            needs,
            gives,
            brain,
            line,
        })
    }

    // ── Objective ─────────────────────────────────────────────────────────────

    fn parse_objective(&mut self) -> Result<ObjectiveDef, String> {
        let line = self.line();
        self.advance();
        let name = self.expect_string()?;
        self.expect(&TokenKind::LBrace)?;
        self.skip_newlines();
        self.expect(&TokenKind::When)?;
        let when_expr = self.parse_expr()?;
        self.skip_newlines();
        self.expect(&TokenKind::Then)?;
        let then_action = self.parse_expr()?;
        self.skip_newlines();
        self.eat(&TokenKind::RBrace);
        Ok(ObjectiveDef {
            name,
            when_expr,
            then_action,
            line,
        })
    }

    // ── When block (Phase 2 simple syntax) ────────────────────────────────────

    fn parse_when_block(&mut self) -> Result<WhenBlock, String> {
        let line = self.line();
        self.advance(); // consume `when`
        self.skip_newlines();

        let trigger = if self.eat(&TokenKind::Started) {
            WhenTrigger::Started
        } else {
            let expr = self.parse_expr()?;
            self.skip_newlines();
            if self.eat(&TokenKind::Changes) {
                WhenTrigger::Changes(expr)
            } else {
                WhenTrigger::Expr(expr)
            }
        };

        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_stmts()?;
        Ok(WhenBlock {
            trigger,
            body,
            line,
        })
    }

    // ── Statements ────────────────────────────────────────────────────────────

    fn parse_stmts(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
                self.eat(&TokenKind::RBrace);
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.skip_newlines();

        match self.peek_kind().clone() {
            TokenKind::If => self.parse_if(),
            TokenKind::For => self.parse_for_each(),
            TokenKind::While => self.parse_while(),
            TokenKind::Break => {
                self.advance();
                Ok(Stmt::Break { line })
            }
            TokenKind::Continue => {
                self.advance();
                Ok(Stmt::Continue { line })
            }
            TokenKind::Assert => self.parse_assert(),
            TokenKind::Try => self.parse_try_catch(),
            TokenKind::Emit => self.parse_emit(),
            TokenKind::ReRun => {
                self.advance();
                Ok(Stmt::ReRun { line })
            }
            TokenKind::Escalate => {
                self.advance();
                // consume optional `to human`
                self.eat(&TokenKind::Ident("to".into()));
                self.eat(&TokenKind::Human);
                Ok(Stmt::EscalateToHuman { line })
            }
            TokenKind::Broadcast => {
                self.advance();
                let event = self.expect_string()?;
                // Broadcast may optionally have a payload (treat as emit)
                if matches!(self.peek_kind(), TokenKind::LBrace) {
                    self.advance();
                    let mut payload = Vec::new();
                    loop {
                        self.skip_newlines();
                        if self.eat(&TokenKind::RBrace) {
                            break;
                        }
                        let key = self.expect_ident()?;
                        self.expect(&TokenKind::Colon)?;
                        let value = self.parse_expr()?;
                        payload.push((key, value));
                        self.eat(&TokenKind::Comma);
                    }
                    Ok(Stmt::Emit {
                        event,
                        payload,
                        line,
                    })
                } else {
                    Ok(Stmt::Broadcast { event, line })
                }
            }
            TokenKind::Log => {
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let value = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Stmt::Log { value, line })
            }
            TokenKind::Output => {
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let value = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Stmt::Output { value, line })
            }
            TokenKind::Say => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::Say { value, line })
            }
            TokenKind::Return => {
                self.advance();
                self.skip_newlines();
                if matches!(
                    self.peek_kind(),
                    TokenKind::RBrace | TokenKind::Newline | TokenKind::Eof
                ) {
                    Ok(Stmt::Return { value: None, line })
                } else {
                    Ok(Stmt::Return {
                        value: Some(self.parse_expr()?),
                        line,
                    })
                }
            }
            TokenKind::Wait => {
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let ms = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Stmt::Wait { ms, line })
            }
            TokenKind::Serve => self.parse_serve(),
            TokenKind::Respond => self.parse_respond(),
            _ => {
                let expr = self.parse_expr()?;
                self.skip_newlines();
                if self.eat(&TokenKind::Eq) {
                    let value = self.parse_expr()?;
                    return Ok(Stmt::Assign {
                        target: expr,
                        value,
                        line,
                    });
                }
                if self.eat(&TokenKind::PlusEq) {
                    let value = self.parse_expr()?;
                    return Ok(Stmt::PlusAssign {
                        target: expr,
                        value,
                        line,
                    });
                }
                if self.eat(&TokenKind::MinusEq) {
                    let value = self.parse_expr()?;
                    return Ok(Stmt::MinusAssign {
                        target: expr,
                        value,
                        line,
                    });
                }
                if self.eat(&TokenKind::StarEq) {
                    let value = self.parse_expr()?;
                    return Ok(Stmt::MulAssign {
                        target: expr,
                        value,
                        line,
                    });
                }
                if self.eat(&TokenKind::SlashEq) {
                    let value = self.parse_expr()?;
                    return Ok(Stmt::DivAssign {
                        target: expr,
                        value,
                        line,
                    });
                }
                Ok(Stmt::Expr { expr, line })
            }
        }
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.expect(&TokenKind::While)?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_stmts()?;
        Ok(Stmt::While {
            condition,
            body,
            line,
        })
    }

    // serve on port 3000 { route GET "/" { ... } ... }
    fn parse_serve(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.expect(&TokenKind::Serve)?;
        self.eat(&TokenKind::On);
        self.eat(&TokenKind::Port);
        let port = self.parse_expr()?;
        self.expect(&TokenKind::LBrace)?;
        let mut routes = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            if matches!(self.peek_kind(), TokenKind::Eof) {
                break;
            }
            self.expect(&TokenKind::Route)?;
            let method = self.expect_ident()?.to_uppercase();
            let path = self.expect_string()?;
            self.expect(&TokenKind::LBrace)?;
            let body = self.parse_stmts()?;
            routes.push(RouteDecl {
                method,
                path,
                body,
                line: self.line(),
            });
        }
        Ok(Stmt::Serve { port, routes, line })
    }

    // respond html "..." | respond json { ... } | respond "..."
    fn parse_respond(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.expect(&TokenKind::Respond)?;
        let format = match self.peek_kind() {
            TokenKind::Ident(s) if matches!(s.as_str(), "html" | "json" | "text") => {
                let f = s.clone();
                self.advance();
                f
            }
            _ => "text".to_string(),
        };
        // optional status code: respond html 200 "..."
        let status = if matches!(self.peek_kind(), TokenKind::NumberLit(_)) {
            if let TokenKind::NumberLit(n) = self.peek_kind().clone() {
                self.advance();
                n as u16
            } else {
                200
            }
        } else {
            200
        };
        let value = self.parse_expr()?;
        Ok(Stmt::Respond {
            format,
            value,
            status,
            line,
        })
    }

    fn parse_assert(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.advance(); // consume `assert`
        let condition = self.parse_expr()?;
        self.skip_newlines();
        let message = if matches!(self.peek_kind(), TokenKind::StringLit(_)) {
            Some(self.parse_expr()?)
        } else if matches!(self.peek_kind(), TokenKind::Comma) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::Assert {
            condition,
            message,
            line,
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        let mut branches = Vec::new();
        let mut else_body = None;

        self.expect(&TokenKind::If)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::LBrace)?;
        branches.push((cond, self.parse_stmts()?));

        loop {
            self.skip_newlines();
            if !self.eat(&TokenKind::Else) {
                break;
            }
            self.skip_newlines();
            if self.eat(&TokenKind::If) {
                let cond = self.parse_expr()?;
                self.expect(&TokenKind::LBrace)?;
                branches.push((cond, self.parse_stmts()?));
            } else {
                self.expect(&TokenKind::LBrace)?;
                else_body = Some(self.parse_stmts()?);
                break;
            }
        }

        Ok(Stmt::If {
            branches,
            else_body,
            line,
        })
    }

    fn parse_for_each(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.expect(&TokenKind::For)?;
        self.eat(&TokenKind::Each); // `each` is optional
        let var = self.expect_ident()?;
        self.expect(&TokenKind::In)?;
        let iter = self.parse_expr()?;
        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_stmts()?;
        Ok(Stmt::ForEach {
            var,
            iter,
            body,
            line,
        })
    }

    fn parse_try_catch(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.expect(&TokenKind::Try)?;
        self.expect(&TokenKind::LBrace)?;
        let try_body = self.parse_stmts()?;
        self.expect(&TokenKind::Catch)?;
        let catch_var = self.expect_ident()?;
        self.expect(&TokenKind::LBrace)?;
        let catch_body = self.parse_stmts()?;
        Ok(Stmt::TryCatch {
            try_body,
            catch_var,
            catch_body,
            line,
        })
    }

    fn parse_emit(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        self.advance();
        let event = self.expect_string()?;
        let mut payload = Vec::new();
        if self.eat(&TokenKind::LBrace) {
            loop {
                self.skip_newlines();
                if self.eat(&TokenKind::RBrace) {
                    break;
                }
                let key = self.expect_ident()?;
                self.expect(&TokenKind::Colon)?;
                let value = self.parse_expr()?;
                payload.push((key, value));
                self.eat(&TokenKind::Comma);
            }
        }
        Ok(Stmt::Emit {
            event,
            payload,
            line,
        })
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_null_coalesce()
    }

    fn parse_null_coalesce(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_or()?;
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::QuestionQuestion) {
                let right = self.parse_or()?;
                left = Expr::BinOp {
                    left: Box::new(left),
                    op: BinOp::NullCoalesce,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::Or) {
                let right = self.parse_and()?;
                left = Expr::BinOp {
                    left: Box::new(left),
                    op: BinOp::Or,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_cmp()?;
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::And) {
                let right = self.parse_cmp()?;
                left = Expr::BinOp {
                    left: Box::new(left),
                    op: BinOp::And,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_add()?;
        loop {
            self.skip_newlines();
            let op = match self.peek_kind() {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::NotEq => BinOp::NotEq,
                TokenKind::Lt => BinOp::Lt,
                TokenKind::LtEq => BinOp::LtEq,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::GtEq => BinOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_add()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            self.skip_newlines();
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_mul()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_newlines();
            let op = match self.peek_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat(&TokenKind::Not) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.eat(&TokenKind::Minus) {
            // Unary minus: -expr  →  0 - expr
            let inner = self.parse_unary()?;
            return Ok(Expr::BinOp {
                left: Box::new(Expr::Num(0.0)),
                op: BinOp::Sub,
                right: Box::new(inner),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    // Check for bridge call: js.axios.get(...) or py.requests.get(...)
                    if let Expr::Ident(ref ns) = expr {
                        if self.namespaces.contains(ns.as_str()) {
                            // ns.module
                            let module = field;
                            self.skip_newlines();
                            if matches!(self.peek_kind(), TokenKind::Dot) {
                                self.advance();
                                let method = self.expect_ident()?;
                                self.skip_newlines();
                                if matches!(self.peek_kind(), TokenKind::LParen) {
                                    self.advance();
                                    let args = self.parse_args()?;
                                    expr = Expr::BridgeCall {
                                        namespace: ns.clone(),
                                        module,
                                        method,
                                        args,
                                    };
                                    continue;
                                }
                                // ns.module.method without call — field access chain
                                expr = Expr::FieldAccess {
                                    object: Box::new(Expr::FieldAccess {
                                        object: Box::new(expr),
                                        field: module,
                                    }),
                                    field: method,
                                };
                            } else {
                                expr = Expr::FieldAccess {
                                    object: Box::new(expr),
                                    field: module,
                                };
                            }
                            continue;
                        }
                    }
                    expr = Expr::FieldAccess {
                        object: Box::new(expr),
                        field,
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                TokenKind::LParen => {
                    self.advance();
                    let args = self.parse_args()?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RParen) {
                break;
            }
            args.push(self.parse_expr()?);
            if !self.eat(&TokenKind::Comma) {
                self.skip_newlines();
                self.expect(&TokenKind::RParen)?;
                break;
            }
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        self.skip_newlines();
        match self.peek_kind().clone() {
            TokenKind::StringLit(s) => {
                self.advance();
                Ok(parse_interpolated(s))
            }
            TokenKind::NumberLit(n) => {
                self.advance();
                Ok(Expr::Num(n))
            }
            TokenKind::BoolLit(b) => {
                self.advance();
                Ok(Expr::Bool(b))
            }
            TokenKind::Null => {
                self.advance();
                Ok(Expr::Null)
            }

            TokenKind::LParen => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(e)
            }

            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(&TokenKind::RBracket) {
                        break;
                    }
                    items.push(self.parse_expr()?);
                    self.eat(&TokenKind::Comma);
                }
                Ok(Expr::Array(items))
            }

            TokenKind::LBrace => {
                self.advance();
                let mut pairs = Vec::new();
                loop {
                    self.skip_newlines();
                    if self.eat(&TokenKind::RBrace) {
                        break;
                    }
                    let key = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let val = self.parse_expr()?;
                    pairs.push((key, val));
                    self.eat(&TokenKind::Comma);
                }
                Ok(Expr::Object(pairs))
            }

            // Phase 3: AI primitives
            TokenKind::Ask => {
                self.advance();
                // ask openai { ... }  OR  ask ollama:llama3 { ... }
                let mut provider = self.expect_ident()?;
                let mut model = None;
                if self.eat(&TokenKind::Colon) {
                    model = Some(self.expect_ident()?);
                }
                // Some users may write ask openai "prompt text" as shorthand
                self.skip_newlines();
                let params = if matches!(self.peek_kind(), TokenKind::LBrace) {
                    self.advance();
                    self.parse_kv_pairs()?
                } else if matches!(self.peek_kind(), TokenKind::StringLit(_)) {
                    let prompt = self.expect_string()?;
                    vec![("prompt".to_string(), Expr::Str(prompt))]
                } else {
                    Vec::new()
                };
                // Normalize provider names
                provider = normalize_provider(&provider);
                Ok(Expr::AskAI {
                    provider,
                    model,
                    params,
                })
            }

            TokenKind::Embed => {
                self.advance();
                let text = self.parse_expr()?;
                Ok(Expr::Embed {
                    text: Box::new(text),
                })
            }

            TokenKind::Infer => {
                self.advance();
                self.eat(&TokenKind::Classifier);
                self.expect(&TokenKind::LBrace)?;
                let mut input = Expr::Null;
                let mut classes = Expr::Array(Vec::new());
                loop {
                    self.skip_newlines();
                    if self.eat(&TokenKind::RBrace) {
                        break;
                    }
                    let key = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let val = self.parse_expr()?;
                    match key.as_str() {
                        "input" => input = val,
                        "classes" => classes = val,
                        _ => {}
                    }
                    self.eat(&TokenKind::Comma);
                }
                Ok(Expr::InferClassifier {
                    input: Box::new(input),
                    classes: Box::new(classes),
                })
            }

            _ => {
                let name = self.expect_ident()?;
                Ok(Expr::Ident(name))
            }
        }
    }

    /// Public: parse a single statement from the current position (for indent_parser)
    pub fn parse_one_stmt(&mut self) -> Result<Stmt, String> {
        self.skip_newlines();
        self.parse_stmt()
    }

    /// Public: parse a single expression from the current position (for indent_parser)
    pub fn parse_one_expr(&mut self) -> Result<Expr, String> {
        self.parse_expr()
    }

    fn parse_kv_pairs(&mut self) -> Result<Vec<(String, Expr)>, String> {
        let mut pairs = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            let key = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let val = self.parse_expr()?;
            pairs.push((key, val));
            self.eat(&TokenKind::Comma);
        }
        Ok(pairs)
    }
}

fn normalize_provider(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "openai" | "gpt" | "chatgpt" => "openai".into(),
        "anthropic" | "claude" => "anthropic".into(),
        "ollama" | "local" => "ollama".into(),
        other => other.to_string(),
    }
}

// ── String interpolation ──────────────────────────────────────────────────────

fn parse_interpolated(s: String) -> Expr {
    if !s.contains('{') {
        return Expr::Str(s);
    }
    let mut parts = Vec::new();
    let mut literal = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if !literal.is_empty() {
                parts.push(InterpolatedPart::Literal(std::mem::take(&mut literal)));
            }
            i += 1;
            let mut expr_src = String::new();
            while i < chars.len() && chars[i] != '}' {
                expr_src.push(chars[i]);
                i += 1;
            }
            i += 1;
            match Lexer::new(&expr_src).tokenize() {
                Ok(toks) => match Parser::new(toks).parse_expr() {
                    Ok(e) => parts.push(InterpolatedPart::Expr(e)),
                    Err(_) => parts.push(InterpolatedPart::Literal(format!("{{{}}}", expr_src))),
                },
                Err(_) => parts.push(InterpolatedPart::Literal(format!("{{{}}}", expr_src))),
            }
        } else {
            literal.push(chars[i]);
            i += 1;
        }
    }
    if !literal.is_empty() {
        parts.push(InterpolatedPart::Literal(literal));
    }
    if parts.len() == 1 {
        if let InterpolatedPart::Literal(s) = &parts[0] {
            return Expr::Str(s.clone());
        }
    }
    Expr::Interpolated(parts)
}

use crate::lexer::Lexer;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Program {
        let tokens = Lexer::new(src).tokenize().expect("lex failed");
        Parser::new(tokens).parse().expect("parse failed")
    }

    #[test]
    fn test_parse_empty_helper() {
        let p = parse(r#"helper "foo" { }"#);
        assert_eq!(p.helpers.len(), 1);
        assert_eq!(p.helpers[0].name, "foo");
    }

    #[test]
    fn test_parse_memory() {
        let p = parse(
            r#"
helper "calc" {
  remember { count = 0  name = "test" }
}"#,
        );
        assert_eq!(p.helpers[0].memory.len(), 2);
    }

    #[test]
    fn test_parse_brain() {
        let p = parse(
            r#"
helper "h" {
  brain {
    plan { plan = { action: "go" } }
    execute { log("hello") }
    remember { }
    communicate { }
  }
}"#,
        );
        assert!(p.helpers[0].brain.is_some());
    }

    #[test]
    fn test_parse_hello_world() {
        let src = std::fs::read_to_string("docs/examples/hello_world.gx").unwrap();
        let p = parse(&src);
        assert_eq!(p.helpers[0].name, "hello_world");
    }

    #[test]
    fn test_parse_agent_when() {
        let p = parse(
            r#"
agent "bot" {
  remember greeting = "hello"
  when started {
    say memory.greeting
  }
}"#,
        );
        assert_eq!(p.helpers[0].when_blocks.len(), 1);
        assert!(matches!(
            p.helpers[0].when_blocks[0].trigger,
            WhenTrigger::Started
        ));
    }

    #[test]
    fn test_parse_import() {
        let p = parse(
            r#"
use js.axios
use py.requests
helper "h" { brain { plan {} execute {} remember {} communicate {} } }
"#,
        );
        assert_eq!(p.imports.len(), 2);
        assert_eq!(p.imports[0].namespace, "js");
        assert_eq!(p.imports[0].package, "axios");
    }

    #[test]
    fn test_parse_ask_ai() {
        let p = parse(
            r#"
helper "ai" {
  brain {
    plan {}
    execute { result = ask openai { prompt: "hello", model: "gpt-4o" } }
    remember {}
    communicate {}
  }
}"#,
        );
        let brain = p.helpers[0].brain.as_ref().unwrap();
        assert_eq!(brain.execute.len(), 1);
    }
}
