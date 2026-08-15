//! seed.rs — the Corros bootstrap seed.
//!
//! Every language needs a first compiler written in *some* other language —
//! rustc's first compiler was written in OCaml, and GCC was written in C.
//! This file is that seed for Corros: a small tree-walking interpreter that
//! runs just enough of Corros to boot the real interpreter, which lives in
//! Corros itself:
//!
//!   - `src/compiler.cor`  — the Corros lexer + bytecode compiler
//!   - `src/vm.cor`        — the Corros virtual machine
//!   - `src/prelude.cor`   — the Corros standard library
//!
//! `corros hello.cor` therefore does: the seed runs the Corros compiler (which
//! compiles hello.cor to bytecode), then the seed runs the Corros VM (which
//! executes that bytecode). The language you use *is* the Corros-written one;
//! the seed only knows the disciplined subset the self-hosted files are
//! written in (craft/forge/whilst/when/each, lists, strings, numbers, ranges,
//! maps, and the builtins).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::lexer::{Token, TokenKind};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<HashMap<u64, (Value, Value)>>>),
    Range { start: f64, end: f64, inclusive: bool },
    Fn(Rc<FnValue>),
    Native(&'static str),
}

#[derive(Debug)]
pub struct FnValue {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub env: Rc<Env>,
}

pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Nil => "nil",
        Value::Bool(_) => "bool",
        Value::Num(_) => "num",
        Value::Str(_) => "string",
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Range { .. } => "range",
        Value::Fn(_) | Value::Native(_) => "function",
    }
}

pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Nil | Value::Bool(false) => false,
        Value::Num(n) => *n != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(l) => !l.borrow().is_empty(),
        Value::Map(m) => !m.borrow().is_empty(),
        _ => true,
    }
}

pub fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::List(x), Value::List(y)) => Rc::ptr_eq(x, y),
        (Value::Map(x), Value::Map(y)) => Rc::ptr_eq(x, y),
        (
            Value::Range { start: a1, end: a2, inclusive: a3 },
            Value::Range { start: b1, end: b2, inclusive: b3 },
        ) => a1 == b1 && a2 == b2 && a3 == b3,
        (Value::Fn(x), Value::Fn(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => x == y,
        _ => false,
    }
}

/// Hash a map key (nil, bool, num, string only).
pub(crate) fn hash_key(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match v {
        Value::Nil => 0u8.hash(&mut hasher),
        Value::Bool(b) => {
            1u8.hash(&mut hasher);
            b.hash(&mut hasher);
        }
        Value::Num(n) => {
            2u8.hash(&mut hasher);
            let bits = if *n == 0.0 { 0.0f64.to_bits() } else { n.to_bits() };
            bits.hash(&mut hasher);
        }
        Value::Str(s) => {
            3u8.hash(&mut hasher);
            s.hash(&mut hasher);
        }
        _ => panic!("non-hashable value used as a map key"),
    }
    hasher.finish()
}

/// Format a number the way Corros prints it: integers without a trailing ".0".
pub fn format_num(n: f64) -> String {
    if n.is_nan() {
        return "nan".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    n.to_string()
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

/// The quoted form used for map keys and error messages.
pub fn repr(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("\"{}\"", escape_str(s)),
        _ => to_string(v),
    }
}

pub fn to_string(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => format_num(*n),
        Value::Str(s) => s.to_string(),
        Value::List(items) => {
            let parts: Vec<String> = items.borrow().iter().map(to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Map(entries) => {
            let parts: Vec<String> = entries
                .borrow()
                .values()
                .map(|(k, v)| format!("{}: {}", repr(k), to_string(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Range { start, end, inclusive } => format!(
            "{}{}{}",
            format_num(*start),
            if *inclusive { "..=" } else { ".." },
            format_num(*end)
        ),
        Value::Fn(f) => format!("<fn {}>", f.name),
        Value::Native(name) => format!("<native {}>", name),
    }
}

pub fn range_len(start: f64, end: f64, inclusive: bool) -> usize {
    let count = if inclusive { end - start + 1.0 } else { end - start };
    if count <= 0.0 {
        0
    } else {
        count as usize
    }
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Stmt {
    Craft { name: String, params: Vec<String>, body: Vec<Stmt> },
    Forge { name: String, init: Expr },
    Expr(Expr),
    Whilst { cond: Expr, body: Vec<Stmt> },
    When { branches: Vec<(Expr, Vec<Stmt>)>, else_body: Option<Vec<Stmt>> },
    Each { var: String, iter: Expr, body: Vec<Stmt> },
    Break,
    Onward,
    Return(Option<Expr>),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Nil,
    Var(String),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Index { container: Box<Expr>, key: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Assign { target: AssignTarget, op: Option<BinOp>, value: Box<Expr> },
    Fn { params: Vec<String>, body: Vec<Stmt> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Power,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Eq,
    Neq,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Name(String),
    Index { container: Box<Expr>, key: Box<Expr> },
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn advance(&mut self) -> TokenKind {
        let kind = self.tokens[self.pos].kind.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        kind
    }

    fn check(&self, k: &TokenKind) -> bool {
        self.peek() == k
    }

    fn match_kind(&mut self, k: &TokenKind) -> bool {
        if self.check(k) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, k: &TokenKind, what: &str) -> Result<(), String> {
        if self.check(k) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "expected {} but found {:?}",
                what,
                self.peek()
            ))
        }
    }

    fn identifier(&mut self, what: &str) -> Result<String, String> {
        match self.peek().clone() {
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(format!("expected {} but found {:?}", what, other)),
        }
    }

    fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>, String> {
        let mut p = Parser { tokens, pos: 0 };
        let mut stmts = Vec::new();
        while !p.check(&TokenKind::Eof) {
            p.skip_semicolons();
            if p.check(&TokenKind::Eof) {
                break;
            }
            stmts.push(p.statement()?);
            p.skip_semicolons();
        }
        Ok(stmts)
    }

    fn skip_semicolons(&mut self) {
        while self.check(&TokenKind::Semicolon) {
            self.advance();
        }
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        match self.peek() {
            TokenKind::Craft => self.craft_stmt(),
            TokenKind::Forge => self.forge_stmt(),
            TokenKind::Whilst => self.whilst_stmt(),
            TokenKind::When => self.when_stmt(),
            TokenKind::Each => self.each_stmt(),
            TokenKind::Break => {
                self.advance();
                Ok(Stmt::Break)
            }
            TokenKind::Onward => {
                self.advance();
                Ok(Stmt::Onward)
            }
            TokenKind::Return => {
                self.advance();
                if self.check(&TokenKind::RBrace)
                    || self.check(&TokenKind::Eof)
                    || self.check(&TokenKind::Semicolon)
                {
                    Ok(Stmt::Return(None))
                } else {
                    let e = self.expression()?;
                    Ok(Stmt::Return(Some(e)))
                }
            }
            _ => {
                let e = self.expression()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            self.skip_semicolons();
            if self.check(&TokenKind::RBrace) || self.check(&TokenKind::Eof) {
                break;
            }
            stmts.push(self.statement()?);
            self.skip_semicolons();
        }
        self.expect(&TokenKind::RBrace, "'}'")?;
        Ok(stmts)
    }

    fn craft_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // craft
        let name = self.identifier("a function name after 'craft'")?;
        self.expect(&TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        while !self.check(&TokenKind::RParen) {
            params.push(self.identifier("a parameter name")?);
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        let body = self.block()?;
        Ok(Stmt::Craft { name, params, body })
    }

    fn forge_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // forge
        let name = self.identifier("a variable name after 'forge'")?;
        self.expect(&TokenKind::Equal, "'='")?;
        let init = self.expression()?;
        Ok(Stmt::Forge { name, init })
    }

    fn whilst_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // whilst
        let cond = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::Whilst { cond, body })
    }

    fn when_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // when
        let cond = self.expression()?;
        let body = self.block()?;
        let mut branches = vec![(cond, body)];
        let mut else_body = None;
        if self.match_kind(&TokenKind::Else) {
            if self.check(&TokenKind::When) {
                // else when ... — fold into the branches
                let nested = self.when_stmt()?;
                match nested {
                    Stmt::When { branches: b, else_body: e } => {
                        branches.extend(b);
                        else_body = e;
                    }
                    _ => unreachable!(),
                }
            } else {
                let body = self.block()?;
                else_body = Some(body);
            }
        }
        Ok(Stmt::When { branches, else_body })
    }

    fn each_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // each
        let var = self.identifier("a loop variable after 'each'")?;
        self.expect(&TokenKind::In, "'in'")?;
        let iter = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::Each { var, iter, body })
    }

    // --- expressions -------------------------------------------------------

    fn expression(&mut self) -> Result<Expr, String> {
        let left = self.or_expr()?;
        let op = match self.peek() {
            TokenKind::Equal => Some(None),
            TokenKind::PlusEqual => Some(Some(BinOp::Add)),
            TokenKind::MinusEqual => Some(Some(BinOp::Sub)),
            TokenKind::StarEqual => Some(Some(BinOp::Mul)),
            TokenKind::SlashEqual => Some(Some(BinOp::Div)),
            TokenKind::PercentEqual => Some(Some(BinOp::Mod)),
            TokenKind::PowerEqual => Some(Some(BinOp::Power)),
            _ => None,
        };
        if let Some(compound) = op {
            self.advance();
            let value = self.expression()?;
            let target = lvalue(left)?;
            Ok(Expr::Assign { target, op: compound, value: Box::new(value) })
        } else {
            Ok(left)
        }
    }

    fn or_expr(&mut self) -> Result<Expr, String> {
        let mut e = self.and_expr()?;
        while self.match_kind(&TokenKind::Or) {
            let right = self.and_expr()?;
            e = Expr::Binary { op: BinOp::Or, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn and_expr(&mut self) -> Result<Expr, String> {
        let mut e = self.equality()?;
        while self.match_kind(&TokenKind::And) {
            let right = self.equality()?;
            e = Expr::Binary { op: BinOp::And, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn equality(&mut self) -> Result<Expr, String> {
        let mut e = self.comparison()?;
        loop {
            let op = if self.match_kind(&TokenKind::EqualEqual) {
                BinOp::Eq
            } else if self.match_kind(&TokenKind::BangEqual) {
                BinOp::Neq
            } else {
                break;
            };
            let right = self.comparison()?;
            e = Expr::Binary { op, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut e = self.term()?;
        loop {
            let op = if self.match_kind(&TokenKind::Less) {
                BinOp::Less
            } else if self.match_kind(&TokenKind::LessEqual) {
                BinOp::LessEqual
            } else if self.match_kind(&TokenKind::Greater) {
                BinOp::Greater
            } else if self.match_kind(&TokenKind::GreaterEqual) {
                BinOp::GreaterEqual
            } else {
                break;
            };
            let right = self.term()?;
            e = Expr::Binary { op, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut e = self.factor()?;
        loop {
            let op = if self.match_kind(&TokenKind::Plus) {
                BinOp::Add
            } else if self.match_kind(&TokenKind::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let right = self.factor()?;
            e = Expr::Binary { op, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn factor(&mut self) -> Result<Expr, String> {
        let mut e = self.unary()?;
        loop {
            let op = if self.match_kind(&TokenKind::Star) {
                BinOp::Mul
            } else if self.match_kind(&TokenKind::Slash) {
                BinOp::Div
            } else if self.match_kind(&TokenKind::Percent) {
                BinOp::Mod
            } else {
                break;
            };
            let right = self.unary()?;
            e = Expr::Binary { op, left: Box::new(e), right: Box::new(right) };
        }
        Ok(e)
    }

    fn power(&mut self) -> Result<Expr, String> {
        let left = self.postfix()?;
        if self.match_kind(&TokenKind::Power) {
            let right = self.unary()?;
            Ok(Expr::Binary { op: BinOp::Power, left: Box::new(left), right: Box::new(right) })
        } else {
            Ok(left)
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.match_kind(&TokenKind::Bang) {
            let operand = self.unary()?;
            Ok(Expr::Unary { op: UnaryOp::Not, operand: Box::new(operand) })
        } else if self.match_kind(&TokenKind::Minus) {
            let operand = self.unary()?;
            Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(operand) })
        } else {
            self.power()
        }
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            if self.match_kind(&TokenKind::LParen) {
                let mut args = Vec::new();
                while !self.check(&TokenKind::RParen) {
                    args.push(self.expression()?);
                    if !self.match_kind(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen, "')'")?;
                e = Expr::Call { callee: Box::new(e), args };
            } else if self.match_kind(&TokenKind::LBracket) {
                let key = self.expression()?;
                self.expect(&TokenKind::RBracket, "']'")?;
                e = Expr::Index { container: Box::new(e), key: Box::new(key) };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            TokenKind::Number(n) => {
                self.advance();
                Ok(Expr::Num(n))
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            TokenKind::Nil => {
                self.advance();
                Ok(Expr::Nil)
            }
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(Expr::Var(name))
            }
            TokenKind::LParen => {
                self.advance();
                let e = self.expression()?;
                self.expect(&TokenKind::RParen, "')'")?;
                Ok(e)
            }
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.check(&TokenKind::RBracket) {
                    items.push(self.expression()?);
                    if !self.match_kind(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RBracket, "']'")?;
                Ok(Expr::List(items))
            }
            TokenKind::LBrace => {
                self.advance();
                let mut entries = Vec::new();
                while !self.check(&TokenKind::RBrace) {
                    let k = self.expression()?;
                    self.expect(&TokenKind::Colon, "':' after map key")?;
                    let v = self.expression()?;
                    entries.push((k, v));
                    if !self.match_kind(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace, "'}'")?;
                Ok(Expr::Map(entries))
            }
            TokenKind::Craft => {
                // Anonymous craft expression: craft(params) { body }
                self.advance();
                self.expect(&TokenKind::LParen, "'('")?;
                let mut params = Vec::new();
                while !self.check(&TokenKind::RParen) {
                    params.push(self.identifier("a parameter name")?);
                    if !self.match_kind(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen, "')'")?;
                let body = self.block()?;
                Ok(Expr::Fn { params, body })
            }
            other => Err(format!("unexpected token {:?} in expression", other)),
        }
    }
}

fn lvalue(e: Expr) -> Result<AssignTarget, String> {
    match e {
        Expr::Var(name) => Ok(AssignTarget::Name(name)),
        Expr::Index { container, key } => {
            Ok(AssignTarget::Index { container, key })
        }
        _ => Err("invalid assignment target".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Environments
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Env {
    pub vars: RefCell<HashMap<String, Value>>,
    pub parent: Option<Rc<Env>>,
}

impl Env {
    fn child(parent: &Rc<Env>) -> Rc<Env> {
        Rc::new(Env { vars: RefCell::new(HashMap::new()), parent: Some(parent.clone()) })
    }

    fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    /// Assign to an existing binding anywhere up the chain. Returns false when
    /// the name is not bound anywhere (the caller then creates a global).
    fn set_existing(&self, name: &str, value: Value) -> bool {
        if self.vars.borrow().contains_key(name) {
            self.vars.borrow_mut().insert(name.to_string(), value);
            return true;
        }
        match &self.parent {
            Some(p) => p.set_existing(name, value),
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// The interpreter
// ---------------------------------------------------------------------------

enum Control {
    Normal,
    Break,
    Onward,
    Return(Value),
}

pub struct Interpreter {
    root: Rc<Env>,
    pub output: Vec<String>,
    pub echo: bool,
    args: Vec<String>,
    start: Instant,
    /// Corros call stack, for debugging: function names of active calls.
    call_stack: Vec<String>,
}

impl Interpreter {
    pub fn new(args: Vec<String>) -> Self {
        let root = Rc::new(Env::default());
        let mut interp = Interpreter {
            root: root.clone(),
            output: Vec::new(),
            echo: false,
            args,
            start: Instant::now(),
            call_stack: Vec::new(),
        };
        interp.install_builtins();
        // `args` — the command-line arguments, as seen by Corros programs.
        let args_list = Value::List(Rc::new(RefCell::new(
            interp.args.iter().map(|s| Value::Str(s.as_str().into())).collect(),
        )));
        root.vars.borrow_mut().insert("args".to_string(), args_list);
        interp
    }

    fn install_builtins(&mut self) {
        let names = [
            "speak", "hear", "size", "nature", "str", "num", "int", "bool", "abs",
            "root", "least", "greatest", "tick", "span", "vouch", "flaw", "read",
            "readlines", "shove", "yank", "file_exists", "mcall",
        ];
        let mut vars = self.root.vars.borrow_mut();
        for name in names {
            vars.insert(name.to_string(), Value::Native(name));
        }
    }

    /// Lex, parse, and run a file of Corros source. Returns the collected
    /// `speak` output (or an error message).
    pub fn run_file(&mut self, path: &std::path::Path) -> Result<Vec<String>, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("corros: could not open '{}': {}", path.display(), e))?;
        self.run_source_str(&source, &path.display().to_string())
    }

    pub fn run_source_str(&mut self, source: &str, file: &str) -> Result<Vec<String>, String> {
        let tokens = crate::lexer::lex(source, file).map_err(|e| e.message)?;
        let stmts = Parser::parse(tokens)?;
        match self.exec(&self.root.clone(), &stmts) {
            Ok(control) => {
                if let Control::Return(_) = control {
                    // Top-level return: ignore the value.
                }
                Ok(self.output.clone())
            }
            Err(e) => Err(self.with_trace(e)),
        }
    }

    // --- statements --------------------------------------------------------

    fn exec(&mut self, env: &Rc<Env>, stmts: &[Stmt]) -> Result<Control, String> {
        for stmt in stmts {
            match stmt {
                Stmt::Craft { name, params, body } => {
                    let f = Value::Fn(Rc::new(FnValue {
                        name: name.clone(),
                        params: params.clone(),
                        body: body.clone(),
                        env: env.clone(),
                    }));
                    env.vars.borrow_mut().insert(name.clone(), f);
                }
                Stmt::Forge { name, init } => {
                    let v = self.eval(env, init)?;
                    env.vars.borrow_mut().insert(name.clone(), v);
                }
                Stmt::Expr(e) => {
                    self.eval(env, e)?;
                }
                Stmt::Whilst { cond, body } => {
                    loop {
                        let c = self.eval(env, cond)?;
                        if !is_truthy(&c) {
                            break;
                        }
                        let scope = Env::child(env);
                        match self.exec(&scope, body)? {
                            Control::Break => break,
                            Control::Return(v) => return Ok(Control::Return(v)),
                            _ => {}
                        }
                    }
                }
                Stmt::When { branches, else_body } => {
                    let mut taken = false;
                    for (cond, body) in branches {
                        let c = self.eval(env, cond)?;
                        if is_truthy(&c) {
                            taken = true;
                            let scope = Env::child(env);
                            let r = self.exec(&scope, body)?;
                            if !matches!(r, Control::Normal) {
                                return Ok(r);
                            }
                            break;
                        }
                    }
                    if !taken {
                        if let Some(body) = else_body {
                            let scope = Env::child(env);
                            let r = self.exec(&scope, body)?;
                            if !matches!(r, Control::Normal) {
                                return Ok(r);
                            }
                        }
                    }
                }
                Stmt::Each { var, iter, body } => {
                    let it = self.eval(env, iter)?;
                    let items: Vec<Value> = match &it {
                        Value::List(l) => l.borrow().clone(),
                        Value::Range { start, end, inclusive } => {
                            let len = range_len(*start, *end, *inclusive);
                            (0..len).map(|i| Value::Num(start + i as f64)).collect()
                        }
                        Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string().into())).collect(),
                        _ => {
                            return Err(format!(
                                "cannot iterate a value of type {}",
                                type_name(&it)
                            ));
                        }
                    };
                    for item in items {
                        let scope = Env::child(env);
                        scope.vars.borrow_mut().insert(var.clone(), item);
                        match self.exec(&scope, body)? {
                            Control::Break => break,
                            Control::Return(v) => return Ok(Control::Return(v)),
                            _ => {}
                        }
                    }
                }
                Stmt::Break => return Ok(Control::Break),
                Stmt::Onward => return Ok(Control::Onward),
                Stmt::Return(e) => {
                    let v = match e {
                        Some(e) => self.eval(env, e)?,
                        None => Value::Nil,
                    };
                    return Ok(Control::Return(v));
                }
            }
        }
        Ok(Control::Normal)
    }

    // --- expressions -------------------------------------------------------

    fn eval(&mut self, env: &Rc<Env>, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(Value::Str(s.as_str().into())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Var(name) => env
                .get(name)
                .ok_or_else(|| format!("undefined variable '{}'", name)),
            Expr::List(items) => {
                let mut list = Vec::with_capacity(items.len());
                for item in items {
                    list.push(self.eval(env, item)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(list))))
            }
            Expr::Map(entries) => {
                let mut map = HashMap::new();
                for (k, v) in entries {
                    let key = self.eval(env, k)?;
                    let value = self.eval(env, v)?;
                    let h = match &key {
                        Value::Nil | Value::Bool(_) | Value::Num(_) | Value::Str(_) => hash_key(&key),
                        other => {
                            return Err(format!("invalid map key of type {}", type_name(other)));
                        }
                    };
                    map.insert(h, (key, value));
                }
                Ok(Value::Map(Rc::new(RefCell::new(map))))
            }
            Expr::Index { container, key } => {
                let c = self.eval(env, container)?;
                let k = self.eval(env, key)?;
                index_get(&c, &k)
            }
            Expr::Unary { op, operand } => {
                let v = self.eval(env, operand)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Num(n) => Ok(Value::Num(-n)),
                        other => Err(format!("cannot negate a value of type {}", type_name(&other))),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!is_truthy(&v))),
                }
            }
            Expr::Binary { op: BinOp::And, left, right } => {
                let l = self.eval(env, left)?;
                if !is_truthy(&l) {
                    return Ok(l);
                }
                self.eval(env, right)
            }
            Expr::Binary { op: BinOp::Or, left, right } => {
                let l = self.eval(env, left)?;
                if is_truthy(&l) {
                    return Ok(l);
                }
                self.eval(env, right)
            }
            Expr::Binary { op, left, right } => {
                let l = self.eval(env, left)?;
                let r = self.eval(env, right)?;
                binary_op(*op, &l, &r)
            }
            Expr::Call { callee, args } => {
                let callee_v = self.eval(env, callee)?;
                let mut arg_values = Vec::with_capacity(args.len());
                for a in args {
                    arg_values.push(self.eval(env, a)?);
                }
                self.call(&callee_v, arg_values)
            }
            Expr::Assign { target, op, value } => match target {
                AssignTarget::Name(name) => {
                    let v = self.eval(env, value)?;
                    let v = match op {
                        Some(binop) => {
                            let current = env.get(name).ok_or_else(|| {
                                format!("undefined variable '{}'", name)
                            })?;
                            binary_op(*binop, &current, &v)?
                        }
                        None => v,
                    };
                    if !env.set_existing(name, v.clone()) {
                        self.root.vars.borrow_mut().insert(name.clone(), v.clone());
                    }
                    Ok(v)
                }
                AssignTarget::Index { container, key } => {
                    let c = self.eval(env, container)?;
                    let k = self.eval(env, key)?;
                    let v = self.eval(env, value)?;
                    let v = match op {
                        Some(binop) => {
                            let current = index_get(&c, &k)?;
                            binary_op(*binop, &current, &v)?
                        }
                        None => v,
                    };
                    index_set(&c, &k, &v)?;
                    Ok(v)
                }
            },
            Expr::Fn { params, body } => Ok(Value::Fn(Rc::new(FnValue {
                name: "<anonymous>".to_string(),
                params: params.clone(),
                body: body.clone(),
                env: env.clone(),
            }))),
        }
    }

    fn call(&mut self, callee: &Value, args: Vec<Value>) -> Result<Value, String> {
        match callee {
            Value::Native(name) => self.call_native(name, &args),
            Value::Fn(f) => {
                if args.len() != f.params.len() {
                    return Err(format!(
                        "function '{}' expects {} argument(s) but got {}",
                        f.name,
                        f.params.len(),
                        args.len()
                    ));
                }
                let scope = Env::child(&f.env);
                for (p, a) in f.params.iter().zip(args) {
                    scope.vars.borrow_mut().insert(p.clone(), a);
                }
                self.call_stack.push(f.name.clone());
                let result = self.exec(&scope, &f.body);
                match result {
                    Ok(control) => {
                        self.call_stack.pop();
                        match control {
                            Control::Return(v) => Ok(v),
                            _ => Ok(Value::Nil),
                        }
                    }
                    // Keep the call stack intact on error so the top level can
                    // attach a traceback.
                    Err(e) => Err(e),
                }
            }
            other => Err(format!("cannot call a value of type {}", type_name(other))),
        }
    }

    /// Attach the current Corros call stack to an error when debugging.
    fn with_trace(&self, err: String) -> String {
        if std::env::var("CORROS_SEED_DEBUG").is_ok() && !self.call_stack.is_empty() {
            format!("{} (in {})", err, self.call_stack.join(" -> "))
        } else {
            err
        }
    }
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

pub(crate) fn binary_op(op: BinOp, a: &Value, b: &Value) -> Result<Value, String> {
    match op {
        BinOp::Add => match (a, b) {
            (Value::Num(x), Value::Num(y)) => Ok(Value::Num(x + y)),
            (Value::Str(x), Value::Str(y)) => Ok(Value::Str(format!("{}{}", x, y).into())),
            (Value::List(x), Value::List(y)) => {
                let mut items = x.borrow().clone();
                items.extend(y.borrow().iter().cloned());
                Ok(Value::List(Rc::new(RefCell::new(items))))
            }
            _ => Err(format!("cannot add {} and {}", type_name(a), type_name(b))),
        },
        BinOp::Sub => num_op(a, b, |x, y| x - y, "subtract"),
        BinOp::Mul => num_op(a, b, |x, y| x * y, "multiply"),
        BinOp::Div => num_op(a, b, |x, y| x / y, "divide"),
        BinOp::Mod => num_op(a, b, |x, y| x % y, "take the remainder of"),
        BinOp::Power => num_op(a, b, |x, y| x.powf(y), "raise"),
        BinOp::Less => cmp_op(a, b, |x, y| x < y, |x, y| x < y),
        BinOp::LessEqual => cmp_op(a, b, |x, y| x <= y, |x, y| x <= y),
        BinOp::Greater => cmp_op(a, b, |x, y| x > y, |x, y| x > y),
        BinOp::GreaterEqual => cmp_op(a, b, |x, y| x >= y, |x, y| x >= y),
        BinOp::Eq => Ok(Value::Bool(value_eq(a, b))),
        BinOp::Neq => Ok(Value::Bool(!value_eq(a, b))),
        BinOp::And => Ok(if is_truthy(a) { b.clone() } else { a.clone() }),
        BinOp::Or => Ok(if is_truthy(a) { a.clone() } else { b.clone() }),
    }
}

fn num_op(
    a: &Value,
    b: &Value,
    f: impl Fn(f64, f64) -> f64,
    verb: &str,
) -> Result<Value, String> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Ok(Value::Num(f(*x, *y))),
        _ => Err(format!(
            "cannot {} {} and {}",
            verb,
            type_name(a),
            type_name(b)
        )),
    }
}

fn cmp_op(
    a: &Value,
    b: &Value,
    num_cmp: impl Fn(f64, f64) -> bool,
    str_cmp: impl Fn(&str, &str) -> bool,
) -> Result<Value, String> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Ok(Value::Bool(num_cmp(*x, *y))),
        (Value::Str(x), Value::Str(y)) => Ok(Value::Bool(str_cmp(x, y))),
        _ => Err(format!("cannot compare {} and {}", type_name(a), type_name(b))),
    }
}

// ---------------------------------------------------------------------------
// Indexing
// ---------------------------------------------------------------------------

fn index_from_value(key: &Value) -> Result<usize, String> {
    match key {
        Value::Num(n) if n.fract() == 0.0 && *n >= 0.0 => Ok(*n as usize),
        Value::Num(n) => Err(format!(
            "index must be a non-negative integer, got num ({})",
            format_num(*n)
        )),
        _ => Err(format!(
            "index must be a non-negative integer, got {}",
            type_name(key)
        )),
    }
}

pub(crate) fn index_get(container: &Value, key: &Value) -> Result<Value, String> {
    match container {
        Value::List(items) => {
            let i = index_from_value(key)?;
            match items.borrow().get(i) {
                Some(v) => Ok(v.clone()),
                None => Err(format!(
                    "index out of bounds: {} (list has {} elements)",
                    i,
                    items.borrow().len()
                )),
            }
        }
        Value::Str(s) => {
            let i = index_from_value(key)?;
            match s.chars().nth(i) {
                Some(c) => Ok(Value::Str(c.to_string().into())),
                None => Err(format!(
                    "index out of bounds: {} (string has {} characters)",
                    i,
                    s.chars().count()
                )),
            }
        }
        Value::Range { start, end, inclusive } => {
            let i = index_from_value(key)?;
            let len = range_len(*start, *end, *inclusive);
            if i < len {
                Ok(Value::Num(start + i as f64))
            } else {
                Err(format!(
                    "index out of bounds: {} (range has {} elements)",
                    i, len
                ))
            }
        }
        Value::Map(entries) => {
            let h = hash_key(key);
            match entries.borrow().get(&h) {
                Some((_, v)) => Ok(v.clone()),
                None => Err(format!("map has no key {}", repr(key))),
            }
        }
        other => Err(format!("cannot index a value of type {}", type_name(other))),
    }
}

pub(crate) fn index_set(container: &Value, key: &Value, value: &Value) -> Result<(), String> {
    match container {
        Value::List(items) => {
            let i = index_from_value(key)?;
            let mut items = items.borrow_mut();
            if i < items.len() {
                items[i] = value.clone();
                Ok(())
            } else {
                Err(format!(
                    "index out of bounds: {} (list has {} elements)",
                    i,
                    items.len()
                ))
            }
        }
        Value::Map(entries) => {
            let h = match key {
                Value::Nil | Value::Bool(_) | Value::Num(_) | Value::Str(_) => hash_key(key),
                other => {
                    return Err(format!("invalid map key of type {}", type_name(other)));
                }
            };
            entries.borrow_mut().insert(h, (key.clone(), value.clone()));
            Ok(())
        }
        Value::Str(_) => Err("strings are immutable".to_string()),
        other => Err(format!("cannot index a value of type {}", type_name(other))),
    }
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

fn expect_args(name: &str, args: &[Value], count: usize) -> Result<(), String> {
    if args.len() != count {
        return Err(format!(
            "{} expects {} argument(s) but got {}",
            name,
            count,
            args.len()
        ));
    }
    Ok(())
}

fn expect_args_between(name: &str, args: &[Value], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        return Err(format!(
            "{} expects between {} and {} arguments but got {}",
            name,
            min,
            max,
            args.len()
        ));
    }
    Ok(())
}

fn want_num(name: &str, v: &Value) -> Result<f64, String> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(format!("{} expects a number, got {}", name, type_name(other))),
    }
}

fn want_str(name: &str, v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        other => Err(format!(
            "{} expects a string, got {}",
            name,
            type_name(other)
        )),
    }
}

/// Execute one of the builtins available to compiled programs (`speak`,
/// `size`, `mcall`, ...). Stateless apart from the output buffer, the echo
/// flag, and the clock, so both the tree-walking seed and the native
/// bytecode executor can share the exact same table.
pub(crate) fn native_builtin(
    name: &str,
    args: &[Value],
    output: &mut Vec<String>,
    echo: bool,
    start: &Instant,
) -> Result<Value, String> {
    match name {
        "speak" => {
            let parts: Vec<String> = args.iter().map(to_string).collect();
            let line = parts.join(" ");
            output.push(line.clone());
            if echo {
                println!("{}", line);
            }
            Ok(Value::Nil)
        }
            "hear" => {
                expect_args_between("hear", args, 0, 1)?;
                if let Some(prompt) = args.first() {
                    if echo {
                        print!("{}", to_string(prompt));
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    }
                }
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) => Err("hear: end of input".to_string()),
                    Ok(_) => {
                        while line.ends_with('\n') || line.ends_with('\r') {
                            line.pop();
                        }
                        Ok(Value::Str(line.into()))
                    }
                    Err(e) => Err(format!("hear: {}", e)),
                }
            }
            "size" => {
                expect_args("size", args, 1)?;
                let len = match &args[0] {
                    Value::Str(s) => s.chars().count(),
                    Value::List(items) => items.borrow().len(),
                    Value::Map(entries) => entries.borrow().len(),
                    Value::Range { start, end, inclusive } => {
                        range_len(*start, *end, *inclusive)
                    }
                    other => {
                        return Err(format!(
                            "size expects a string, list, map, or range, got {}",
                            type_name(other)
                        ));
                    }
                };
                Ok(Value::Num(len as f64))
            }
            "nature" => {
                expect_args("nature", args, 1)?;
                Ok(Value::Str(type_name(&args[0]).into()))
            }
            "str" => {
                expect_args("str", args, 1)?;
                Ok(Value::Str(to_string(&args[0]).into()))
            }
            "num" => {
                expect_args("num", args, 1)?;
                let n = match &args[0] {
                    Value::Num(n) => *n,
                    Value::Str(s) => s.trim().parse::<f64>().map_err(|_| {
                        format!("cannot num '{}' as a number", s)
                    })?,
                    Value::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    other => {
                        return Err(format!(
                            "cannot num {} as a number",
                            type_name(other)
                        ));
                    }
                };
                Ok(Value::Num(n))
            }
            "int" => {
                expect_args("int", args, 1)?;
                let n = match &args[0] {
                    Value::Num(n) => n.trunc(),
                    Value::Str(s) => s
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| format!("cannot int '{}'", s))?
                        .trunc(),
                    Value::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    other => {
                        return Err(format!("cannot int {}", type_name(other)));
                    }
                };
                Ok(Value::Num(n))
            }
            "bool" => {
                expect_args("bool", args, 1)?;
                Ok(Value::Bool(is_truthy(&args[0])))
            }
            "abs" => {
                expect_args("abs", args, 1)?;
                Ok(Value::Num(want_num("abs", &args[0])?.abs()))
            }
            "root" => {
                expect_args("root", args, 1)?;
                Ok(Value::Num(want_num("root", &args[0])?.sqrt()))
            }
            "least" => {
                if args.is_empty() {
                    return Err("least expects at least one argument".to_string());
                }
                let mut best = f64::INFINITY;
                for a in args {
                    best = best.min(want_num("least", a)?);
                }
                Ok(Value::Num(best))
            }
            "greatest" => {
                if args.is_empty() {
                    return Err("greatest expects at least one argument".to_string());
                }
                let mut best = f64::NEG_INFINITY;
                for a in args {
                    best = best.max(want_num("greatest", a)?);
                }
                Ok(Value::Num(best))
            }
            "tick" => Ok(Value::Num(start.elapsed().as_secs_f64())),
            "span" => {
                expect_args_between("span", args, 1, 2)?;
                let start = match args.len() {
                    1 => 0.0,
                    _ => want_num("span", &args[0])?,
                };
                let end = want_num("span", &args[args.len() - 1])?;
                Ok(Value::Range { start, end, inclusive: false })
            }
            "vouch" => {
                expect_args_between("vouch", args, 1, 2)?;
                if !is_truthy(&args[0]) {
                    let message = args
                        .get(1)
                        .map(to_string)
                        .unwrap_or_else(|| "vouch failed: the condition was false".to_string());
                    return Err(message);
                }
                Ok(Value::Nil)
            }
            "flaw" => {
                expect_args("flaw", args, 1)?;
                Err(to_string(&args[0]))
            }
            "read" => {
                expect_args("read", args, 1)?;
                let path = want_str("read", &args[0])?;
                match std::fs::read_to_string(&path) {
                    Ok(text) => Ok(Value::Str(text.into())),
                    Err(e) => Err(format!("read: cannot open '{}': {}", path, e)),
                }
            }
            "readlines" => {
                expect_args("readlines", args, 1)?;
                let path = want_str("readlines", &args[0])?;
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let lines: Vec<Value> = text
                            .lines()
                            .map(|l| Value::Str(l.to_string().into()))
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(lines))))
                    }
                    Err(e) => Err(format!("readlines: cannot open '{}': {}", path, e)),
                }
            }
            "shove" => {
                expect_args("shove", args, 2)?;
                match &args[0] {
                    Value::List(l) => {
                        l.borrow_mut().push(args[1].clone());
                        Ok(Value::Nil)
                    }
                    other => Err(format!("expected a list, got {}", type_name(other))),
                }
            }
            "yank" => {
                expect_args("yank", args, 1)?;
                match &args[0] {
                    Value::List(l) => l
                        .borrow_mut()
                        .pop()
                        .ok_or_else(|| "yank: the list is empty".to_string()),
                    other => Err(format!("expected a list, got {}", type_name(other))),
                }
            }
            "file_exists" => {
                expect_args("file_exists", args, 1)?;
                let path = want_str("file_exists", &args[0])?;
                Ok(Value::Bool(std::path::Path::new(&path).exists()))
            }
            "mcall" => {
                expect_args("mcall", args, 3)?;
                let name = want_str("mcall", &args[0])?;
                let receiver = &args[1];
                let arg_list: Vec<Value> = match &args[2] {
                    Value::List(l) => l.borrow().clone(),
                    other => return Err(format!("expected a list, got {}", type_name(other))),
                };
                match lookup_method(receiver, &name) {
                    Some(method) => method(receiver, &arg_list),
                    None => Err(format!(
                        "value of type {} has no method '{}'",
                        type_name(receiver),
                        name
                    )),
                }
            }
            other => Err(format!("unknown builtin '{}'", other)),
    }
}

impl Interpreter {
    fn call_native(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        native_builtin(name, args, &mut self.output, self.echo, &self.start)
    }
}

// ---------------------------------------------------------------------------
// The native method table (fallback for what the Corros stdlib cannot express)
// ---------------------------------------------------------------------------

type MethodFn = fn(&Value, &[Value]) -> Result<Value, String>;

fn lookup_method(receiver: &Value, name: &str) -> Option<MethodFn> {
    match receiver {
        Value::List(_) => match name {
            "shove" => Some(list_shove),
            "yank" => Some(list_yank),
            "size" => Some(list_size),
            "slot" => Some(list_slot),
            "pluck" => Some(list_pluck),
            "holds" => Some(list_holds),
            "weld" => Some(list_weld),
            "order" => Some(list_order),
            "flip" => Some(list_flip),
            "clear" => Some(list_clear),
            _ => None,
        },
        Value::Str(_) => match name {
            "size" => Some(str_size),
            "loud" => Some(str_loud),
            "quiet" => Some(str_quiet),
            "shave" => Some(str_shave),
            "split" => Some(str_split),
            "holds" => Some(str_holds),
            "opens" => Some(str_opens),
            "closes" => Some(str_closes),
            "reforge" => Some(str_reforge),
            _ => None,
        },
        Value::Map(_) => match name {
            "size" => Some(map_size),
            "labels" => Some(map_labels),
            "contents" => Some(map_contents),
            "holds" => Some(map_holds),
            "fetch" => Some(map_fetch),
            "pluck" => Some(map_pluck),
            "clear" => Some(map_clear),
            _ => None,
        },
        Value::Range { .. } => match name {
            "size" => Some(range_size),
            "holds" => Some(range_holds),
            _ => None,
        },
        _ => None,
    }
}

fn as_list(v: &Value) -> Result<Rc<RefCell<Vec<Value>>>, String> {
    match v {
        Value::List(l) => Ok(l.clone()),
        other => Err(format!("expected a list, got {}", type_name(other))),
    }
}

/// Internal storage of a map value: hash -> (key, value), keeping the key so
/// it can be printed and compared by identity.
type MapStorage = HashMap<u64, (Value, Value)>;

fn as_map(v: &Value) -> Result<Rc<RefCell<MapStorage>>, String> {
    match v {
        Value::Map(m) => Ok(m.clone()),
        other => Err(format!("expected a map, got {}", type_name(other))),
    }
}

fn as_str(v: &Value) -> Result<Rc<str>, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(format!("expected a string, got {}", type_name(other))),
    }
}

fn as_range(v: &Value) -> Result<(f64, f64, bool), String> {
    match v {
        Value::Range { start, end, inclusive } => Ok((*start, *end, *inclusive)),
        other => Err(format!("expected a range, got {}", type_name(other))),
    }
}

fn index_arg(name: &str, v: &Value) -> Result<usize, String> {
    match v {
        Value::Num(n) if n.fract() == 0.0 && *n >= 0.0 => Ok(*n as usize),
        _ => Err(format!(
            "{} expects a non-negative integer index, got {}",
            name,
            type_name(v)
        )),
    }
}

fn value_less(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Ok(x < y),
        (Value::Str(x), Value::Str(y)) => Ok(x < y),
        _ => Err(format!(
            "cannot order: cannot compare {} and {}",
            type_name(a),
            type_name(b)
        )),
    }
}

fn list_shove(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("shove", args, 1)?;
    as_list(receiver)?.borrow_mut().push(args[0].clone());
    Ok(Value::Nil)
}

fn list_yank(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("yank", args, 0)?;
    as_list(receiver)?
        .borrow_mut()
        .pop()
        .ok_or_else(|| "yank: the list is empty".to_string())
}

fn list_size(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("size", args, 0)?;
    Ok(Value::Num(as_list(receiver)?.borrow().len() as f64))
}

fn list_slot(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("slot", args, 2)?;
    let i = index_arg("slot", &args[0])?;
    let list = as_list(receiver)?;
    let mut items = list.borrow_mut();
    if i <= items.len() {
        items.insert(i, args[1].clone());
        Ok(Value::Nil)
    } else {
        Err(format!(
            "slot: index out of bounds: {} (list has {} elements)",
            i,
            items.len()
        ))
    }
}

fn list_pluck(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("pluck", args, 1)?;
    let i = index_arg("pluck", &args[0])?;
    let list = as_list(receiver)?;
    let mut items = list.borrow_mut();
    if i < items.len() {
        Ok(items.remove(i))
    } else {
        Err(format!(
            "pluck: index out of bounds: {} (list has {} elements)",
            i,
            items.len()
        ))
    }
}

fn list_holds(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("holds", args, 1)?;
    Ok(Value::Bool(as_list(receiver)?.borrow().iter().any(|x| value_eq(x, &args[0]))))
}

fn list_weld(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("weld", args, 1)?;
    let sep = match &args[0] {
        Value::Str(s) => s.to_string(),
        other => {
            return Err(format!(
                "weld expects a string separator, got {}",
                type_name(other)
            ));
        }
    };
    let parts: Vec<String> = as_list(receiver)?.borrow().iter().map(to_string).collect();
    Ok(Value::Str(parts.join(&sep).into()))
}

fn list_order(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("order", args, 0)?;
    let list = as_list(receiver)?;
    let mut items = list.borrow_mut();
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && value_less(&items[j], &items[j - 1])? {
            items.swap(j, j - 1);
            j -= 1;
        }
    }
    Ok(Value::Nil)
}

fn list_flip(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("flip", args, 0)?;
    as_list(receiver)?.borrow_mut().reverse();
    Ok(Value::Nil)
}

fn list_clear(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("clear", args, 0)?;
    as_list(receiver)?.borrow_mut().clear();
    Ok(Value::Nil)
}

fn str_size(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("size", args, 0)?;
    Ok(Value::Num(as_str(receiver)?.chars().count() as f64))
}

fn str_loud(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("loud", args, 0)?;
    Ok(Value::Str(as_str(receiver)?.to_uppercase().into()))
}

fn str_quiet(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("quiet", args, 0)?;
    Ok(Value::Str(as_str(receiver)?.to_lowercase().into()))
}

fn str_shave(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("shave", args, 0)?;
    Ok(Value::Str(as_str(receiver)?.trim().into()))
}

fn str_split(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("split", args, 1)?;
    let sep = match &args[0] {
        Value::Str(s) => s.to_string(),
        other => {
            return Err(format!(
                "split expects a string separator, got {}",
                type_name(other)
            ));
        }
    };
    if sep.is_empty() {
        return Err("split: the separator must not be empty".to_string());
    }
    let parts: Vec<Value> = as_str(receiver)?
        .split(&sep)
        .map(|p| Value::Str(p.to_string().into()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(parts))))
}

fn str_holds(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("holds", args, 1)?;
    let needle = want_str("holds", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.contains(&needle)))
}

fn str_opens(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("opens", args, 1)?;
    let prefix = want_str("opens", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.starts_with(&prefix)))
}

fn str_closes(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("closes", args, 1)?;
    let suffix = want_str("closes", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.ends_with(&suffix)))
}

fn str_reforge(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("reforge", args, 2)?;
    let from = want_str("reforge", &args[0])?;
    let to = want_str("reforge", &args[1])?;
    Ok(Value::Str(as_str(receiver)?.replace(&from, &to).into()))
}

fn map_size(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("size", args, 0)?;
    Ok(Value::Num(as_map(receiver)?.borrow().len() as f64))
}

fn map_labels(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("labels", args, 0)?;
    let keys: Vec<Value> = as_map(receiver)?.borrow().values().map(|(k, _)| k.clone()).collect();
    Ok(Value::List(Rc::new(RefCell::new(keys))))
}

fn map_contents(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("contents", args, 0)?;
    let values: Vec<Value> = as_map(receiver)?.borrow().values().map(|(_, v)| v.clone()).collect();
    Ok(Value::List(Rc::new(RefCell::new(values))))
}

fn map_holds(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("holds", args, 1)?;
    let h = hash_key(&args[0]);
    Ok(Value::Bool(as_map(receiver)?.borrow().contains_key(&h)))
}

fn map_fetch(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args_between("fetch", args, 1, 2)?;
    let h = hash_key(&args[0]);
    match as_map(receiver)?.borrow().get(&h) {
        Some((_, v)) => Ok(v.clone()),
        None => Ok(args.get(1).cloned().unwrap_or(Value::Nil)),
    }
}

fn map_pluck(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("pluck", args, 1)?;
    let h = hash_key(&args[0]);
    Ok(Value::Bool(as_map(receiver)?.borrow_mut().remove(&h).is_some()))
}

fn map_clear(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("clear", args, 0)?;
    as_map(receiver)?.borrow_mut().clear();
    Ok(Value::Nil)
}

fn range_size(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("size", args, 0)?;
    let (start, end, inclusive) = as_range(receiver)?;
    Ok(Value::Num(range_len(start, end, inclusive) as f64))
}

fn range_holds(receiver: &Value, args: &[Value]) -> Result<Value, String> {
    expect_args("holds", args, 1)?;
    let x = want_num("holds", &args[0])?;
    let (start, end, inclusive) = as_range(receiver)?;
    let within = if inclusive {
        x >= start && x <= end
    } else {
        x >= start && x < end
    };
    Ok(Value::Bool(within))
}

// ---------------------------------------------------------------------------
// Bootstrapping: locate the Corros-written interpreter and run it
// ---------------------------------------------------------------------------

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Locate one of the Corros-written source files (`compiler.cor`, `vm.cor`,
/// `prelude.cor`). Search order: `$CORROS_LIB` directory, next to the running
/// binary, `../src` from it, `./src` (development), and the crate directory.
fn find_src(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("CORROS_LIB") {
        candidates.push(PathBuf::from(dir).join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
            candidates.push(dir.join("..").join("src").join(name));
        }
    }
    candidates.push(PathBuf::from("src").join(name));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join(name));
    candidates.into_iter().find(|p| p.exists())
}

fn write_temp(prefix: &str, content: &str) -> Result<PathBuf, String> {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "{}-{}-{}.tmp",
        prefix,
        std::process::id(),
        n
    ));
    std::fs::write(&path, content)
        .map_err(|e| format!("corros: could not write temporary file: {}", e))?;
    Ok(path)
}

/// Compile a program with the Corros-written compiler and return its
/// bytecode text.
fn compile_to_bytecode(path: &str, args: &[String]) -> Result<String, String> {
    let compiler_cor = find_src("compiler.cor")
        .ok_or_else(|| "corros: cannot find src/compiler.cor (the Corros-written compiler)".to_string())?;
    let prelude = find_src("prelude.cor");
    let mut compile_args = vec![path.to_string()];
    if let Some(p) = &prelude {
        compile_args.push(p.display().to_string());
    }
    compile_args.extend(args.iter().cloned());
    let mut compiler = Interpreter::new(compile_args);
    compiler.echo = false;
    let bytecode_lines = match compiler.run_file(&compiler_cor) {
        Ok(lines) => lines,
        Err(e) => {
            if std::env::var("CORROS_SEED_DEBUG").is_ok() {
                eprintln!("[compiler output before error:]");
                for line in &compiler.output {
                    eprintln!("{}", line);
                }
            }
            return Err(e);
        }
    };
    Ok(bytecode_lines.join("\n") + "\n")
}

/// Run a Corros program: the seed runs `src/compiler.cor` (compiling the
/// program to bytecode), then the native executor (`src/native.rs`) runs that
/// bytecode at native speed. Returns the program's `speak` output. This is
/// the same path the `corros` binary uses for every program.
pub fn run_source(source: &str, args: &[String]) -> Result<Vec<String>, String> {
    let src_path = write_temp("corros-src", source)?;
    let result = run_chain(&src_path.display().to_string(), args);
    let _ = std::fs::remove_file(&src_path);
    result
}

/// Run the chain for a program that already exists on disk: the Corros
/// compiler compiles it, and the native executor runs the bytecode.
pub fn run_file(path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let bytecode = compile_to_bytecode(path, args)?;
    crate::native::run_bytecode(&bytecode, args, echo)
}

/// The pure-Corros path: the seed runs `src/vm.cor`, which interprets the
/// bytecode the Corros compiler produced. This is the reference interpreter
/// (the `--reference` flag and `demo.sh`); it is correct but slow, because
/// the VM's dispatch is itself interpreted.
pub fn run_file_reference(path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let bytecode = compile_to_bytecode(path, args)?;
    run_bytecode_on_reference(&bytecode, args, echo)
}

/// Execute pre-compiled bytecode text on the native executor.
pub fn run_bytecode_on_native(text: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    crate::native::run_bytecode(text, args, echo)
}

/// Execute pre-compiled bytecode text through the Corros-written VM
/// (`src/vm.cor`), for the self-hosting demo.
pub fn run_bytecode_on_reference(text: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let vm_cor = find_src("vm.cor")
        .ok_or_else(|| "corros: cannot find src/vm.cor (the Corros-written VM)".to_string())?;
    let bc_path = write_temp("corros-bc", text)?;
    let mut vm_args = vec![bc_path.display().to_string()];
    vm_args.extend(args.iter().cloned());
    let mut vm = Interpreter::new(vm_args);
    vm.echo = echo;
    let result = vm.run_file(&vm_cor);
    let _ = std::fs::remove_file(&bc_path);
    result
}

fn run_chain(path: &str, args: &[String]) -> Result<Vec<String>, String> {
    run_file(path, args, false)
}

/// `--dump FILE`: run the Corros compiler on FILE and return the bytecode it
/// prints, without executing it.
pub fn dump_bytecode(path: &str) -> Result<Vec<String>, String> {
    let compiler_cor = find_src("compiler.cor")
        .ok_or_else(|| "corros: cannot find src/compiler.cor (the Corros-written compiler)".to_string())?;
    let prelude = find_src("prelude.cor");
    let mut args = vec![path.to_string()];
    if let Some(p) = &prelude {
        args.push(p.display().to_string());
    }
    let mut compiler = Interpreter::new(args);
    compiler.echo = false;
    compiler.run_file(&compiler_cor)
}

/// `--run-bc FILE.bc`: execute compiled bytecode text on the native executor.
pub fn run_vm_on(bc_path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(bc_path)
        .map_err(|e| format!("corros: could not open '{}': {}", bc_path, e))?;
    crate::native::run_bytecode(&text, args, echo)
}

/// `--reference --run-bc FILE.bc`: execute compiled bytecode text through the
/// Corros-written VM (`src/vm.cor`) — the self-hosting demo path.
pub fn run_vm_on_reference(bc_path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(bc_path)
        .map_err(|e| format!("corros: could not open '{}': {}", bc_path, e))?;
    run_bytecode_on_reference(&text, args, echo)
}
