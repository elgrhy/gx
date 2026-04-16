/// GX Abstract Syntax Tree

#[derive(Debug, Clone)]
pub struct Program {
    pub helpers: Vec<HelperDef>,
    pub top_level_brain: Option<BrainBlock>,
}

// ── Helper ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HelperDef {
    pub name: String,
    pub can_do: Vec<String>,
    pub memory: Vec<MemoryEntry>,
    pub receive_block: Vec<ChannelDef>,
    pub brain: Option<BrainBlock>,
    pub recipes: Vec<RecipeDef>,
    pub objectives: Vec<ObjectiveDef>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub value: Expr,
    pub line: usize,
}

// ── Brain ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BrainBlock {
    pub plan: Vec<Stmt>,
    pub execute: Vec<Stmt>,
    pub remember: Vec<Stmt>,
    pub communicate: Vec<Stmt>,
    pub line: usize,
}

// ── Recipe ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RecipeDef {
    pub name: String,
    pub needs: Vec<String>,   // input parameter names
    pub gives: Option<String>, // output variable name
    pub brain: BrainBlock,
    pub line: usize,
}

// ── Objective ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ObjectiveDef {
    pub name: String,
    pub when_expr: Expr,
    pub then_action: Expr,
    pub line: usize,
}

// ── Channel / Receive ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChannelDef {
    pub name: String,
    pub source: Option<String>,
    pub msg_type: Option<String>,
    pub bind: Option<Expr>,
    pub on_receive: Option<String>,
    pub line: usize,
}

// ── Statements ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    /// variable = expr  OR  memory.key = expr
    Assign { target: Expr, value: Expr, line: usize },
    /// variable += expr
    PlusAssign { target: Expr, value: Expr, line: usize },
    /// if expr { ... } else if expr { ... } else { ... }
    If { branches: Vec<(Expr, Vec<Stmt>)>, else_body: Option<Vec<Stmt>>, line: usize },
    /// for each name in expr { ... }
    ForEach { var: String, iter: Expr, body: Vec<Stmt>, line: usize },
    /// try { ... } catch name { ... }
    TryCatch { try_body: Vec<Stmt>, catch_var: String, catch_body: Vec<Stmt>, line: usize },
    /// emit "event" { key: value, ... }
    Emit { event: String, payload: Vec<(String, Expr)>, line: usize },
    /// broadcast "event"
    Broadcast { event: String, line: usize },
    /// log(expr)
    Log { value: Expr, line: usize },
    /// output(expr)
    Output { value: Expr, line: usize },
    /// say expr  (simple-syntax alias for output)
    Say { value: Expr, line: usize },
    /// return expr
    Return { value: Option<Expr>, line: usize },
    /// bare expression statement (function calls)
    Expr { expr: Expr, line: usize },
    /// wait(ms)
    Wait { ms: Expr, line: usize },
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    /// "hello"
    Str(String),
    /// 42.0
    Num(f64),
    /// true / false
    Bool(bool),
    /// null
    Null,
    /// identifier
    Ident(String),
    /// memory.key or obj.field
    FieldAccess { object: Box<Expr>, field: String },
    /// obj[index]
    Index { object: Box<Expr>, index: Box<Expr> },
    /// func(arg1, arg2)
    Call { callee: Box<Expr>, args: Vec<Expr> },
    /// { key: value, ... }
    Object(Vec<(String, Expr)>),
    /// [a, b, c]
    Array(Vec<Expr>),
    /// left OP right
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    /// !expr
    Not(Box<Expr>),
    /// string interpolation: "hello {name}"  → parts
    Interpolated(Vec<InterpolatedPart>),
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    And, Or,
    Concat, // + on strings
}
