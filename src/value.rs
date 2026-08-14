//! Runtime values for Corros.
//!
//! Corros is dynamically typed. All values are represented by the [`Value`] enum.
//! Collections (`List`, `Map`) and callables are reference-counted (`Rc`), so
//! copying a value is cheap. Strings are reference-counted too, which makes
//! concatenation and passing strings around fast.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::chunk::Function;
use crate::error::RuntimeResult;
use crate::vm::VM;

/// Signature of a native (built-in) Corros function.
pub type NativeFn = fn(&mut VM, &[Value]) -> RuntimeResult<Value>;

/// Signature of a native method on a collection (list, string, map, range).
/// The receiver is passed as the first argument.
pub type MethodFn = fn(&Value, &[Value]) -> RuntimeResult<Value>;

/// A native method definition, looked up by name at runtime.
#[derive(Debug, Clone, Copy)]
pub struct MethodDef {
    pub name: &'static str,
    pub call: MethodFn,
}

/// An upvalue: a variable captured by a closure from an enclosing scope.
///
/// While the captured variable is still on the VM's value stack (i.e. the
/// enclosing function hasn't returned), the upvalue is "open" and reads/writes
/// go straight to the stack slot at `slot`. When the enclosing scope exits, the
/// value is copied into `closed` and the upvalue becomes "closed".
#[derive(Debug)]
pub struct Upvalue {
    /// Stack slot this upvalue captures while open.
    pub slot: usize,
    pub closed: Option<Value>,
}

impl Upvalue {
    pub fn open(slot: usize) -> Self {
        Upvalue { slot, closed: None }
    }
}

/// Every runtime value in Corros.
#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<HashMap<Value, Value>>>),
    Range {
        start: f64,
        end: f64,
        inclusive: bool,
    },
    /// A compiled Corros function, used as a constant (referenced by OP_CLOSURE).
    Function(Rc<Function>),
    /// A Corros function together with the upvalues it captured.
    Closure {
        function: Rc<Function>,
        upvalues: Vec<Rc<RefCell<Upvalue>>>,
    },
    /// A built-in function implemented in Rust.
    Native { name: &'static str, fun: NativeFn },
    /// A built-in method bound to a receiver (e.g. `list.push`).
    NativeMethod {
        receiver: Box<Value>,
        method: &'static MethodDef,
    },
}

impl Value {
    pub fn num(n: f64) -> Value {
        Value::Num(n)
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(Rc::<str>::from(s.into()))
    }

    pub fn list() -> Value {
        Value::List(Rc::new(RefCell::new(Vec::new())))
    }

    pub fn map() -> Value {
        Value::Map(Rc::new(RefCell::new(HashMap::new())))
    }

    pub fn range(start: f64, end: f64, inclusive: bool) -> Value {
        Value::Range { start, end, inclusive }
    }

    /// The name of this value's type, as returned by the `nature()` builtin.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Num(_) => "num",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Range { .. } => "range",
            Value::Function(_)
            | Value::Closure { .. }
            | Value::Native { .. }
            | Value::NativeMethod { .. } => "function",
        }
    }

    /// Can this value be used as a map key?
    pub fn is_hashable(&self) -> bool {
        matches!(self, Value::Nil | Value::Bool(_) | Value::Num(_) | Value::Str(_))
    }

    /// Corros truthiness: `false`, `nil`, `0`, `""`, `[]`, and `{}` are falsy.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Nil | Value::Bool(false) => false,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Map(m) => !m.borrow().is_empty(),
            _ => true,
        }
    }

    /// A quoted, debug-oriented rendering (strings get quotes and escapes).
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", escape_str(s)),
            _ => self.to_string(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Num(n) => write!(f, "{}", format_num(*n)),
            Value::Str(s) => write!(f, "{}", s),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Map(entries) => {
                write!(f, "{{")?;
                for (i, (k, v)) in entries.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k.repr(), v)?;
                }
                write!(f, "}}")
            }
            Value::Range { start, end, inclusive } => {
                write!(
                    f,
                    "{}{}{}",
                    format_num(*start),
                    if *inclusive { "..=" } else { ".." },
                    format_num(*end)
                )
            }
            Value::Function(function) => write!(f, "{}", function),
            Value::Closure { function, .. } => write!(f, "{}", function),
            Value::Native { name, .. } => write!(f, "<native {}>", name),
            Value::NativeMethod { method, .. } => write!(f, "<method {}>", method.name),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
            (Value::Map(a), Value::Map(b)) => Rc::ptr_eq(a, b),
            (
                Value::Range { start: a1, end: a2, inclusive: a3 },
                Value::Range { start: b1, end: b2, inclusive: b3 },
            ) => a1 == b1 && a2 == b2 && a3 == b3,
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::Closure { function: a, .. }, Value::Closure { function: b, .. }) => {
                Rc::ptr_eq(a, b)
            }
            (Value::Native { fun: a, .. }, Value::Native { fun: b, .. }) => {
                std::ptr::eq(*a as *const (), *b as *const ())
            }
            (
                Value::NativeMethod { receiver: a, method: ma },
                Value::NativeMethod { receiver: b, method: mb },
            ) => a == b && std::ptr::eq(*ma, *mb),
            _ => false,
        }
    }
}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Nil => 0u8.hash(state),
            Value::Bool(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            Value::Num(n) => {
                2u8.hash(state);
                // Normalize -0.0 so it hashes identically to 0.0 (they compare equal).
                let bits = if *n == 0.0 { 0.0f64.to_bits() } else { n.to_bits() };
                bits.hash(state);
            }
            Value::Str(s) => {
                3u8.hash(state);
                s.hash(state);
            }
            _ => unreachable!("non-hashable value used as a map key"),
        }
    }
}

impl Eq for Value {}

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

/// Escape a string for debug output.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_numbers() {
        assert_eq!(format_num(5.0), "5");
        assert_eq!(format_num(5.5), "5.5");
        assert_eq!(format_num(-0.0), "0");
        assert_eq!(format_num(1e20), "100000000000000000000");
        assert_eq!(format_num(f64::NAN), "nan");
    }

    #[test]
    fn truthiness() {
        assert!(!Value::Nil.is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(!Value::num(0.0).is_truthy());
        assert!(Value::num(0.5).is_truthy());
        assert!(!Value::str("").is_truthy());
        assert!(Value::str("x").is_truthy());
        assert!(!Value::list().is_truthy());
        assert!(Value::range(0.0, 3.0, false).is_truthy());
    }

    #[test]
    fn numeric_equality() {
        assert_eq!(Value::num(1.0), Value::num(1.0));
        assert_ne!(Value::num(1.0), Value::num(2.0));
        assert_ne!(Value::num(1.0), Value::str("1"));
    }

    #[test]
    // Value keys here are guaranteed hashable (numbers only), so the lint is a
    // false positive for this test.
    #[allow(clippy::mutable_key_type)]
    fn negative_zero_hashes_like_zero() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Value::num(0.0));
        set.insert(Value::num(-0.0));
        assert_eq!(set.len(), 1);
    }
}
