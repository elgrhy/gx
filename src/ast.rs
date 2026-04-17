/// GX Abstract Syntax Tree

#[derive(Debug, Clone)]
pub struct Program {
    pub imports: Vec<ImportDecl>,
    pub helpers: Vec<HelperDef>,
    pub top_level_brain: Option<BrainBlock>,
}

// ── Import declarations ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub namespace: String,  // "js", "py", "rust"
    pub package: String,    // "axios", "requests", "serde"
    pub line: usize,
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
    pub when_blocks: Vec<WhenBlock>,   // Phase 2: simple syntax
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub value: Expr,
    pub line: usize,
}

// ── Phase 2: When blocks ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WhenBlock {
    pub trigger: WhenTrigger,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum WhenTrigger {
    Started,
    Expr(Expr),
    Changes(Expr),  // when X changes
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
    pub needs: Vec<String>,
    pub gives: Option<String>,
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
    Assign { target: Expr, value: Expr, line: usize },
    PlusAssign { target: Expr, value: Expr, line: usize },
    If { branches: Vec<(Expr, Vec<Stmt>)>, else_body: Option<Vec<Stmt>>, line: usize },
    ForEach { var: String, iter: Expr, body: Vec<Stmt>, line: usize },
    TryCatch { try_body: Vec<Stmt>, catch_var: String, catch_body: Vec<Stmt>, line: usize },
    Emit { event: String, payload: Vec<(String, Expr)>, line: usize },
    Broadcast { event: String, line: usize },
    Log { value: Expr, line: usize },
    Output { value: Expr, line: usize },
    Say { value: Expr, line: usize },
    Return { value: Option<Expr>, line: usize },
    Expr { expr: Expr, line: usize },
    Wait { ms: Expr, line: usize },
    // Phase 2
    ReRun { line: usize },
    EscalateToHuman { line: usize },
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    Ident(String),
    FieldAccess { object: Box<Expr>, field: String },
    Index { object: Box<Expr>, index: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Object(Vec<(String, Expr)>),
    Array(Vec<Expr>),
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    Not(Box<Expr>),
    Interpolated(Vec<InterpolatedPart>),
    // Phase 3: AI primitives
    AskAI {
        provider: String,             // "openai", "anthropic", "ollama"
        model: Option<String>,        // optional model override e.g. "gpt-4o"
        params: Vec<(String, Expr)>,  // { prompt: "...", context: ..., ... }
    },
    Embed { text: Box<Expr> },
    InferClassifier { input: Box<Expr>, classes: Box<Expr> },
    // Phase 4: bridge call  js.axios.get("url")
    BridgeCall {
        namespace: String,  // "js" or "py"
        module: String,     // "axios"
        method: String,     // "get"
        args: Vec<Expr>,
    },
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
    Concat,
}
