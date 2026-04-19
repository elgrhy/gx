/// GX Lexer — turns raw source text into a flat list of tokens.

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Ident(String),
    StringLit(String),
    NumberLit(f64),
    BoolLit(bool),
    Null,

    // Keywords
    Helper,
    Agent,
    Brain,
    Plan,
    Execute,
    Remember,
    Communicate,
    Memory,
    Receive,
    Channel,
    Emit,
    Broadcast,
    Recipe,
    Objective,
    Needs,
    Gives,
    CanDo,
    When,
    Then,
    Try,
    Catch,
    For,
    Each,
    In,
    If,
    Else,
    Return,
    Output,
    Log,
    Say,
    Use,
    From,
    As,
    On,
    Bind,
    Source,
    Type,
    Do,
    Wait,
    Assign,
    Spawn,
    Count,
    Push,
    Not,
    And,
    Or,
    // Phase 2 — simple syntax
    Started,
    ReRun,
    Escalate,
    Human,
    Changes,
    // Phase 3 — AI primitives
    Ask,
    Embed,
    Infer,
    Classifier,
    // Phase 5 — user-defined functions and file imports
    Function,
    Import,
    // Control flow additions
    While,
    Break,
    Continue,
    Assert,
    // HTTP server
    Serve,
    Route,
    Respond,
    Port,
    // Phase 5 — multi-agent orchestration
    With,
    To,
    Message,
    Call,
    Pipe, // |>

    // Operators / punctuation
    LBrace,           // {
    RBrace,           // }
    LBracket,         // [
    RBracket,         // ]
    LParen,           // (
    RParen,           // )
    Colon,            // :
    Comma,            // ,
    Dot,              // .
    Eq,               // =
    EqEq,             // ==
    NotEq,            // !=
    Lt,               // <
    LtEq,             // <=
    Gt,               // >
    GtEq,             // >=
    Plus,             // +
    PlusEq,           // +=
    Minus,            // -
    MinusEq,          // -=
    Star,             // *
    StarEq,           // *=
    Slash,            // /
    SlashEq,          // /=
    Percent,          // %
    Arrow,            // ->
    DotDot,           // ..
    QuestionQuestion, // ??

    // Structure
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

impl Token {
    fn new(kind: TokenKind, line: usize, col: usize) -> Self {
        Token { kind, line, col }
    }
}

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.source.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.source.get(self.pos).copied();
        if let Some(c) = ch {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') => {
                    self.advance();
                }
                Some('/') if self.peek_next() == Some('/') => {
                    // line comment
                    while self.peek().map(|c| c != '\n').unwrap_or(false) {
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn read_string(&mut self) -> Result<String, String> {
        let line = self.line;
        // consume opening quote
        self.advance();
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err(format!("Unterminated string at line {}", line)),
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some(c) => {
                        s.push('\\');
                        s.push(c);
                    }
                    None => return Err(format!("Unterminated escape at line {}", line)),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn read_number(&mut self) -> f64 {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // Handle `re-run` as a single token
        if s == "re" && self.peek() == Some('-') {
            // Peek ahead to check if it's "re-run"
            let saved_pos = self.pos;
            let saved_line = self.line;
            let saved_col = self.col;
            self.advance(); // consume '-'
            let rest = self.read_ident();
            if rest == "run" {
                return "re-run".to_string();
            }
            // Not "re-run", restore position
            self.pos = saved_pos;
            self.line = saved_line;
            self.col = saved_col;
        }
        s
    }

    fn keyword_or_ident(s: String) -> TokenKind {
        match s.as_str() {
            "helper" => TokenKind::Helper,
            "agent" => TokenKind::Agent,
            "brain" => TokenKind::Brain,
            "plan" => TokenKind::Plan,
            "execute" => TokenKind::Execute,
            "remember" => TokenKind::Remember,
            "communicate" => TokenKind::Communicate,
            "memory" => TokenKind::Memory,
            "receive" => TokenKind::Receive,
            "channel" => TokenKind::Channel,
            "emit" => TokenKind::Emit,
            "broadcast" => TokenKind::Broadcast,
            "recipe" => TokenKind::Recipe,
            "objective" => TokenKind::Objective,
            "needs" => TokenKind::Needs,
            "gives" => TokenKind::Gives,
            "can_do" => TokenKind::CanDo,
            "when" => TokenKind::When,
            "then" => TokenKind::Then,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "for" => TokenKind::For,
            "each" => TokenKind::Each,
            "in" => TokenKind::In,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "return" => TokenKind::Return,
            "output" => TokenKind::Output,
            "log" => TokenKind::Log,
            "say" => TokenKind::Say,
            "use" => TokenKind::Use,
            "from" => TokenKind::From,
            "as" => TokenKind::As,
            "on" => TokenKind::On,
            "bind" => TokenKind::Bind,
            "source" => TokenKind::Source,
            "type" => TokenKind::Type,
            "do" => TokenKind::Do,
            "wait" => TokenKind::Wait,
            "assign" => TokenKind::Assign,
            "spawn" => TokenKind::Spawn,
            "count" => TokenKind::Count,
            "push" => TokenKind::Push,
            "not" => TokenKind::Not,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "re-run" => TokenKind::ReRun,
            "started" => TokenKind::Started,
            "escalate" => TokenKind::Escalate,
            "human" => TokenKind::Human,
            "changes" => TokenKind::Changes,
            "ask" => TokenKind::Ask,
            "embed" => TokenKind::Embed,
            "infer" => TokenKind::Infer,
            "classifier" => TokenKind::Classifier,
            "function" => TokenKind::Function,
            "import" => TokenKind::Import,
            "while" => TokenKind::While,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "assert" => TokenKind::Assert,
            "serve" => TokenKind::Serve,
            "with" => TokenKind::With,
            "to" => TokenKind::To,
            "message" => TokenKind::Message,
            "call" => TokenKind::Call,
            "route" => TokenKind::Route,
            "respond" => TokenKind::Respond,
            "port" => TokenKind::Port,
            "true" => TokenKind::BoolLit(true),
            "false" => TokenKind::BoolLit(false),
            "null" => TokenKind::Null,
            _ => TokenKind::Ident(s),
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();
            let line = self.line;
            let col = self.col;

            match self.peek() {
                None => {
                    tokens.push(Token::new(TokenKind::Eof, line, col));
                    break;
                }
                Some('\n') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Newline, line, col));
                }
                Some('"') => {
                    let s = self.read_string()?;
                    tokens.push(Token::new(TokenKind::StringLit(s), line, col));
                }
                Some(c) if c.is_ascii_digit() => {
                    let n = self.read_number();
                    tokens.push(Token::new(TokenKind::NumberLit(n), line, col));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    let ident = self.read_ident();
                    let kind = Self::keyword_or_ident(ident);
                    tokens.push(Token::new(kind, line, col));
                }
                Some('{') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LBrace, line, col));
                }
                Some('}') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RBrace, line, col));
                }
                Some('[') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LBracket, line, col));
                }
                Some(']') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RBracket, line, col));
                }
                Some('(') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LParen, line, col));
                }
                Some(')') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RParen, line, col));
                }
                Some(':') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Colon, line, col));
                }
                Some(',') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Comma, line, col));
                }
                Some('.') => {
                    self.advance();
                    if self.peek() == Some('.') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::DotDot, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Dot, line, col));
                    }
                }
                Some('+') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::PlusEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Plus, line, col));
                    }
                }
                Some('-') => {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::Arrow, line, col));
                    } else if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::MinusEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Minus, line, col));
                    }
                }
                Some('*') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::StarEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Star, line, col));
                    }
                }
                Some('/') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::SlashEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Slash, line, col));
                    }
                }
                Some('%') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Percent, line, col));
                }
                Some('=') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::EqEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Eq, line, col));
                    }
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::NotEq, line, col));
                    } else {
                        return Err(format!("Unexpected '!' at line {}, col {}", line, col));
                    }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::LtEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Lt, line, col));
                    }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::GtEq, line, col));
                    } else {
                        tokens.push(Token::new(TokenKind::Gt, line, col));
                    }
                }
                Some(';') => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Newline, line, col));
                }
                Some('|') => {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::Pipe, line, col));
                    } else {
                        return Err(format!(
                            "Unexpected '|' at line {}, col {} (did you mean '|>'?)",
                            line, col
                        ));
                    }
                }
                Some('?') => {
                    self.advance();
                    if self.peek() == Some('?') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::QuestionQuestion, line, col));
                    } else {
                        return Err(format!(
                            "Unexpected '?' at line {}, col {} (did you mean '??'?)",
                            line, col
                        ));
                    }
                }
                Some(c) => {
                    return Err(format!(
                        "Unexpected character '{}' at line {}, col {}",
                        c, line, col
                    ));
                }
            }
        }

        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(src: &str) -> Vec<TokenKind> {
        Lexer::new(src)
            .tokenize()
            .unwrap()
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn test_keywords() {
        let kinds = tok("helper brain plan execute remember communicate");
        assert!(matches!(kinds[0], TokenKind::Helper));
        assert!(matches!(kinds[1], TokenKind::Brain));
        assert!(matches!(kinds[2], TokenKind::Plan));
        assert!(matches!(kinds[3], TokenKind::Execute));
        assert!(matches!(kinds[4], TokenKind::Remember));
        assert!(matches!(kinds[5], TokenKind::Communicate));
    }

    #[test]
    fn test_string_literal() {
        let kinds = tok("\"hello world\"");
        assert_eq!(kinds[0], TokenKind::StringLit("hello world".into()));
    }

    #[test]
    fn test_number() {
        let kinds = tok("42");
        assert_eq!(kinds[0], TokenKind::NumberLit(42.0));
    }

    #[test]
    fn test_operators() {
        let kinds = tok("= == != += + - * / < > <= >=");
        assert!(matches!(kinds[0], TokenKind::Eq));
        assert!(matches!(kinds[1], TokenKind::EqEq));
        assert!(matches!(kinds[2], TokenKind::NotEq));
        assert!(matches!(kinds[3], TokenKind::PlusEq));
    }

    #[test]
    fn test_booleans() {
        let kinds = tok("true false null");
        assert_eq!(kinds[0], TokenKind::BoolLit(true));
        assert_eq!(kinds[1], TokenKind::BoolLit(false));
        assert_eq!(kinds[2], TokenKind::Null);
    }

    #[test]
    fn test_comment_skipped() {
        let kinds = tok("helper // this is a comment\nbrain");
        assert!(matches!(kinds[0], TokenKind::Helper));
        // newline may appear, brain must be somewhere
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Brain)));
    }
}
