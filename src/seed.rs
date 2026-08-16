//! seed.rs — the Corros bootstrap seed.
//!
//! Every language needs a first compiler written in *some* other language —
//! rustc's first compiler was written in OCaml, and GCC was written in C.
//! This file is that seed for Corros: a small tree-walking interpreter that
//! runs just enough of Corros to boot the real interpreter, which lives in
//! Corros itself:
//!
//!   - `src/compiler.cro`  — the Corros lexer + bytecode compiler
//!   - `src/vm.cro`        — the Corros virtual machine
//!   - `src/prelude.cro`   — the Corros standard library
//!
//! `corros hello.cro` therefore does: the seed runs the Corros compiler (which
//! compiles hello.cro to bytecode), then the seed runs the Corros VM (which
//! executes that bytecode). The language you use *is* the Corros-written one;
//! the seed only knows the disciplined subset the self-hosted files are
//! written in (craft/forge/whilst/when/each, lists, strings, numbers, ranges,
//! maps, and the builtins).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::lexer::{Token, TokenKind};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RangeData {
    pub start: f64,
    pub end: f64,
    pub inclusive: bool,
}

/// A runtime value. `Range` is boxed so `Value` stays 16 bytes — the stack
/// copies a whole `Value` on every push, so a smaller enum is measurably
/// faster in the native executor's hot loop.
#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<HashMap<u64, (Value, Value)>>>),
    Range(Rc<RangeData>),
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
        (Value::Range(a), Value::Range(b)) => {
            a.start == b.start && a.end == b.end && a.inclusive == b.inclusive
        }
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
        Value::Range(r) => format!(
            "{}{}{}",
            format_num(r.start),
            if r.inclusive { "..=" } else { ".." },
            format_num(r.end)
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
            "speak", "eprint", "hear", "size", "nature", "str", "num", "int", "bool", "abs",
            "root", "least", "greatest", "tick", "span", "vouch", "flaw", "read",
            "readlines", "shove", "yank", "file_exists", "mcall",
            // Host services (the Corros-written Ollama server).
            "net_listen", "net_accept", "net_read", "net_write", "net_close", "net_timeout",
            "http_get", "http_download", "file_write", "file_append", "sys_exec",
            "load_lib", "lib_call", "mem_i64", "cstr_alloc", "cstr_get", "cstr_free",
            "mem_alloc", "mem_free", "lib_close", "getenv",
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
                        Value::Range(r) => {
                            let len = range_len(r.start, r.end, r.inclusive);
                            (0..len).map(|i| Value::Num(r.start + i as f64)).collect()
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
        Value::Range(r) => {
            let i = index_from_value(key)?;
            let len = range_len(r.start, r.end, r.inclusive);
            if i < len {
                Ok(Value::Num(r.start + i as f64))
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

fn want_str_list(name: &str, v: &Value) -> Result<Vec<String>, String> {
    match v {
        Value::List(l) => l
            .borrow()
            .iter()
            .map(|x| match x {
                Value::Str(s) => Ok(s.to_string()),
                other => Err(format!(
                    "{} expects a list of strings, got a list containing {}",
                    name,
                    type_name(other)
                )),
            })
            .collect(),
        other => Err(format!(
            "{} expects a list of strings, got {}",
            name,
            type_name(other)
        )),
    }
}

// ---------------------------------------------------------------------------
// Host services: networking, HTTP, files, processes, and dynamic libraries
// ---------------------------------------------------------------------------
//
// These power the Corros-written Ollama server (`/sdcard/Projects/Ollama`):
// Corros holds every handle as an opaque u64 number, and the seed is the only
// place that touches the OS beyond the filesystem. Handles start at 1 and
// increment, so they never lose precision in f64.

fn next_handle() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

static LISTENERS: Mutex<Option<HashMap<u64, TcpListener>>> = Mutex::new(None);
/// Open connections: (stream, read-timeout in ms). 0 = no timeout.
static CONNS: Mutex<Option<HashMap<u64, (TcpStream, u64)>>> = Mutex::new(None);
/// dlopen handles stored as `usize` (raw pointers are not Sync, usize is).
static LIBS: Mutex<Option<HashMap<u64, usize>>> = Mutex::new(None);
/// Scratch allocations: ptr -> byte size (so `mem_free` can dealloc safely).
static MEMS: Mutex<Option<HashMap<u64, usize>>> = Mutex::new(None);

fn listeners() -> std::sync::MutexGuard<'static, Option<HashMap<u64, TcpListener>>> {
    LISTENERS.lock().unwrap()
}
fn conns() -> std::sync::MutexGuard<'static, Option<HashMap<u64, (TcpStream, u64)>>> {
    CONNS.lock().unwrap()
}
fn libs() -> std::sync::MutexGuard<'static, Option<HashMap<u64, usize>>> {
    LIBS.lock().unwrap()
}

#[cfg(unix)]
#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const std::ffi::c_char, flag: i32) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn dlclose(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(unix)]
const RTLD_NOW: i32 = 2;

/// The dynamic-FFI calling convention: every C function in the Corros engine
/// shim (`engine.c`) takes up to 8 integer/pointer arguments and returns an
/// integer/pointer. This matches the SysV x86-64 and AAPCS64 ABIs, so the
/// transmute is safe in practice for pointer/int args.
#[cfg(unix)]
type DynFn = unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64;

/// One GET request (optionally streamed to a file so binary GGUF downloads
/// never round-trip through a string). Follows up to 5 redirects. Returns
/// (status, body bytes).
/// Fetch over https. Corros has no TLS stack in-tree, so https is delegated
/// to curl (present on every target platform): redirects are followed, the
/// status code is captured with `-w`, and the body goes to stdout or to `-o
/// file`. Returns (status, body) like the plain-http path.
fn http_fetch_https(url: &str, to_path: Option<&str>) -> Result<(f64, Vec<u8>), String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sSL", "-w", "\n%{http_code}", url]);
    if let Some(p) = to_path {
        cmd.arg("-o").arg(p);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("http_get: curl: {}", e))?;
    // curl appends "\n<status>" after the body (or alone when -o is used).
    let text = String::from_utf8_lossy(&out.stdout);
    let code: f64 = text
        .rsplit('\n')
        .next()
        .unwrap_or("0")
        .trim()
        .parse()
        .unwrap_or(0.0);
    let body = if to_path.is_some() {
        Vec::new()
    } else {
        // strip the trailing separator + status line curl appended
        match text.rfind('\n') {
            Some(i) => out.stdout[..i].to_vec(),
            None => Vec::new(),
        }
    };
    Ok((code, body))
}

fn http_fetch(url: &str, to_path: Option<&str>) -> Result<(f64, Vec<u8>), String> {
    if url.starts_with("https://") {
        return http_fetch_https(url, to_path);
    }
    let mut target = url.to_string();
    for _ in 0..5 {
        let rest = target
            .strip_prefix("http://")
            .ok_or_else(|| format!("http_get: only http:// URLs are supported (got '{}')", url))?;
        let (hostport, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match hostport.find(':') {
            Some(i) => (
                &hostport[..i],
                hostport[i + 1..]
                    .parse::<u16>()
                    .map_err(|_| format!("http_get: bad port in '{}'", url))?,
            ),
            None => (hostport, 80u16),
        };
        if host.is_empty() {
            return Err(format!("http_get: bad URL '{}'", url));
        }
        let mut stream = TcpStream::connect((host, port))
            .map_err(|e| format!("http_get: connect to {}:{}: {}", host, port, e))?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: corros/0.1\r\n\r\n",
            path, hostport
        );
        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("http_get: send: {}", e))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("http_get: read: {}", e))?;
        let head = match String::from_utf8_lossy(&raw).find("\r\n\r\n") {
            Some(i) => String::from_utf8_lossy(&raw[..i]).into_owned(),
            None => String::new(),
        };
        let status: f64 = head
            .lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if (300.0..400.0).contains(&status) {
            if let Some(loc) = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("location:"))
                .and_then(|l| l.splitn(2, ':').nth(1))
            {
                target = loc.trim().to_string();
                continue;
            }
        }
        let body_start = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(raw.len());
        if let Some(p) = to_path {
            std::fs::write(p, &raw[body_start..])
                .map_err(|e| format!("http_get: write '{}': {}", p, e))?;
        }
        return Ok((status, raw[body_start..].to_vec()));
    }
    Err(format!("http_get: too many redirects for '{}'", url))
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
        "eprint" => {
            // Write to stderr so diagnostics never pollute captured output
            // (e.g. the C source emitted by `--compile`).
            let parts: Vec<String> = args.iter().map(to_string).collect();
            eprintln!("{}", parts.join(" "));
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
                    Value::Range(r) => range_len(r.start, r.end, r.inclusive),
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
                Ok(Value::Range(Rc::new(RangeData {
                    start,
                    end,
                    inclusive: false,
                })))
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
            // --- CLI bridge: used by src/cli.cro (the Corros-written CLI) ---
            "version" => Ok(Value::Str(env!("CARGO_PKG_VERSION").into())),
            "run" => {
                // run(path, [args]) — compile and execute a program, echoing
                // its output live (exactly like `corros file.cro`).
                expect_args("run", args, 2)?;
                let path = want_str("run", &args[0])?;
                let list = want_str_list("run", &args[1])?;
                run_file(&path, &list, true)?;
                Ok(Value::List(Rc::new(RefCell::new(Vec::new()))))
            }
            "run_bc" => {
                // run_bc(path, [args]) — execute compiled bytecode natively.
                expect_args("run_bc", args, 2)?;
                let path = want_str("run_bc", &args[0])?;
                let list = want_str_list("run_bc", &args[1])?;
                run_vm_on(&path, &list, true)?;
                Ok(Value::List(Rc::new(RefCell::new(Vec::new()))))
            }
            "run_ref" => {
                // run_ref(path, [args]) — run through the Corros-written VM.
                expect_args("run_ref", args, 2)?;
                let path = want_str("run_ref", &args[0])?;
                let list = want_str_list("run_ref", &args[1])?;
                run_file_reference(&path, &list, true)?;
                Ok(Value::List(Rc::new(RefCell::new(Vec::new()))))
            }
            "dump" => {
                // dump(path) — the bytecode the Corros compiler emits.
                expect_args("dump", args, 1)?;
                let path = want_str("dump", &args[0])?;
                let lines = dump_bytecode(&path)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    lines.into_iter().map(|l| Value::Str(l.into())).collect(),
                ))))
            }
            "run_src_try" => {
                // run_src_try(source) — run a source string, never failing:
                // [true, line...] on success, [false, error] on failure. The
                // REPL needs this to keep going after an error.
                expect_args("run_src_try", args, 1)?;
                let src = want_str("run_src_try", &args[0])?;
                let mut items: Vec<Value> = Vec::new();
                match run_source(&src, &[]) {
                    Ok(lines) => {
                        items.push(Value::Bool(true));
                        for l in lines {
                            items.push(Value::Str(l.into()));
                        }
                    }
                    Err(e) => {
                        items.push(Value::Bool(false));
                        items.push(Value::Str(e.into()));
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(items))))
            }
            "native_compile" => {
                // native_compile(path, [out]) — AOT-compile a program to a
                // native binary: the Corros compiler produces bytecode, the
                // static analyzer (src/codegen.rs) types it, C is emitted and
                // built with cc -O3. Returns the output path.
                expect_args("native_compile", args, 2)?;
                let path = want_str("native_compile", &args[0])?;
                let list = want_str_list("native_compile", &args[1])?;
                let out = if let Some(o) = list.first() {
                    o.clone()
                } else {
                    // Strip the canonical .cro extension (or the legacy
                    // .cor alias) for the output binary name.
                    path.strip_suffix(".cro")
                        .or_else(|| path.strip_suffix(".cor"))
                        .unwrap_or(&path)
                        .to_string()
                };
                // Compile WITHOUT the prelude: the prelude uses methods,
                // closures, and maps that the static analyzer cannot type.
                // Programs that only use plain functions, numbers, ranges,
                // and builtins are what --compile targets.
                let bytecode = compile_user_program_no_prelude(&path)?;
                // The AOT backend is itself written in Corros (src/codegen.cro):
                // run its compiled bytecode on the user's bytecode and capture
                // the C source it prints.
                let bc_path = write_temp("corros-src-bc", &bytecode)?;
                let cg_bc = get_codegen_bytecode()?;
                let debug = std::env::var("CORROS_CODEGEN_DEBUG").is_ok();
                let mut cg_args = vec![bc_path.display().to_string()];
                if debug {
                    cg_args.push("--dbg".to_string());
                }
                let c_lines = crate::native::run_bytecode(&cg_bc, &cg_args, debug)?;
                let _ = std::fs::remove_file(&bc_path);
                let c_src = c_lines.join("\n") + "\n";
                let c_path = write_temp("corros-c", &c_src)?;
                let status = std::process::Command::new("cc")
                    .args(["-O3", "-x", "c", "-o", &out, c_path.to_str().unwrap(), "-lm"])
                    .status()
                    .map_err(|e| format!("native_compile: could not run cc: {}", e))?;
                if std::env::var("CORROS_KEEP_C").is_ok() {
                    eprintln!("[C source: {}]", c_path.display());
                } else {
                    let _ = std::fs::remove_file(&c_path);
                }
                if !status.success() {
                    return Err(format!(
                        "native_compile: cc failed (exit {})",
                        status
                    ));
                }
                Ok(Value::Str(out.into()))
            }
            // --- Host services: networking, HTTP, files, processes, FFI ---
            // Power the Corros-written Ollama server. Handles are opaque u64s.
            "net_listen" => {
                expect_args("net_listen", args, 1)?;
                let port = want_num("net_listen", &args[0])? as u16;
                let listener = TcpListener::bind(("0.0.0.0", port))
                    .map_err(|e| format!("net_listen: cannot bind port {}: {}", port, e))?;
                let h = next_handle();
                listeners().get_or_insert_with(HashMap::new).insert(h, listener);
                Ok(Value::Num(h as f64))
            }
            "net_accept" => {
                expect_args("net_accept", args, 1)?;
                let h = want_num("net_accept", &args[0])? as u64;
                let listener = listeners()
                    .get_or_insert_with(HashMap::new)
                    .get(&h)
                    .and_then(|l| l.try_clone().ok())
                    .ok_or_else(|| "net_accept: no such listener".to_string())?;
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let ch = next_handle();
                        stream.set_read_timeout(Some(Duration::from_millis(30_000))).ok();
                        conns()
                            .get_or_insert_with(HashMap::new)
                            .insert(ch, (stream, 30_000));
                        Ok(Value::Num(ch as f64))
                    }
                    Err(e) => Err(format!("net_accept: {}", e)),
                }
            }
            "net_read" => {
                // net_read(conn, max) — read up to max bytes. Returns "" on
                // EOF or when the read timeout elapses (a stalled client must
                // never hang the server).
                expect_args("net_read", args, 2)?;
                let ch = want_num("net_read", &args[0])? as u64;
                let max = want_num("net_read", &args[1])? as usize;
                let mut guard = conns();
                let map = guard.get_or_insert_with(HashMap::new);
                let (stream, timeout_ms) = map
                    .get_mut(&ch)
                    .ok_or_else(|| "net_read: no such connection".to_string())?;
                if *timeout_ms > 0 {
                    stream.set_read_timeout(Some(Duration::from_millis(*timeout_ms))).ok();
                }
                let mut buf = vec![0u8; max.max(1)];
                match stream.read(&mut buf) {
                    Ok(0) => Ok(Value::Str("".into())),
                    Ok(n) => Ok(Value::Str(
                        String::from_utf8_lossy(&buf[..n]).into_owned().into(),
                    )),
                    Err(_) => Ok(Value::Str("".into())),
                }
            }
            "net_write" => {
                expect_args("net_write", args, 2)?;
                let ch = want_num("net_write", &args[0])? as u64;
                let data = want_str("net_write", &args[1])?;
                let mut guard = conns();
                let map = guard.get_or_insert_with(HashMap::new);
                let (stream, _) = map
                    .get_mut(&ch)
                    .ok_or_else(|| "net_write: no such connection".to_string())?;
                let n = stream
                    .write(data.as_bytes())
                    .map_err(|e| format!("net_write: {}", e))?;
                Ok(Value::Num(n as f64))
            }
            "net_close" => {
                expect_args("net_close", args, 1)?;
                let ch = want_num("net_close", &args[0])? as u64;
                let mut guard = conns();
                guard.get_or_insert_with(HashMap::new).remove(&ch);
                Ok(Value::Nil)
            }
            "net_timeout" => {
                // net_timeout(conn, ms) — set the read timeout (0 = wait forever).
                expect_args("net_timeout", args, 2)?;
                let ch = want_num("net_timeout", &args[0])? as u64;
                let ms = want_num("net_timeout", &args[1])? as u64;
                let mut guard = conns();
                if let Some((stream, t)) = guard.get_or_insert_with(HashMap::new).get_mut(&ch) {
                    *t = ms;
                    if ms > 0 {
                        stream.set_read_timeout(Some(Duration::from_millis(ms))).ok();
                    } else {
                        stream.set_read_timeout(None).ok();
                    }
                }
                Ok(Value::Nil)
            }
            "http_get" => {
                // http_get(url) -> [status, body]
                expect_args("http_get", args, 1)?;
                let url = want_str("http_get", &args[0])?;
                let (status, body) = http_fetch(&url, None)?;
                Ok(Value::List(Rc::new(RefCell::new(vec![
                    Value::Num(status),
                    Value::Str(String::from_utf8_lossy(&body).into_owned().into()),
                ]))))
            }
            "http_download" => {
                // http_download(url, path) -> status (streams raw bytes to file)
                expect_args("http_download", args, 2)?;
                let url = want_str("http_download", &args[0])?;
                let path = want_str("http_download", &args[1])?;
                let (status, _) = http_fetch(&url, Some(&path))?;
                Ok(Value::Num(status))
            }
            "file_write" => {
                expect_args("file_write", args, 2)?;
                let path = want_str("file_write", &args[0])?;
                let data = want_str("file_write", &args[1])?;
                std::fs::write(&path, data.as_bytes())
                    .map_err(|e| format!("file_write '{}': {}", path, e))?;
                Ok(Value::Nil)
            }
            "file_append" => {
                expect_args("file_append", args, 2)?;
                let path = want_str("file_append", &args[0])?;
                let data = want_str("file_append", &args[1])?;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|e| format!("file_append '{}': {}", path, e))?;
                f.write_all(data.as_bytes())
                    .map_err(|e| format!("file_append '{}': {}", path, e))?;
                Ok(Value::Nil)
            }
            "getenv" => {
                // getenv(name) -> value (or "" when unset)
                expect_args("getenv", args, 1)?;
                let name = want_str("getenv", &args[0])?;
                Ok(Value::Str(
                    std::env::var(&name).unwrap_or_default().into(),
                ))
            }
            "sys_exec" => {
                // sys_exec(cmd, [args]) -> [status, stdout]
                expect_args("sys_exec", args, 2)?;
                let cmd = want_str("sys_exec", &args[0])?;
                let list = want_str_list("sys_exec", &args[1])?;
                let out = std::process::Command::new(&cmd)
                    .args(&list)
                    .output()
                    .map_err(|e| format!("sys_exec '{}': {}", cmd, e))?;
                let code = out.status.code().unwrap_or(-1) as f64;
                Ok(Value::List(Rc::new(RefCell::new(vec![
                    Value::Num(code),
                    Value::Str(String::from_utf8_lossy(&out.stdout).into_owned().into()),
                ]))))
            }
            "load_lib" => {
                expect_args("load_lib", args, 1)?;
                let path = want_str("load_lib", &args[0])?;
                #[cfg(unix)]
                {
                    let cpath = std::ffi::CString::new(path.as_str())
                        .map_err(|_| "load_lib: path contains a NUL byte".to_string())?;
                    let h = unsafe { dlopen(cpath.as_ptr(), RTLD_NOW) };
                    if h.is_null() {
                        // 0 = failure; the caller decides (e.g. copy the lib
                        // to a loadable location and retry). Some filesystems
                        // (Android's sdcard fuse mount) refuse dlopen.
                        eprintln!("load_lib: could not dlopen '{}'", path);
                        return Ok(Value::Num(0.0));
                    }
                    let id = next_handle();
                    let mut guard = libs();
                    guard.get_or_insert_with(HashMap::new).insert(id, h as usize);
                    Ok(Value::Num(id as f64))
                }
                #[cfg(not(unix))]
                {
                    Err("load_lib: only supported on unix".to_string())
                }
            }
            "lib_call" => {
                // lib_call(lib, "fn", [i64 args...]) -> i64 result. Pointers
                // travel as u64 bit patterns (canonical user pointers are < 2^48,
                // so f64 round-trips them exactly).
                expect_args("lib_call", args, 3)?;
                let id = want_num("lib_call", &args[0])? as u64;
                let name = want_str("lib_call", &args[1])?;
                let arg_list = match &args[2] {
                    Value::List(l) => l.borrow().clone(),
                    _ => return Err("lib_call: third argument must be a list".to_string()),
                };
                #[cfg(unix)]
                {
                    let mut guard = libs();
                    let lib = guard
                        .get_or_insert_with(HashMap::new)
                        .get(&id)
                        .copied()
                        .ok_or_else(|| "lib_call: no such library".to_string())?
                        as *mut std::ffi::c_void;
                    let cname = std::ffi::CString::new(name.as_str())
                        .map_err(|_| "lib_call: symbol name contains a NUL byte".to_string())?;
                    let sym = unsafe { dlsym(lib, cname.as_ptr()) };
                    if sym.is_null() {
                        return Err(format!("lib_call: symbol '{}' not found", name));
                    }
                    let f: DynFn = unsafe { std::mem::transmute(sym) };
                    let mut a = [0i64; 8];
                    for (i, v) in arg_list.iter().enumerate().take(8) {
                        match v {
                            Value::Num(n) => a[i] = (*n as u64) as i64,
                            Value::Bool(b) => a[i] = if *b { 1 } else { 0 },
                            _ => {
                                return Err(
                                    "lib_call: arguments must be numbers (integers or pointers)"
                                        .to_string(),
                                );
                            }
                        }
                    }
                    let r = unsafe { f(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]) };
                    Ok(Value::Num(r as f64))
                }
                #[cfg(not(unix))]
                {
                    Err("lib_call: only supported on unix".to_string())
                }
            }
            "mem_i64" => {
                // mem_i64(ptr, index) — read the i64 at ptr + index*8.
                expect_args("mem_i64", args, 2)?;
                let ptr = want_num("mem_i64", &args[0])? as u64 as *const i64;
                let idx = want_num("mem_i64", &args[1])? as usize;
                let v = unsafe { *ptr.add(idx) };
                Ok(Value::Num(v as f64))
            }
            "mem_alloc" => {
                // mem_alloc(bytes) -> pointer to 8-aligned scratch memory
                // (token buffers passed to the engine shim).
                expect_args("mem_alloc", args, 1)?;
                let n = want_num("mem_alloc", &args[0])? as usize;
                if n == 0 {
                    return Err("mem_alloc: size must be positive".to_string());
                }
                let layout = std::alloc::Layout::from_size_align(n, 8)
                    .map_err(|_| "mem_alloc: bad size".to_string())?;
                let p = unsafe { std::alloc::alloc(layout) };
                if p.is_null() {
                    return Err("mem_alloc: out of memory".to_string());
                }
                let id = p as u64;
                MEMS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, n);
                Ok(Value::Num(id as f64))
            }
            "mem_free" => {
                expect_args("mem_free", args, 1)?;
                let id = want_num("mem_free", &args[0])? as u64;
                let mut guard = MEMS.lock().unwrap();
                let n = guard
                    .get_or_insert_with(HashMap::new)
                    .remove(&id)
                    .ok_or_else(|| "mem_free: not a tracked allocation".to_string())?;
                let layout = std::alloc::Layout::from_size_align(n, 8)
                    .map_err(|_| "mem_free: bad size".to_string())?;
                unsafe {
                    std::alloc::dealloc(id as *mut u8, layout);
                }
                Ok(Value::Nil)
            }
            "cstr_alloc" => {
                // cstr_alloc(s) -> pointer to a C string (leak until cstr_free).
                expect_args("cstr_alloc", args, 1)?;
                let s = want_str("cstr_alloc", &args[0])?;
                let c = std::ffi::CString::new(s.as_str())
                    .map_err(|_| "cstr_alloc: string contains a NUL byte".to_string())?;
                let p = c.into_raw();
                Ok(Value::Num(p as u64 as f64))
            }
            "cstr_get" => {
                // cstr_get(ptr) -> string (reads up to the NUL terminator).
                expect_args("cstr_get", args, 1)?;
                let ptr = want_num("cstr_get", &args[0])? as u64 as *const u8;
                let mut v = Vec::new();
                unsafe {
                    let mut p = ptr;
                    loop {
                        let b = *p;
                        if b == 0 {
                            break;
                        }
                        v.push(b);
                        p = p.add(1);
                    }
                }
                Ok(Value::Str(String::from_utf8_lossy(&v).into_owned().into()))
            }
            "cstr_free" => {
                expect_args("cstr_free", args, 1)?;
                let ptr = want_num("cstr_free", &args[0])? as u64 as *mut std::ffi::c_char;
                unsafe {
                    drop(std::ffi::CString::from_raw(ptr));
                }
                Ok(Value::Nil)
            }
            "lib_close" => {
                expect_args("lib_close", args, 1)?;
                let id = want_num("lib_close", &args[0])? as u64;
                #[cfg(unix)]
                {
                    let mut guard = libs();
                    if let Some(h) = guard.get_or_insert_with(HashMap::new).remove(&id) {
                        unsafe {
                            dlclose(h as *mut std::ffi::c_void);
                        }
                    }
                }
                Ok(Value::Nil)
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
        Value::Range(_) => match name {
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
        Value::Range(r) => Ok((r.start, r.end, r.inclusive)),
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

/// Locate one of the Corros-written source files (`compiler.cro`, `vm.cro`,
/// `prelude.cro`). Search order: `$CORROS_LIB` directory, next to the running
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

/// Compile a program with the Corros-written compiler (the seed interprets
/// `src/compiler.cro` from source) and return its bytecode text.
fn compile_to_bytecode(path: &str, args: &[String]) -> Result<String, String> {
    let compiler_cor = find_src("compiler.cro")
        .ok_or_else(|| "corros: cannot find src/compiler.cro (the Corros-written compiler)".to_string())?;
    let prelude = find_src("prelude.cro");
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

/// FNV-1a — a stable, dependency-free hash for cache keys.
fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Where the compiled compiler is cached: a content-addressed bytecode file
/// keyed on `compiler.cro` and `prelude.cro`, so the seed never re-interprets
/// the compiler for programs it has already seen the sources of.
fn compiler_cache_path() -> Result<PathBuf, String> {
    let compiler_cor = find_src("compiler.cro")
        .ok_or_else(|| "corros: cannot find src/compiler.cro (the Corros-written compiler)".to_string())?;
    let csrc = std::fs::read(&compiler_cor)
        .map_err(|e| format!("corros: could not read '{}': {}", compiler_cor.display(), e))?;
    let mut hash = fnv1a(&csrc);
    if let Some(p) = find_src("prelude.cro") {
        if let Ok(psrc) = std::fs::read(&p) {
            hash ^= fnv1a(&psrc).rotate_left(32);
        }
    }
    Ok(std::env::temp_dir().join(format!("corros-compiler-{:016x}.bc", hash)))
}

/// The compiled compiler: `compiler.cro` compiled once (with the prelude
/// spliced, exactly like any program) and cached. `demo.sh` proves the
/// compiled compiler is byte-identical to the source, so running it on the
/// native executor compiles programs exactly as the seed would — without the
/// seed re-interpreting the compiler for every run.
fn get_compiler_bytecode() -> Result<String, String> {
    let cache = compiler_cache_path()?;
    if let Ok(text) = std::fs::read_to_string(&cache) {
        return Ok(text);
    }
    let compiler_cor = find_src("compiler.cro")
        .ok_or_else(|| "corros: cannot find src/compiler.cro (the Corros-written compiler)".to_string())?;
    let bytecode = compile_to_bytecode(&compiler_cor.display().to_string(), &[])?;
    let _ = std::fs::write(&cache, &bytecode);
    Ok(bytecode)
}

/// Compile a program using the cached compiled compiler (native speed), and
/// return its bytecode text.
fn compile_user_program(path: &str, args: &[String]) -> Result<String, String> {
    let compiler_bc = get_compiler_bytecode()?;
    let prelude = find_src("prelude.cro");
    let mut compile_args = vec![path.to_string()];
    if let Some(p) = &prelude {
        compile_args.push(p.display().to_string());
    }
    compile_args.extend(args.iter().cloned());
    let lines = crate::native::run_bytecode(&compiler_bc, &compile_args, false)?;
    Ok(lines.join("\n") + "\n")
}

/// Compile a program with the cached compiled compiler but WITHOUT splicing
/// the prelude — used by `--compile`, whose static analyzer rejects the
/// prelude's methods/closures/maps.
fn compile_user_program_no_prelude(path: &str) -> Result<String, String> {
    let compiler_bc = get_compiler_bytecode()?;
    let compile_args = vec![path.to_string()];
    let lines = crate::native::run_bytecode(&compiler_bc, &compile_args, false)?;
    Ok(lines.join("\n") + "\n")
}

/// Content-addressed cache path for the compiled AOT backend
/// (`src/codegen.cro` compiled with the prelude, by the cached compiler).
fn codegen_cache_path() -> Result<PathBuf, String> {
    let cg = find_src("codegen.cro")
        .ok_or_else(|| "corros: cannot find src/codegen.cro (the Corros-written AOT backend)".to_string())?;
    let mut hash = fnv1a(&std::fs::read(&cg).map_err(|e| e.to_string())?);
    if let Some(p) = find_src("prelude.cro") {
        if let Ok(psrc) = std::fs::read(&p) {
            hash ^= fnv1a(&psrc).rotate_left(32);
        }
    }
    if let Some(p) = find_src("compiler.cro") {
        if let Ok(csrc) = std::fs::read(&p) {
            hash ^= fnv1a(&csrc).rotate_left(16);
        }
    }
    Ok(std::env::temp_dir().join(format!("corros-codegen-{:016x}.bc", hash)))
}

/// The compiled AOT backend bytecode, cached so --compile skips recompiling
/// codegen.cro on every run.
fn get_codegen_bytecode() -> Result<String, String> {
    let cache = codegen_cache_path()?;
    if let Ok(text) = std::fs::read_to_string(&cache) {
        return Ok(text);
    }
    let cg = find_src("codegen.cro")
        .ok_or_else(|| "corros: cannot find src/codegen.cro (the Corros-written AOT backend)".to_string())?;
    let bytecode = compile_user_program(&cg.display().to_string(), &[])?;
    let _ = std::fs::write(&cache, &bytecode);
    Ok(bytecode)
}

/// Run a Corros program: the cached compiled compiler (`compiler.cro`
/// compiled once, then run at native speed) compiles the program, and the
/// native executor (`src/native.rs`) runs that bytecode. Returns the
/// program's `speak` output. This is the same path the `corros` binary uses
/// for every program.
pub fn run_source(source: &str, args: &[String]) -> Result<Vec<String>, String> {
    let src_path = write_temp("corros-src", source)?;
    let result = run_chain(&src_path.display().to_string(), args);
    let _ = std::fs::remove_file(&src_path);
    result
}

/// Run the chain for a program that already exists on disk: the Corros
/// compiler compiles it, and the native executor runs the bytecode.
pub fn run_file(path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let bytecode = compile_user_program(path, args)?;
    crate::native::run_bytecode(&bytecode, args, echo)
}

/// The pure-Corros path: the seed runs `src/vm.cro`, which interprets the
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
/// (`src/vm.cro`), for the self-hosting demo.
pub fn run_bytecode_on_reference(text: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let bc_path = write_temp("corros-bc", text)?;
    let mut vm_args = vec![bc_path.display().to_string()];
    vm_args.extend(args.iter().cloned());
    // Run the *compiled* VM on the native executor — the same "compiled VM
    // runs compiled code" step demo.sh proves — instead of tree-walking
    // vm.cro's source through the seed. The reference interpreter is then
    // only as slow as its own Corros-written dispatch, not the seed's.
    let vm_bc = get_vm_bytecode()?;
    let result = crate::native::run_bytecode(&vm_bc, &vm_args, echo);
    let _ = std::fs::remove_file(&bc_path);
    result
}

/// Content-addressed cache path for the compiled VM (`src/vm.cro` compiled
/// with the prelude, by the cached compiled compiler).
fn vm_cache_path() -> Result<PathBuf, String> {
    let vm_cor = find_src("vm.cro")
        .ok_or_else(|| "corros: cannot find src/vm.cro (the Corros-written VM)".to_string())?;
    let mut hash = fnv1a(&std::fs::read(&vm_cor).map_err(|e| e.to_string())?);
    if let Some(p) = find_src("prelude.cro") {
        if let Ok(psrc) = std::fs::read(&p) {
            hash ^= fnv1a(&psrc).rotate_left(32);
        }
    }
    if let Some(p) = find_src("compiler.cro") {
        if let Ok(csrc) = std::fs::read(&p) {
            hash ^= fnv1a(&csrc).rotate_left(16);
        }
    }
    Ok(std::env::temp_dir().join(format!("corros-vm-{:016x}.bc", hash)))
}

/// The compiled VM bytecode, cached so the reference path skips recompiling
/// vm.cro on every run.
fn get_vm_bytecode() -> Result<String, String> {
    let cache = vm_cache_path()?;
    if let Ok(text) = std::fs::read_to_string(&cache) {
        return Ok(text);
    }
    let vm_cor = find_src("vm.cro")
        .ok_or_else(|| "corros: cannot find src/vm.cro (the Corros-written VM)".to_string())?;
    let bytecode = compile_user_program(&vm_cor.display().to_string(), &[])?;
    let _ = std::fs::write(&cache, &bytecode);
    Ok(bytecode)
}

fn run_chain(path: &str, args: &[String]) -> Result<Vec<String>, String> {
    run_file(path, args, false)
}

/// `--dump FILE`: run the Corros compiler on FILE and return the bytecode it
/// prints, without executing it.
pub fn dump_bytecode(path: &str) -> Result<Vec<String>, String> {
    let compiler_cor = find_src("compiler.cro")
        .ok_or_else(|| "corros: cannot find src/compiler.cro (the Corros-written compiler)".to_string())?;
    let prelude = find_src("prelude.cro");
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
/// Corros-written VM (`src/vm.cro`) — the self-hosting demo path.
pub fn run_vm_on_reference(bc_path: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(bc_path)
        .map_err(|e| format!("corros: could not open '{}': {}", bc_path, e))?;
    run_bytecode_on_reference(&text, args, echo)
}

/// Boot the Corros-written CLI (`src/cli.cro`): compile it with the cached
/// compiled compiler and run it on the native executor, echoing its output
/// live. Every decision the old Rust `main` made — flags, `--dump`,
/// `--run-bc`, `--reference`, the REPL — is now made in Corros.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let cli_path = find_src("cli.cro")
        .ok_or_else(|| "corros: cannot find src/cli.cro (the Corros-written CLI)".to_string())?;
    let bytecode = compile_user_program(&cli_path.display().to_string(), &[])?;
    crate::native::run_bytecode(&bytecode, args, true)?;
    Ok(())
}
