/// GX Abstract Syntax Tree

#[derive(Debug, Clone)]
pub struct Program {
    pub file_imports: Vec<FileImport>,
    pub imports: Vec<ImportDecl>,
    pub functions: Vec<FunctionDef>,
    pub tools: Vec<ToolDef>,
    pub helpers: Vec<HelperDef>,
    pub top_level_brain: Option<BrainBlock>,
}

// ── File import (`import "path.gx"` or `import "path.gx" as name`) ───────────

#[derive(Debug, Clone)]
pub struct FileImport {
    pub path: String,
    /// When `Some("utils")`, functions from the file are namespaced as `utils.fn_name`.
    pub alias: Option<String>,
    pub line: usize,
}

// ── User-defined functions ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

// ── AI Tool definitions (function calling) ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub params: Vec<ToolParam>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ToolParam {
    pub name: String,
    /// JSON Schema type: "string" | "number" | "boolean" | "array" | "object"
    pub param_type: String,
    pub description: Option<String>,
    pub required: bool,
}

// ── Package import declarations (`use js.X`) ─────────────────────────────────

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub namespace: String, // "js", "py", "rust"
    pub package: String,   // "axios", "requests", "serde"
    pub line: usize,
}

// ── Helper ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HelperDef {
    pub name: String,
    pub goal: Option<String>,
    pub can_do: Vec<String>,
    pub memory: Vec<MemoryEntry>,
    pub receive_block: Vec<ChannelDef>,
    pub brain: Option<BrainBlock>,
    pub recipes: Vec<RecipeDef>,
    pub objectives: Vec<ObjectiveDef>,
    pub when_blocks: Vec<WhenBlock>,
    pub retry: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub on_error: Option<String>,
    /// Functions declared inside the agent body — registered globally when the agent is loaded.
    pub functions: Vec<FunctionDef>,
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
    Changes(Expr),
    Message(String), // when message "event_name" { }
    Cron(String),    // when cron "*/5 * * * *" { }
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
    Assign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    PlusAssign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    MinusAssign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    MulAssign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    DivAssign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
        line: usize,
    },
    ForEach {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    Break {
        line: usize,
    },
    Continue {
        line: usize,
    },
    TryCatch {
        try_body: Vec<Stmt>,
        catch_kind: Option<String>, // e.g. "NetworkError" — None means catch all
        catch_var: String,
        catch_body: Vec<Stmt>,
        line: usize,
    },
    Assert {
        condition: Expr,
        message: Option<Expr>,
        line: usize,
    },
    Emit {
        event: String,
        payload: Vec<(String, Expr)>,
        line: usize,
    },
    Broadcast {
        event: String,
        line: usize,
    },
    Log {
        value: Expr,
        line: usize,
    },
    Output {
        value: Expr,
        line: usize,
    },
    Say {
        value: Expr,
        line: usize,
    },
    Return {
        value: Option<Expr>,
        line: usize,
    },
    Expr {
        expr: Expr,
        line: usize,
    },
    Wait {
        ms: Expr,
        line: usize,
    },
    ReRun {
        line: usize,
    },
    EscalateToHuman {
        line: usize,
    },
    Serve {
        port: Expr,
        routes: Vec<RouteDecl>,
        line: usize,
    },
    // Phase 5: send "event" to "agent" with { key: val }
    SendMessage {
        agent_name: Expr,
        event: String,
        data: Vec<(String, Expr)>,
        line: usize,
    },
    // v0.2.0: opinionated sugar
    Think {
        prompt: Expr,
        model: Option<String>,
        temperature: Option<Expr>,
        min_confidence: Option<Expr>,
        into_var: String,
        line: usize,
    },
    Observe {
        bindings: Vec<(String, Expr)>,
        line: usize,
    },
    Act {
        body: Vec<Stmt>,
        line: usize,
    },
    LoopUntil {
        condition: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    RepeatTimes {
        count: Expr,
        var: Option<String>,
        body: Vec<Stmt>,
        line: usize,
    },
    Parallel {
        branches: Vec<Vec<Stmt>>,
        line: usize,
    },
    Respond {
        format: String, // "html" | "json" | "text"
        value: Expr,
        status: u16,
        line: usize,
    },
    /// await { a: expr, b: expr } — run all branches concurrently, collect results as object
    Await {
        bindings: Vec<(String, Expr)>,
        into_var: String,
        line: usize,
    },
}

// ── HTTP route declaration ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RouteDecl {
    pub method: String, // "GET" | "POST" | "PUT" | "DELETE" | "ANY"
    pub path: String,
    pub body: Vec<Stmt>,
    pub line: usize,
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    Ident(String),
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Object(Vec<(String, Expr)>),
    Array(Vec<Expr>),
    BinOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    Not(Box<Expr>),
    Interpolated(Vec<InterpolatedPart>),
    // AI primitives
    AskAI {
        provider: String,
        model: Option<String>,
        params: Vec<(String, Expr)>,
    },
    Embed {
        text: Box<Expr>,
    },
    InferClassifier {
        input: Box<Expr>,
        classes: Box<Expr>,
    },
    // Bridge call: js.axios.get("url")
    BridgeCall {
        namespace: String,
        module: String,
        method: String,
        args: Vec<Expr>,
    },
    // Phase 5: spawn agent "name" with { key: val } [timeout Ns]
    CallAgent {
        name: Box<Expr>,
        input: Vec<(String, Expr)>,
        timeout_ms: Option<Box<Expr>>, // optional timeout in milliseconds
    },
    // Anonymous function: fn(params) { body }
    Lambda {
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    // Range for slice indexing: expr[start..end]
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    // Parallel named results: parallel { a: expr, b: expr }
    ParallelMap(Vec<(String, Expr)>),
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Concat,
    NullCoalesce, // ??
    Pipe,         // |>
}
