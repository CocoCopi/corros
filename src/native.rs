//! native.rs — the native bytecode executor.
//!
//! The interpreter is written in Corros (`src/compiler.cor`, `src/vm.cor`,
//! `src/prelude.cor`), and the seed in `seed.rs` boots it. But running user
//! bytecode through the Corros VM *under* the tree-walking seed costs tens of
//! microseconds per instruction — the Corros VM's dispatch is itself
//! interpreted. This module is the bootstrap accelerator: it executes the
//! exact same textual bytecode that `compiler.cor` emits, natively, with the
//! same semantics as `vm.cor` (globals, frames, upvalue cells, maps, ranges,
//! methods routed through the Corros-written `$method` stdlib). What it adds
//! over the reference VM is speed: O(1) HashMap globals instead of a scanned
//! list, a flat `Vec` stack, and a tight opcode loop.
//!
//! The Corros VM (`src/vm.cor`) remains the reference implementation and is
//! still exercised by `demo.sh` and the `--reference` flag.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::seed::{
    self, binary_op, index_get, index_set, is_truthy, to_string, value_eq, BinOp, Value,
};

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Const(Value),
    Nil,
    True,
    False,
    Pop,
    GetLocal(usize),
    SetLocal(usize),
    GetUpvalue(usize),
    SetUpvalue(usize),
    GetGlobal(String),
    SetGlobal(String),
    DefineGlobal(String),
    GetIndex,
    SetIndex,
    GetField(String),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Power,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    Neg,
    Not,
    Jump(usize),
    JumpIfFalse(usize),
    Loop(usize),
    Call(usize),
    Return,
    Closure { fid: usize, upvals: Vec<(bool, usize)> },
    CloseUpvalue,
    Rotate3,
    BuildList(usize),
    BuildMap(usize),
    BuildRange(bool),
}

#[derive(Debug)]
struct Function {
    name: String,
    arity: usize,
    instrs: Vec<Op>,
}

/// Reverse of `compiler.cor`'s `escape_str`: turn `"a\nb"` back into a real
/// string. Mirrors `vm.cor`'s `unescape` exactly.
fn unescape(s: &str) -> String {
    let inner = &s[1..s.len().saturating_sub(1)];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_literal(s: &str) -> Result<Value, String> {
    match s {
        "nil" => Ok(Value::Nil),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ if s.starts_with('"') => Ok(Value::Str(unescape(s).into())),
        _ => s
            .trim()
            .parse::<f64>()
            .map(Value::Num)
            .map_err(|_| format!("bad literal '{}' in bytecode", s)),
    }
}

fn parse_num(what: &str, s: &str) -> Result<usize, String> {
    s.trim()
        .parse::<usize>()
        .map_err(|_| format!("bad {} '{}' in bytecode", what, s))
}

fn parse_instr(line: &str) -> Result<Op, String> {
    let mut parts = line.splitn(2, ' ');
    let op = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    match op {
        "CONST" => Ok(Op::Const(parse_literal(rest)?)),
        "GET_GLOBAL" | "SET_GLOBAL" | "DEFINE_GLOBAL" | "GET_FIELD" => {
            Ok(match op {
                "GET_GLOBAL" => Op::GetGlobal(rest.to_string()),
                "SET_GLOBAL" => Op::SetGlobal(rest.to_string()),
                "DEFINE_GLOBAL" => Op::DefineGlobal(rest.to_string()),
                _ => Op::GetField(rest.to_string()),
            })
        }
        "GET_LOCAL" => Ok(Op::GetLocal(parse_num("index", rest)?)),
        "SET_LOCAL" => Ok(Op::SetLocal(parse_num("index", rest)?)),
        "GET_UPVALUE" => Ok(Op::GetUpvalue(parse_num("index", rest)?)),
        "SET_UPVALUE" => Ok(Op::SetUpvalue(parse_num("index", rest)?)),
        "CALL" => Ok(Op::Call(parse_num("arg count", rest)?)),
        "BUILD_LIST" => Ok(Op::BuildList(parse_num("arg count", rest)?)),
        "BUILD_MAP" => Ok(Op::BuildMap(parse_num("pair count", rest)?)),
        "JUMP" => Ok(Op::Jump(parse_num("target", rest)?)),
        "JUMP_IF_FALSE" => Ok(Op::JumpIfFalse(parse_num("target", rest)?)),
        "LOOP" => Ok(Op::Loop(parse_num("target", rest)?)),
        "CLOSURE" => {
            // CLOSURE <fid> <is_local> <index> ...  (variable-length operands)
            let toks: Vec<&str> = rest.split(' ').collect();
            let fid = parse_num("function id", toks.first().unwrap_or(&""))?;
            let mut upvals = Vec::new();
            let mut k = 1;
            while k + 1 < toks.len() {
                let is_local = parse_num("closure flag", toks[k])? != 0;
                let idx = parse_num("closure index", toks[k + 1])?;
                upvals.push((is_local, idx));
                k += 2;
            }
            Ok(Op::Closure { fid, upvals })
        }
        "BUILD_RANGE" => Ok(Op::BuildRange(rest == "inclusive")),
        "NIL" => Ok(Op::Nil),
        "TRUE" => Ok(Op::True),
        "FALSE" => Ok(Op::False),
        "POP" => Ok(Op::Pop),
        "GET_INDEX" => Ok(Op::GetIndex),
        "SET_INDEX" => Ok(Op::SetIndex),
        "ADD" => Ok(Op::Add),
        "SUB" => Ok(Op::Sub),
        "MUL" => Ok(Op::Mul),
        "DIV" => Ok(Op::Div),
        "MOD" => Ok(Op::Mod),
        "POWER" => Ok(Op::Power),
        "EQ" => Ok(Op::Eq),
        "NEQ" => Ok(Op::Neq),
        "LT" => Ok(Op::Lt),
        "LE" => Ok(Op::Le),
        "GT" => Ok(Op::Gt),
        "GE" => Ok(Op::Ge),
        "NEG" => Ok(Op::Neg),
        "NOT" => Ok(Op::Not),
        "RETURN" => Ok(Op::Return),
        "CLOSE_UPVALUE" => Ok(Op::CloseUpvalue),
        "ROTATE3" => Ok(Op::Rotate3),
        _ => Err(format!("unknown opcode '{}' in bytecode", op)),
    }
}

/// Parse the textual bytecode that `compiler.cor` prints into functions.
/// Mirrors `vm.cor`'s `parse_bc`: `FUNCTION <id> <name> <arity>` blocks ended
/// by `ENDFN`, `MAIN` headers skipped.
fn load_bytecode(text: &str) -> Result<Vec<Function>, String> {
    let mut fns: Vec<Function> = Vec::new();
    let mut current: Option<(String, usize, Vec<Op>)> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("FUNCTION ") {
            if let Some((name, arity, instrs)) = current.take() {
                fns.push(Function { name, arity, instrs });
            }
            let mut p = rest.splitn(3, ' ');
            let id = parse_num("function id", p.next().unwrap_or(""))?;
            let name = p.next().unwrap_or("<anonymous>").to_string();
            let arity = parse_num("arity", p.next().unwrap_or(""))?;
            if id != fns.len() {
                return Err(format!(
                    "bytecode: function ids out of order (expected {}, got {})",
                    fns.len(),
                    id
                ));
            }
            current = Some((name, arity, Vec::new()));
            continue;
        }
        if line == "ENDFN" {
            if let Some((name, arity, instrs)) = current.take() {
                fns.push(Function { name, arity, instrs });
            }
            continue;
        }
        if line.starts_with("MAIN ") {
            continue;
        }
        if let Some((_, _, instrs)) = current.as_mut() {
            instrs.push(parse_instr(line)?);
        }
    }
    if let Some((name, arity, instrs)) = current.take() {
        fns.push(Function { name, arity, instrs });
    }
    if fns.is_empty() {
        return Err("bytecode: no functions found".to_string());
    }
    Ok(fns)
}

// ---------------------------------------------------------------------------
// The executor
// ---------------------------------------------------------------------------

struct Frame {
    fid: usize,
    pc: usize,
    /// Stack index of local slot 0 (the first argument).
    slots_base: usize,
    /// Stack index the callee was pushed at; Return truncates back to here.
    trunc_base: usize,
    /// Per-slot capture cells (each a one-element list, like vm.cor).
    cells: Vec<Option<Value>>,
    /// Upvalue cells captured from the enclosing frame.
    upvals: Vec<Value>,
}

pub struct NativeVm {
    fns: Vec<Function>,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: HashMap<String, Value>,
    output: Vec<String>,
    echo: bool,
    start: Instant,
}

impl NativeVm {
    fn install_builtins(&mut self) {
        let names = [
            "size", "nature", "str", "num", "int", "bool", "abs", "root", "least", "greatest",
            "tick", "span", "speak", "hear", "read", "readlines", "file_exists", "flaw", "shove",
            "yank", "vouch", "mcall",
        ];
        for name in names {
            self.globals.insert(
                name.to_string(),
                Value::List(Rc::new(RefCell::new(vec![
                    Value::Str("blt".into()),
                    Value::Str(name.into()),
                ]))),
            );
        }
    }

    fn cell_value(v: Value) -> Value {
        Value::List(Rc::new(RefCell::new(vec![v])))
    }

    /// Capture local slot `idx` of the current frame into a shared cell.
    /// Mirrors `vm.cor`'s `capture_cell`.
    fn capture_cell(&mut self, idx: usize) -> Value {
        let fr = self.frames.last_mut().unwrap();
        while fr.cells.len() <= idx {
            fr.cells.push(None);
        }
        if fr.cells[idx].is_none() {
            let v = self.stack[fr.slots_base + idx].clone();
            fr.cells[idx] = Some(Self::cell_value(v));
        }
        fr.cells[idx].clone().unwrap()
    }

    fn do_call(&mut self, argc: usize) -> Result<(), String> {
        let callee_idx = self.stack.len() - argc - 1;
        let callee = self.stack[callee_idx].clone();
        match &callee {
            Value::List(l) => {
                let items = l.borrow();
                if items.len() >= 2 && value_eq(&items[0], &Value::Str("fn".into())) {
                    let fid = match &items[1] {
                        Value::Num(n) => *n as usize,
                        _ => return Err("corrupt closure value".to_string()),
                    };
                    let arity = self.fns[fid].arity;
                    if argc != arity {
                        return Err(format!(
                            "function '{}' expects {} argument(s) but got {}",
                            self.fns[fid].name, arity, argc
                        ));
                    }
                    if self.frames.len() > 5000 {
                        return Err("stack overflow: too many nested calls".to_string());
                    }
                    let upvals = match &items[2] {
                        Value::List(u) => u.borrow().clone(),
                        _ => return Err("corrupt closure upvalues".to_string()),
                    };
                    // Local slot 0 is the first argument (slots_base); Return
                    // truncates to trunc_base (the callee position).
                    self.frames.push(Frame {
                        fid,
                        pc: 0,
                        slots_base: callee_idx + 1,
                        trunc_base: callee_idx,
                        cells: Vec::new(),
                        upvals,
                    });
                    Ok(())
                } else if items.len() == 3 && value_eq(&items[0], &Value::Str("method".into())) {
                    // Route through the Corros-written standard library:
                    // $method(recv, name, [args]).
                    let name = match &items[1] {
                        Value::Str(s) => s.clone(),
                        _ => return Err("corrupt method value".to_string()),
                    };
                    let receiver = items[2].clone();
                    let args_list = Value::List(Rc::new(RefCell::new(
                        self.stack[callee_idx + 1..].to_vec(),
                    )));
                    self.stack.truncate(callee_idx);
                    let m = self
                        .globals
                        .get("$method")
                        .cloned()
                        .ok_or("internal error: $method is not defined (prelude missing)")?;
                    self.stack.push(m);
                    self.stack.push(receiver);
                    self.stack.push(Value::Str(name));
                    self.stack.push(args_list);
                    self.do_call(3)
                } else if items.len() == 2 && value_eq(&items[0], &Value::Str("blt".into())) {
                    let name = match &items[1] {
                        Value::Str(s) => s.to_string(),
                        _ => return Err("corrupt builtin value".to_string()),
                    };
                    let args: Vec<Value> = self.stack[callee_idx + 1..].to_vec();
                    let result =
                        seed::native_builtin(&name, &args, &mut self.output, self.echo, &self.start);
                    self.stack.truncate(callee_idx);
                    self.stack.push(result?);
                    Ok(())
                } else {
                    Err("cannot call a value of type list".to_string())
                }
            }
            other => Err(format!(
                "cannot call a value of type {}",
                seed::type_name(other)
            )),
        }
    }

    fn do_return(&mut self) {
        let result = self.stack.pop().unwrap();
        let frame = self.frames.pop().unwrap();
        while self.stack.len() > frame.trunc_base {
            self.stack.pop();
        }
        self.stack.push(result);
    }

    fn build_list(&mut self, n: usize) {
        let start = self.stack.len() - n;
        let items: Vec<Value> = self.stack[start..].to_vec();
        self.stack.truncate(start);
        self.stack.push(Value::List(Rc::new(RefCell::new(items))));
    }

    fn build_map(&mut self, n: usize) -> Result<(), String> {
        let start = self.stack.len() - 2 * n;
        let pairs: Vec<Value> = self.stack[start..].to_vec();
        self.stack.truncate(start);
        let mut map: HashMap<u64, (Value, Value)> = HashMap::new();
        for pair in pairs.chunks(2) {
            match &pair[0] {
                Value::Nil | Value::Bool(_) | Value::Num(_) | Value::Str(_) => {}
                other => {
                    return Err(format!(
                        "invalid map key of type {}",
                        seed::type_name(other)
                    ));
                }
            }
            let h = seed::hash_key(&pair[0]);
            map.insert(h, (pair[0].clone(), pair[1].clone()));
        }
        self.stack.push(Value::Map(Rc::new(RefCell::new(map))));
        Ok(())
    }

    pub fn execute(&mut self) -> Result<(), String> {
        while !self.frames.is_empty() {
            let (fid, pc) = {
                let top = self.frames.last().unwrap();
                (top.fid, top.pc)
            };
            let op = self.fns[fid].instrs[pc].clone();
            self.frames.last_mut().unwrap().pc = pc + 1;
            match op {
                Op::Const(v) => self.stack.push(v),
                Op::Nil => self.stack.push(Value::Nil),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => {
                    self.stack.pop();
                }
                Op::GetLocal(i) => {
                    let base = self.frames.last().unwrap().slots_base;
                    let v = self.stack[base + i].clone();
                    self.stack.push(v);
                }
                Op::SetLocal(i) => {
                    let v = self.stack[self.stack.len() - 1].clone();
                    let fr = self.frames.last_mut().unwrap();
                    self.stack[fr.slots_base + i] = v.clone();
                    if let Some(Some(Value::List(c))) = fr.cells.get(i) {
                        c.borrow_mut()[0] = v;
                    }
                }
                Op::GetUpvalue(i) => {
                    let up = self.frames.last().unwrap().upvals[i].clone();
                    match &up {
                        Value::List(c) => self.stack.push(c.borrow()[0].clone()),
                        _ => return Err("corrupt upvalue cell".to_string()),
                    }
                }
                Op::SetUpvalue(i) => {
                    let v = self.stack[self.stack.len() - 1].clone();
                    let fr = self.frames.last_mut().unwrap();
                    match &fr.upvals[i] {
                        Value::List(c) => c.borrow_mut()[0] = v,
                        _ => return Err("corrupt upvalue cell".to_string()),
                    }
                }
                Op::GetGlobal(name) => match self.globals.get(&name) {
                    Some(v) => self.stack.push(v.clone()),
                    None => return Err(format!("undefined variable '{}'", name)),
                },
                Op::SetGlobal(name) => {
                    let v = self.stack[self.stack.len() - 1].clone();
                    self.globals.insert(name, v);
                }
                Op::DefineGlobal(name) => {
                    let v = self.stack.pop().unwrap();
                    self.globals.insert(name, v);
                }
                Op::GetIndex => {
                    let key = self.stack.pop().unwrap();
                    let container = self.stack.pop().unwrap();
                    let v = index_get(&container, &key)?;
                    self.stack.push(v);
                }
                Op::SetIndex => {
                    let value = self.stack.pop().unwrap();
                    let key = self.stack.pop().unwrap();
                    let container = self.stack.pop().unwrap();
                    index_set(&container, &key, &value)?;
                    self.stack.push(value);
                }
                Op::GetField(name) => {
                    let receiver = self.stack.pop().unwrap();
                    self.stack.push(Value::List(Rc::new(RefCell::new(vec![
                        Value::Str("method".into()),
                        Value::Str(name.into()),
                        receiver,
                    ]))));
                }
                Op::Add => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Add, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Sub => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Sub, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Mul => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Mul, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Div => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Div, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Mod => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Mod, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Power => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Power, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Eq => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(value_eq(&a, &b)));
                }
                Op::Neq => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(!value_eq(&a, &b)));
                }
                Op::Lt => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Less, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Le => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::LessEqual, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Gt => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::Greater, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Ge => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let r = binary_op(BinOp::GreaterEqual, &a, &b)?;
                    self.stack.push(r);
                }
                Op::Neg => {
                    let v = self.stack.pop().unwrap();
                    match v {
                        Value::Num(n) => self.stack.push(Value::Num(-n)),
                        other => {
                            return Err(format!(
                                "cannot negate a value of type {}",
                                seed::type_name(&other)
                            ));
                        }
                    }
                }
                Op::Not => {
                    let v = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(!is_truthy(&v)));
                }
                Op::Jump(target) => self.frames.last_mut().unwrap().pc = target,
                Op::JumpIfFalse(target) => {
                    let cond = self.stack.pop().unwrap();
                    if !is_truthy(&cond) {
                        self.frames.last_mut().unwrap().pc = target;
                    }
                }
                Op::Loop(target) => self.frames.last_mut().unwrap().pc = target,
                Op::Call(n) => self.do_call(n)?,
                Op::Return => self.do_return(),
                Op::Closure { fid, upvals } => {
                    let fr_upvals = self.frames.last().unwrap().upvals.clone();
                    let mut captured: Vec<Value> = Vec::with_capacity(upvals.len());
                    for (is_local, idx) in upvals {
                        if is_local {
                            captured.push(self.capture_cell(idx));
                        } else {
                            captured.push(fr_upvals[idx].clone());
                        }
                    }
                    self.stack.push(Value::List(Rc::new(RefCell::new(vec![
                        Value::Str("fn".into()),
                        Value::Num(fid as f64),
                        Value::List(Rc::new(RefCell::new(captured))),
                    ]))));
                }
                Op::CloseUpvalue => {
                    self.stack.pop();
                }
                Op::Rotate3 => {
                    let c = self.stack.pop().unwrap();
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    self.stack.push(b);
                    self.stack.push(c);
                    self.stack.push(a);
                }
                Op::BuildList(n) => self.build_list(n),
                Op::BuildMap(n) => self.build_map(n)?,
                Op::BuildRange(inclusive) => {
                    let end = self.stack.pop().unwrap();
                    let start = self.stack.pop().unwrap();
                    match (&start, &end) {
                        (Value::Num(s), Value::Num(e)) => {
                            self.stack.push(Value::Range {
                                start: *s,
                                end: if inclusive { *e + 1.0 } else { *e },
                                inclusive: false,
                            });
                        }
                        _ => {
                            return Err(format!(
                                "range bounds must be numbers, got {} and {}",
                                seed::type_name(&start),
                                seed::type_name(&end)
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Run a compiled program (the textual bytecode `compiler.cor` emits) at
/// native speed. Returns the lines printed by `speak`.
pub fn run_bytecode(text: &str, args: &[String], echo: bool) -> Result<Vec<String>, String> {
    let fns = load_bytecode(text)?;
    let mut vm = NativeVm {
        fns,
        stack: Vec::new(),
        frames: Vec::new(),
        globals: HashMap::new(),
        output: Vec::new(),
        echo,
        start: Instant::now(),
    };
    vm.install_builtins();
    let prog_args = Value::List(Rc::new(RefCell::new(
        args.iter().map(|s| Value::Str(s.as_str().into())).collect(),
    )));
    vm.globals.insert("args".to_string(), prog_args);
    vm.frames.push(Frame {
        fid: 0,
        pc: 0,
        slots_base: 0,
        trunc_base: 0,
        cells: Vec::new(),
        upvals: Vec::new(),
    });
    vm.execute()?;
    Ok(vm.output)
}

/// Convenience for tests: `to_string` of a value as Corros prints it.
pub fn value_to_string(v: &Value) -> String {
    to_string(v)
}
