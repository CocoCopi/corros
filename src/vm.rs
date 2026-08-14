//! The Corros virtual machine: a stack-based interpreter for Corros bytecode.
//!
//! The VM keeps a value stack for operands and locals, a frame stack for
//! function calls, and a table of open upvalues (stack slots captured by
//! closures). Native functions and methods are dispatched here too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::chunk::{Function, OpCode};
use crate::error::{RuntimeError, RuntimeResult, TraceFrame};
use crate::stdlib;
use crate::value::{Upvalue, Value};

/// One active function call.
struct Frame {
    function: Rc<Function>,
    /// Upvalues captured by this frame's closure.
    upvalues: Vec<Rc<RefCell<Upvalue>>>,
    ip: usize,
    /// Index into the value stack where this frame's locals begin.
    /// For a call, this is one past the closure slot, so local 0 (the
    /// first parameter) is at `stack[slots]`.
    slots: usize,
    /// Index to truncate the stack to on return: for a call this is the
    /// closure's own slot (dropping closure + args), for the script it is 0.
    base: usize,
}

pub struct VM {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: HashMap<String, Value>,
    /// Open upvalues keyed by the stack slot they capture.
    open_upvalues: HashMap<usize, Rc<RefCell<Upvalue>>>,
    /// Time the VM was created; powers the `tick()` builtin.
    pub(crate) start: Instant,
    /// Output produced by `print`, line by line.
    pub output: Vec<String>,
    /// When true, `print` also writes to stdout (used by the CLI/REPL).
    pub echo: bool,
    /// Safety limit on call depth.
    max_frames: usize,
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            stack: Vec::new(),
            frames: Vec::new(),
            globals: HashMap::new(),
            open_upvalues: HashMap::new(),
            start: Instant::now(),
            output: Vec::new(),
            echo: false,
            max_frames: 100_000,
        };
        for &(name, fun) in stdlib::BUILTINS {
            vm.globals
                .insert(name.to_string(), Value::Native { name, fun });
        }
        vm
    }

    /// Install a global variable (used by the CLI to expose script args).
    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    /// Run a compiled program (or REPL snippet) to completion.
    pub fn run(&mut self, function: Rc<Function>) -> RuntimeResult<()> {
        self.frames.clear();
        self.open_upvalues.clear();
        self.stack.clear();
        self.frames.push(Frame {
            function,
            upvalues: Vec::new(),
            ip: 0,
            slots: 0,
            base: 0,
        });
        self.run_loop()
    }

    fn run_loop(&mut self) -> RuntimeResult<()> {
        loop {
            let (instr, line) = {
                let frame = self.frames.last_mut().expect("frame underflow");
                let instr = frame.function.chunk.code[frame.ip].clone();
                let line = frame.function.chunk.lines[frame.ip];
                frame.ip += 1;
                (instr, line)
            };
            if std::env::var("NOVA_TRACE").is_ok() {
                let stack: Vec<String> = self.stack.iter().map(Value::repr).collect();
                eprintln!(
                    "[{} {:04}] {:<40} stack=[{}]",
                    self.current_frame().function.name,
                    self.current_frame().ip - 1,
                    format!("{:?}", instr),
                    stack.join(", ")
                );
            }
            match instr {
                OpCode::Constant(c) => {
                    let v = self.constant(c).clone();
                    self.push(v);
                }
                OpCode::Nil => self.push(Value::Nil),
                OpCode::True => self.push(Value::Bool(true)),
                OpCode::False => self.push(Value::Bool(false)),
                OpCode::Pop => {
                    self.pop();
                }
                OpCode::GetLocal(slot) => {
                    let slots = self.current_frame().slots;
                    let v = self.stack[slots + slot as usize].clone();
                    self.push(v);
                }
                OpCode::SetLocal(slot) => {
                    // Assignment is an expression: keep the value on the
                    // stack (statements pop it afterwards).
                    let value = self.stack.last().unwrap().clone();
                    let slots = self.current_frame().slots;
                    self.stack[slots + slot as usize] = value;
                }
                OpCode::DefineGlobal(c) => {
                    let name = self.constant_str(c);
                    let value = self.pop();
                    self.globals.insert(name, value);
                }
                OpCode::GetGlobal(c) => {
                    let name = self.constant_str(c);
                    match self.globals.get(&name) {
                        Some(v) => self.push(v.clone()),
                        None => {
                            return Err(self.runtime_error(
                                format!("undefined variable '{}'", name),
                                line,
                            ));
                        }
                    }
                }
                OpCode::SetGlobal(c) => {
                    // Assignment is an expression: keep the value on the stack.
                    let name = self.constant_str(c);
                    let value = self.stack.last().unwrap().clone();
                    self.globals.insert(name, value);
                }
                OpCode::GetUpvalue(u) => {
                    let up = self.upvalue(u);
                    let value = match up.borrow().closed.clone() {
                        Some(v) => v,
                        None => self.stack[up.borrow().slot].clone(),
                    };
                    self.push(value);
                }
                OpCode::SetUpvalue(u) => {
                    // Assignment is an expression: keep the value on the stack.
                    let value = self.stack.last().unwrap().clone();
                    let up = self.upvalue(u);
                    if up.borrow().closed.is_some() {
                        up.borrow_mut().closed = Some(value);
                    } else {
                        let slot = up.borrow().slot;
                        self.stack[slot] = value;
                    }
                }
                OpCode::GetIndex => {
                    let key = self.pop();
                    let container = self.pop();
                    let result = index_get(&container, &key)?;
                    self.push(result);
                }
                OpCode::SetIndex => {
                    // Stack: [.., container, key, value] with value on top.
                    // Assignment is an expression, so after storing, push the
                    // value back (statements pop it later).
                    let value = self.pop();
                    let key = self.pop();
                    let container = self.pop();
                    index_set(container, key, value.clone())?;
                    self.push(value);
                }
                OpCode::GetField(c) => {
                    let name = self.constant_str(c);
                    let receiver = self.pop();
                    match stdlib::lookup_method(&receiver, &name) {
                        Some(method) => self.push(Value::NativeMethod {
                            receiver: Box::new(receiver),
                            method,
                        }),
                        None => {
                            return Err(self.runtime_error(
                                format!(
                                    "value of type {} has no method '{}'",
                                    receiver.type_name(),
                                    name
                                ),
                                line,
                            ));
                        }
                    }
                }
                OpCode::Add => self.binary_add(line)?,
                OpCode::Subtract => self.binary_num(|x, y| x - y, "subtract", line)?,
                OpCode::Multiply => self.binary_num(|x, y| x * y, "multiply", line)?,
                OpCode::Divide => self.binary_num(|x, y| x / y, "divide", line)?,
                OpCode::Modulo => self.binary_num(|x, y| x % y, "take the remainder of", line)?,
                OpCode::Power => self.binary_num(|x, y| x.powf(y), "raise", line)?,
                OpCode::Negate => {
                    let v = self.pop();
                    match v {
                        Value::Num(n) => self.push(Value::num(-n)),
                        other => {
                            return Err(self.runtime_error(
                                format!("cannot negate a value of type {}", other.type_name()),
                                line,
                            ));
                        }
                    }
                }
                OpCode::Not => {
                    let v = self.pop();
                    self.push(Value::Bool(!v.is_truthy()));
                }
                OpCode::Equal => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(a == b));
                }
                OpCode::NotEqual => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(a != b));
                }
                OpCode::Less => self.binary_compare(|x, y| x < y, |x, y| x < y, "compare", line)?,
                OpCode::LessEqual => {
                    self.binary_compare(|x, y| x <= y, |x, y| x <= y, "compare", line)?
                }
                OpCode::Greater => {
                    self.binary_compare(|x, y| x > y, |x, y| x > y, "compare", line)?
                }
                OpCode::GreaterEqual => {
                    self.binary_compare(|x, y| x >= y, |x, y| x >= y, "compare", line)?
                }
                OpCode::Jump { target } => {
                    self.current_frame_mut().ip = target;
                }
                OpCode::JumpIfFalse { target } => {
                    let cond = self.pop();
                    if !cond.is_truthy() {
                        self.current_frame_mut().ip = target;
                    }
                }
                OpCode::Loop { target } => {
                    self.current_frame_mut().ip = target;
                }
                OpCode::Call(n) => self.call(n as usize, line)?,
                OpCode::Return => {
                    let result = self.pop();
                    let frame = self.frames.pop().expect("frame underflow");
                    self.close_upvalues_from(frame.slots);
                    self.stack.truncate(frame.base);
                    self.stack.push(result);
                    if self.frames.is_empty() {
                        return Ok(());
                    }
                }
                OpCode::Closure {
                    function: c,
                    upvalues: descs,
                } => {
                    let function = match &self.constant(c) {
                        Value::Function(f) => f.clone(),
                        _ => unreachable!("closure constant is not a function"),
                    };
                    let slots = self.current_frame().slots;
                    let mut captured = Vec::with_capacity(descs.len());
                    for desc in descs {
                        if desc.is_local {
                            captured.push(self.capture_upvalue(slots + desc.index as usize));
                        } else {
                            let up = self.current_frame().upvalues[desc.index as usize].clone();
                            captured.push(up);
                        }
                    }
                    self.push(Value::Closure {
                        function,
                        upvalues: captured,
                    });
                }
                OpCode::CloseUpvalue => {
                    let top = self.stack.len() - 1;
                    self.close_upvalue_at(top);
                    self.pop();
                }
                OpCode::Rotate3 => {
                    // [a, b, c] -> [b, c, a]: used to put the result of a
                    // compound indexed assignment under its container+key.
                    let c = self.pop();
                    let b = self.pop();
                    let a = self.pop();
                    self.push(b);
                    self.push(c);
                    self.push(a);
                }
                OpCode::BuildList(n) => {
                    let count = n as usize;
                    let start = self.stack.len() - count;
                    let items = self.stack.split_off(start);
                    self.push(Value::List(Rc::new(RefCell::new(items))));
                }
                OpCode::BuildMap(n) => {
                    let count = n as usize;
                    let start = self.stack.len() - 2 * count;
                    let vals = self.stack.split_off(start);
                    // Only immutable values (Nil/Bool/Num/Str) are hashable,
                    // so the interior-mutability clippy warning is a false
                    // positive: mutable values can never become map keys.
                    #[allow(clippy::mutable_key_type)]
                    let mut map = HashMap::with_capacity(count);
                    for i in (0..vals.len()).step_by(2) {
                        let key = vals[i].clone();
                        if !key.is_hashable() {
                            return Err(self.runtime_error(
                                format!("invalid map key of type {}", key.type_name()),
                                line,
                            ));
                        }
                        map.insert(key, vals[i + 1].clone());
                    }
                    self.push(Value::Map(Rc::new(RefCell::new(map))));
                }
                OpCode::BuildRange { inclusive } => {
                    let end = self.pop();
                    let start = self.pop();
                    let (start, end) = match (start, end) {
                        (Value::Num(s), Value::Num(e)) => (s, e),
                        (a, b) => {
                            return Err(self.runtime_error(
                                format!(
                                    "range bounds must be numbers, got {} and {}",
                                    a.type_name(),
                                    b.type_name()
                                ),
                                line,
                            ));
                        }
                    };
                    self.push(Value::range(start, end, inclusive));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().expect("value stack underflow")
    }

    fn current_frame(&self) -> &Frame {
        self.frames.last().expect("frame underflow")
    }

    fn current_frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("frame underflow")
    }

    fn constant(&self, idx: u32) -> &Value {
        &self.current_frame().function.chunk.constants[idx as usize]
    }

    fn constant_str(&self, idx: u32) -> String {
        match self.constant(idx) {
            Value::Str(s) => s.to_string(),
            _ => unreachable!("constant is not a string"),
        }
    }

    fn upvalue(&self, idx: u8) -> Rc<RefCell<Upvalue>> {
        self.current_frame().upvalues[idx as usize].clone()
    }

    fn capture_upvalue(&mut self, slot: usize) -> Rc<RefCell<Upvalue>> {
        if let Some(up) = self.open_upvalues.get(&slot) {
            return up.clone();
        }
        let up = Rc::new(RefCell::new(Upvalue::open(slot)));
        self.open_upvalues.insert(slot, up.clone());
        up
    }

    fn close_upvalue_at(&mut self, slot: usize) {
        if let Some(up) = self.open_upvalues.remove(&slot) {
            let value = self.stack[slot].clone();
            up.borrow_mut().closed = Some(value);
        }
    }

    fn close_upvalues_from(&mut self, slot: usize) {
        let keys: Vec<usize> = self
            .open_upvalues
            .keys()
            .copied()
            .filter(|k| *k >= slot)
            .collect();
        for k in keys {
            self.close_upvalue_at(k);
        }
    }

    fn runtime_error(&self, message: String, line: u32) -> RuntimeError {
        let mut trace = Vec::new();
        let top = self.frames.len().saturating_sub(1);
        for (i, frame) in self.frames.iter().enumerate().rev() {
            let l = if i == top {
                line
            } else {
                frame
                    .function
                    .chunk
                    .lines
                    .get(frame.ip.saturating_sub(1))
                    .copied()
                    .unwrap_or(line)
            };
            trace.push(TraceFrame {
                function: frame.function.name.clone(),
                file: frame.function.file.clone(),
                line: l,
            });
        }
        RuntimeError { message, trace }
    }

    fn call(&mut self, arg_count: usize, line: u32) -> RuntimeResult<()> {
        let callee_idx = self
            .stack
            .len()
            .checked_sub(arg_count + 1)
            .expect("call underflow");
        match self.stack[callee_idx].clone() {
            Value::Closure {
                function,
                upvalues,
            } => {
                if function.arity as usize != arg_count {
                    return Err(self.runtime_error(
                        format!(
                            "{} expects {} argument(s) but got {}",
                            function.name, function.arity, arg_count
                        ),
                        line,
                    ));
                }
                if self.frames.len() >= self.max_frames {
                    return Err(self.runtime_error(
                        "stack overflow: too many nested calls".to_string(),
                        line,
                    ));
                }
                self.frames.push(Frame {
                    function,
                    upvalues,
                    ip: 0,
                    slots: callee_idx + 1,
                    base: callee_idx,
                });
            }
            Value::Native { fun, .. } => {
                let args = self.stack.split_off(callee_idx);
                match fun(self, &args[1..]) {
                    Ok(v) => self.push(v),
                    Err(e) => return Err(self.runtime_error(e.message, line)),
                }
            }
            Value::NativeMethod { receiver, method } => {
                let mut popped = self.stack.split_off(callee_idx);
                popped.remove(0);
                // Route through the Corros-written standard library ($method
                // from lib/prelude.cor) when it is loaded, so methods are
                // implemented in Corros itself. Without the prelude (e.g.
                // pre-compiled bytecode), fall back to the native method.
                if let Some(mv) = self.globals.get("$method").cloned() {
                    self.push(mv);
                    self.push(*receiver);
                    self.push(Value::str(method.name.to_string()));
                    self.push(Value::List(Rc::new(RefCell::new(popped))));
                    return self.call(3, line);
                }
                match (method.call)(&receiver, &popped) {
                    Ok(v) => self.push(v),
                    Err(e) => return Err(self.runtime_error(e.message, line)),
                }
            }
            other => {
                return Err(self.runtime_error(
                    format!("cannot call a value of type {}", other.type_name()),
                    line,
                ));
            }
        }
        Ok(())
    }

    fn binary_add(&mut self, line: u32) -> RuntimeResult<()> {
        let b = self.pop();
        let a = self.pop();
        match (a, b) {
            (Value::Num(x), Value::Num(y)) => self.push(Value::num(x + y)),
            (Value::Str(x), Value::Str(y)) => {
                self.push(Value::Str(format!("{}{}", x, y).into()));
            }
            (Value::List(x), Value::List(y)) => {
                let mut items = x.borrow().clone();
                items.extend(y.borrow().iter().cloned());
                self.push(Value::List(Rc::new(RefCell::new(items))));
            }
            (a, b) => {
                return Err(self.runtime_error(
                    format!("cannot add {} and {}", a.type_name(), b.type_name()),
                    line,
                ));
            }
        }
        Ok(())
    }

    fn binary_num(
        &mut self,
        f: impl Fn(f64, f64) -> f64,
        verb: &str,
        line: u32,
    ) -> RuntimeResult<()> {
        let b = self.pop();
        let a = self.pop();
        match (a, b) {
            (Value::Num(x), Value::Num(y)) => {
                self.push(Value::num(f(x, y)));
                Ok(())
            }
            (a, b) => Err(self.runtime_error(
                format!(
                    "cannot {} {} and {}",
                    verb,
                    a.type_name(),
                    b.type_name()
                ),
                line,
            )),
        }
    }

    fn binary_compare(
        &mut self,
        num_cmp: impl Fn(f64, f64) -> bool,
        str_cmp: impl Fn(&str, &str) -> bool,
        verb: &str,
        line: u32,
    ) -> RuntimeResult<()> {
        let b = self.pop();
        let a = self.pop();
        match (&a, &b) {
            (Value::Num(x), Value::Num(y)) => {
                self.push(Value::Bool(num_cmp(*x, *y)));
                Ok(())
            }
            (Value::Str(x), Value::Str(y)) => {
                self.push(Value::Bool(str_cmp(x, y)));
                Ok(())
            }
            _ => Err(self.runtime_error(
                format!("cannot {} {} and {}", verb, a.type_name(), b.type_name()),
                line,
            )),
        }
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Indexing
// ---------------------------------------------------------------------------

fn index_from_value(key: &Value) -> RuntimeResult<usize> {
    match key {
        Value::Num(n) if n.fract() == 0.0 && *n >= 0.0 => Ok(*n as usize),
        _ => Err(RuntimeError::new(format!(
            "index must be a non-negative integer, got {}",
            key.type_name()
        ))),
    }
}

fn index_get(container: &Value, key: &Value) -> RuntimeResult<Value> {
    match container {
        Value::List(items) => {
            let i = index_from_value(key)?;
            match items.borrow().get(i) {
                Some(v) => Ok(v.clone()),
                None => Err(RuntimeError::new(format!(
                    "index out of bounds: {} (list has {} elements)",
                    i,
                    items.borrow().len()
                ))),
            }
        }
        Value::Str(s) => {
            let i = index_from_value(key)?;
            match s.chars().nth(i) {
                Some(c) => Ok(Value::str(c.to_string())),
                None => Err(RuntimeError::new(format!(
                    "index out of bounds: {} (string has {} characters)",
                    i,
                    s.chars().count()
                ))),
            }
        }
        Value::Range { start, end, inclusive } => {
            let i = index_from_value(key)?;
            let len = range_len(*start, *end, *inclusive);
            if i < len {
                Ok(Value::num(start + i as f64))
            } else {
                Err(RuntimeError::new(format!(
                    "index out of bounds: {} (range has {} elements)",
                    i, len
                )))
            }
        }
        Value::Map(entries) => match entries.borrow().get(key) {
            Some(v) => Ok(v.clone()),
            None => Err(RuntimeError::new(format!("map has no key {}", key.repr()))),
        },
        other => Err(RuntimeError::new(format!(
            "cannot index a value of type {}",
            other.type_name()
        ))),
    }
}

fn index_set(container: Value, key: Value, value: Value) -> RuntimeResult<()> {
    match container {
        Value::List(items) => {
            let i = index_from_value(&key)?;
            let mut items = items.borrow_mut();
            if i < items.len() {
                items[i] = value;
                Ok(())
            } else {
                Err(RuntimeError::new(format!(
                    "index out of bounds: {} (list has {} elements)",
                    i,
                    items.len()
                )))
            }
        }
        Value::Map(entries) => {
            if !key.is_hashable() {
                return Err(RuntimeError::new(format!(
                    "invalid map key of type {}",
                    key.type_name()
                )));
            }
            entries.borrow_mut().insert(key, value);
            Ok(())
        }
        Value::Str(_) => Err(RuntimeError::new("strings are immutable")),
        other => Err(RuntimeError::new(format!(
            "cannot index a value of type {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn range_len(start: f64, end: f64, inclusive: bool) -> usize {
    let count = if inclusive { end - start + 1.0 } else { end - start };
    if count <= 0.0 {
        0
    } else {
        count as usize
    }
}
