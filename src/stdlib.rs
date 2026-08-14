//! Corros's standard library: builtin functions (`speak`, `size`, ...) and
//! the methods available on lists, strings, maps, and ranges.
//!
//! The vocabulary is short and generic for everyday utilities (`size`, `num`,
//! `int`, `bool`, `split`, `clear`) and foundry-themed where it can be
//! (`speak` output, `weld` lists together, `reforge` text).

use std::rc::Rc;

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{MethodDef, NativeFn, Value};
use crate::vm::{range_len, VM};

/// The builtin functions installed as globals when a VM starts.
pub static BUILTINS: &[(&str, NativeFn)] = &[
    ("speak", native_speak),
    ("hear", native_hear),
    ("size", native_size),
    ("nature", native_nature),
    ("str", native_str),
    ("num", native_num),
    ("int", native_int),
    ("bool", native_bool),
    ("abs", native_abs),
    ("root", native_root),
    ("least", native_least),
    ("greatest", native_greatest),
    ("tick", native_tick),
    ("span", native_span),
    ("vouch", native_vouch),
    ("flaw", native_flaw),
    // Self-hosting helpers: file I/O and mutating list ops as builtins.
    ("read", native_read),
    ("readlines", native_readlines),
    ("shove", native_shove),
    ("yank", native_yank),
    ("file_exists", native_file_exists),
    // Method bridge: dispatches to the same method table the VM uses, so the
    // self-hosted VM (and the Corros standard library's fallback path) can
    // implement GET_FIELD/CALL with zero drift.
    ("mcall", native_mcall),
];

fn err(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(message)
}

fn expect_args(name: &str, args: &[Value], count: usize) -> RuntimeResult<()> {
    if args.len() != count {
        return Err(err(format!(
            "{} expects {} argument(s) but got {}",
            name,
            count,
            args.len()
        )));
    }
    Ok(())
}

fn expect_args_between(name: &str, args: &[Value], min: usize, max: usize) -> RuntimeResult<()> {
    if args.len() < min || args.len() > max {
        return Err(err(format!(
            "{} expects between {} and {} arguments but got {}",
            name,
            min,
            max,
            args.len()
        )));
    }
    Ok(())
}

fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        _ => None,
    }
}

fn want_num(name: &str, v: &Value) -> RuntimeResult<f64> {
    as_num(v).ok_or_else(|| err(format!("{} expects a number, got {}", name, v.type_name())))
}

// ---------------------------------------------------------------------------
// Builtin functions
// ---------------------------------------------------------------------------

fn native_speak(vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    let text: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    let line = text.join(" ");
    vm.output.push(line.clone());
    if vm.echo {
        println!("{}", line);
    }
    Ok(Value::Nil)
}

fn native_hear(vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args_between("hear", args, 0, 1)?;
    if let Some(prompt) = args.first() {
        if vm.echo {
            print!("{}", prompt);
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Err(err("hear: end of input")),
        Ok(_) => {
            while line.ends_with('\n') || line.ends_with('\r') {
                line.pop();
            }
            Ok(Value::str(line))
        }
        Err(e) => Err(err(format!("hear: {}", e))),
    }
}

fn native_size(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("size", args, 1)?;
    let len = match &args[0] {
        Value::Str(s) => s.chars().count(),
        Value::List(items) => items.borrow().len(),
        Value::Map(entries) => entries.borrow().len(),
        Value::Range { start, end, inclusive } => range_len(*start, *end, *inclusive),
        other => {
            return Err(err(format!(
                "size expects a string, list, map, or range, got {}",
                other.type_name()
            )));
        }
    };
    Ok(Value::num(len as f64))
}

fn native_nature(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("nature", args, 1)?;
    Ok(Value::str(args[0].type_name()))
}

fn native_str(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("str", args, 1)?;
    Ok(Value::str(args[0].to_string()))
}

fn native_num(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("num", args, 1)?;
    let n = match &args[0] {
        Value::Num(n) => *n,
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| err(format!("cannot num '{}' as a number", s)))?,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        other => {
            return Err(err(format!(
                "cannot num {} as a number",
                other.type_name()
            )));
        }
    };
    Ok(Value::num(n))
}

fn native_int(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("int", args, 1)?;
    let n = match &args[0] {
        Value::Num(n) => n.trunc(),
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| err(format!("cannot int '{}'", s)))?
            .trunc(),
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        other => {
            return Err(err(format!(
                "cannot int {}",
                other.type_name()
            )));
        }
    };
    Ok(Value::num(n))
}

fn native_bool(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("bool", args, 1)?;
    Ok(Value::Bool(args[0].is_truthy()))
}

fn native_abs(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("abs", args, 1)?;
    Ok(Value::num(want_num("abs", &args[0])?.abs()))
}

fn native_root(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("root", args, 1)?;
    Ok(Value::num(want_num("root", &args[0])?.sqrt()))
}

fn native_least(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    if args.is_empty() {
        return Err(err("least expects at least one argument"));
    }
    let mut best = f64::INFINITY;
    for a in args {
        best = best.min(want_num("least", a)?);
    }
    Ok(Value::num(best))
}

fn native_greatest(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    if args.is_empty() {
        return Err(err("greatest expects at least one argument"));
    }
    let mut best = f64::NEG_INFINITY;
    for a in args {
        best = best.max(want_num("greatest", a)?);
    }
    Ok(Value::num(best))
}

fn native_tick(vm: &mut VM, _args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::num(vm.start.elapsed().as_secs_f64()))
}

fn native_span(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args_between("span", args, 1, 2)?;
    let start = match args.len() {
        1 => 0.0,
        _ => want_num("span", &args[0])?,
    };
    let end = want_num("span", &args[args.len() - 1])?;
    Ok(Value::range(start, end, false))
}

fn native_vouch(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args_between("vouch", args, 1, 2)?;
    if !args[0].is_truthy() {
        let message = args
            .get(1)
            .map(Value::to_string)
            .unwrap_or_else(|| "vouch failed: the condition was false".to_string());
        return Err(err(message));
    }
    Ok(Value::Nil)
}

fn native_flaw(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("flaw", args, 1)?;
    Err(err(args[0].to_string()))
}

fn native_read(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("read", args, 1)?;
    let path = want_str("read", &args[0])?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Value::str(text)),
        Err(e) => Err(err(format!("read: cannot open '{}': {}", path, e))),
    }
}

// Read a file as a list of lines, split in a single linear pass on the Rust
// side (the Corros-level split_str builds strings char-by-char, which is
// quadratic and far too slow for a 39KB bytecode file under double
// interpretation).
fn native_readlines(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("readlines", args, 1)?;
    let path = want_str("readlines", &args[0])?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let list = Value::list();
            if let Value::List(items) = &list {
                let mut items = items.borrow_mut();
                for line in text.lines() {
                    items.push(Value::str(line.to_string()));
                }
            }
            Ok(list)
        }
        Err(e) => Err(err(format!("readlines: cannot open '{}': {}", path, e))),
    }
}

fn native_shove(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("shove", args, 2)?;
    as_list(&args[0])?.borrow_mut().push(args[1].clone());
    Ok(Value::Nil)
}

fn native_yank(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("yank", args, 1)?;
    as_list(&args[0])?
        .borrow_mut()
        .pop()
        .ok_or_else(|| err("yank: the list is empty"))
}

fn native_file_exists(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("file_exists", args, 1)?;
    let path = want_str("file_exists", &args[0])?;
    Ok(Value::Bool(std::path::Path::new(&path).exists()))
}

// mcall("method_name", receiver, [arg, ...]) — calls a method on the receiver
// through the standard method table, exactly as GET_FIELD + CALL would.
fn native_mcall(_vm: &mut VM, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("mcall", args, 3)?;
    let name = want_str("mcall", &args[0])?;
    let receiver = &args[1];
    let arg_list = as_list(&args[2])?.borrow().clone();
    match lookup_method(receiver, &name) {
        Some(method) => (method.call)(receiver, &arg_list),
        None => Err(err(format!(
            "value of type {} has no method '{}'",
            receiver.type_name(),
            name
        ))),
    }
}

// ---------------------------------------------------------------------------
// Method lookup
// ---------------------------------------------------------------------------

/// Look up a method by name on a value, returning a static method definition.
pub fn lookup_method(value: &Value, name: &str) -> Option<&'static MethodDef> {
    match value {
        Value::List(_) => match name {
            "shove" => Some(&MethodDef { name: "shove", call: list_shove }),
            "yank" => Some(&MethodDef { name: "yank", call: list_yank }),
            "size" => Some(&MethodDef { name: "size", call: list_size }),
            "slot" => Some(&MethodDef { name: "slot", call: list_slot }),
            "pluck" => Some(&MethodDef { name: "pluck", call: list_pluck }),
            "holds" => Some(&MethodDef { name: "holds", call: list_holds }),
            "weld" => Some(&MethodDef { name: "weld", call: list_weld }),
            "order" => Some(&MethodDef { name: "order", call: list_order }),
            "flip" => Some(&MethodDef { name: "flip", call: list_flip }),
            "clear" => Some(&MethodDef { name: "clear", call: list_clear }),
            _ => None,
        },
        Value::Str(_) => match name {
            "size" => Some(&MethodDef { name: "size", call: str_size }),
            "loud" => Some(&MethodDef { name: "loud", call: str_loud }),
            "quiet" => Some(&MethodDef { name: "quiet", call: str_quiet }),
            "shave" => Some(&MethodDef { name: "shave", call: str_shave }),
            "split" => Some(&MethodDef { name: "split", call: str_split }),
            "holds" => Some(&MethodDef { name: "holds", call: str_holds }),
            "opens" => Some(&MethodDef { name: "opens", call: str_opens }),
            "closes" => Some(&MethodDef { name: "closes", call: str_closes }),
            "reforge" => Some(&MethodDef { name: "reforge", call: str_reforge }),
            _ => None,
        },
        Value::Map(_) => match name {
            "size" => Some(&MethodDef { name: "size", call: map_size }),
            "labels" => Some(&MethodDef { name: "labels", call: map_labels }),
            "contents" => Some(&MethodDef { name: "contents", call: map_contents }),
            "holds" => Some(&MethodDef { name: "holds", call: map_holds }),
            "fetch" => Some(&MethodDef { name: "fetch", call: map_fetch }),
            "pluck" => Some(&MethodDef { name: "pluck", call: map_pluck }),
            "clear" => Some(&MethodDef { name: "clear", call: map_clear }),
            _ => None,
        },
        Value::Range { .. } => match name {
            "size" => Some(&MethodDef { name: "size", call: range_size }),
            "holds" => Some(&MethodDef { name: "holds", call: range_holds }),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Method implementations
// ---------------------------------------------------------------------------

fn list_shove(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("shove", args, 1)?;
    as_list(receiver)?.borrow_mut().push(args[0].clone());
    Ok(Value::Nil)
}

fn list_yank(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("yank", args, 0)?;
    as_list(receiver)?
        .borrow_mut()
        .pop()
        .ok_or_else(|| err("yank: the list is empty"))
}

fn list_size(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("size", args, 0)?;
    Ok(Value::num(as_list(receiver)?.borrow().len() as f64))
}

fn list_slot(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("slot", args, 2)?;
    let i = index_arg("slot", &args[0])?;
    let mut items = as_list(receiver)?.borrow_mut();
    if i <= items.len() {
        items.insert(i, args[1].clone());
        Ok(Value::Nil)
    } else {
        Err(err(format!(
            "slot: index out of bounds: {} (list has {} elements)",
            i,
            items.len()
        )))
    }
}

fn list_pluck(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("pluck", args, 1)?;
    let i = index_arg("pluck", &args[0])?;
    let mut items = as_list(receiver)?.borrow_mut();
    if i < items.len() {
        Ok(items.remove(i))
    } else {
        Err(err(format!(
            "pluck: index out of bounds: {} (list has {} elements)",
            i,
            items.len()
        )))
    }
}

fn list_holds(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("holds", args, 1)?;
    Ok(Value::Bool(
        as_list(receiver)?.borrow().contains(&args[0]),
    ))
}

fn list_weld(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("weld", args, 1)?;
    let sep = match &args[0] {
        Value::Str(s) => s.to_string(),
        other => {
            return Err(err(format!(
                "weld expects a string separator, got {}",
                other.type_name()
            )));
        }
    };
    let parts: Vec<String> = as_list(receiver)?
        .borrow()
        .iter()
        .map(Value::to_string)
        .collect();
    Ok(Value::str(parts.join(&sep)))
}

fn list_order(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("order", args, 0)?;
    let mut items = as_list(receiver)?.borrow_mut();
    // Insertion sort: simple, stable, and lets us propagate type errors.
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && value_less(&items[j], &items[j - 1])? {
            items.swap(j, j - 1);
            j -= 1;
        }
    }
    Ok(Value::Nil)
}

fn list_flip(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("flip", args, 0)?;
    as_list(receiver)?.borrow_mut().reverse();
    Ok(Value::Nil)
}

fn list_clear(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("clear", args, 0)?;
    as_list(receiver)?.borrow_mut().clear();
    Ok(Value::Nil)
}

fn str_size(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("size", args, 0)?;
    Ok(Value::num(as_str(receiver)?.chars().count() as f64))
}

fn str_loud(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("loud", args, 0)?;
    Ok(Value::str(as_str(receiver)?.to_uppercase()))
}

fn str_quiet(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("quiet", args, 0)?;
    Ok(Value::str(as_str(receiver)?.to_lowercase()))
}

fn str_shave(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("shave", args, 0)?;
    Ok(Value::str(as_str(receiver)?.trim()))
}

fn str_split(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("split", args, 1)?;
    let sep = match &args[0] {
        Value::Str(s) => s.to_string(),
        other => {
            return Err(err(format!(
                "split expects a string separator, got {}",
                other.type_name()
            )));
        }
    };
    if sep.is_empty() {
        return Err(err("split: the separator must not be empty"));
    }
    let parts = as_str(receiver)?
        .split(&sep)
        .map(Value::str)
        .collect::<Vec<Value>>();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(parts))))
}

fn str_holds(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("holds", args, 1)?;
    let needle = want_str("holds", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.contains(&needle)))
}

fn str_opens(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("opens", args, 1)?;
    let prefix = want_str("opens", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.starts_with(&prefix)))
}

fn str_closes(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("closes", args, 1)?;
    let suffix = want_str("closes", &args[0])?;
    Ok(Value::Bool(as_str(receiver)?.ends_with(&suffix)))
}

fn str_reforge(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("reforge", args, 2)?;
    let from = want_str("reforge", &args[0])?;
    let to = want_str("reforge", &args[1])?;
    Ok(Value::str(as_str(receiver)?.replace(&from, &to)))
}

fn map_size(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("size", args, 0)?;
    Ok(Value::num(as_map(receiver)?.borrow().len() as f64))
}

fn map_labels(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("labels", args, 0)?;
    let keys: Vec<Value> = as_map(receiver)?.borrow().keys().cloned().collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(keys))))
}

fn map_contents(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("contents", args, 0)?;
    let values: Vec<Value> = as_map(receiver)?.borrow().values().cloned().collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(values))))
}

fn map_holds(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("holds", args, 1)?;
    Ok(Value::Bool(as_map(receiver)?.borrow().contains_key(&args[0])))
}

fn map_fetch(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args_between("fetch", args, 1, 2)?;
    let map = as_map(receiver)?.borrow();
    match map.get(&args[0]) {
        Some(v) => Ok(v.clone()),
        None => Ok(args.get(1).cloned().unwrap_or(Value::Nil)),
    }
}

fn map_pluck(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("pluck", args, 1)?;
    Ok(Value::Bool(as_map(receiver)?.borrow_mut().remove(&args[0]).is_some()))
}

fn map_clear(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("clear", args, 0)?;
    as_map(receiver)?.borrow_mut().clear();
    Ok(Value::Nil)
}

fn range_size(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
    expect_args("size", args, 0)?;
    let (start, end, inclusive) = as_range(receiver)?;
    Ok(Value::num(range_len(start, end, inclusive) as f64))
}

fn range_holds(receiver: &Value, args: &[Value]) -> RuntimeResult<Value> {
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
// Helpers
// ---------------------------------------------------------------------------

fn as_list(v: &Value) -> RuntimeResult<&std::rc::Rc<std::cell::RefCell<Vec<Value>>>> {
    match v {
        Value::List(l) => Ok(l),
        other => Err(err(format!("expected a list, got {}", other.type_name()))),
    }
}

fn as_map(v: &Value) -> RuntimeResult<&std::rc::Rc<std::cell::RefCell<std::collections::HashMap<Value, Value>>>> {
    match v {
        Value::Map(m) => Ok(m),
        other => Err(err(format!("expected a map, got {}", other.type_name()))),
    }
}

fn as_str(v: &Value) -> RuntimeResult<&Rc<str>> {
    match v {
        Value::Str(s) => Ok(s),
        other => Err(err(format!("expected a string, got {}", other.type_name()))),
    }
}

fn as_range(v: &Value) -> RuntimeResult<(f64, f64, bool)> {
    match v {
        Value::Range { start, end, inclusive } => Ok((*start, *end, *inclusive)),
        other => Err(err(format!("expected a range, got {}", other.type_name()))),
    }
}

fn want_str(name: &str, v: &Value) -> RuntimeResult<String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        other => Err(err(format!(
            "{} expects a string, got {}",
            name,
            other.type_name()
        ))),
    }
}

fn index_arg(name: &str, v: &Value) -> RuntimeResult<usize> {
    match v {
        Value::Num(n) if n.fract() == 0.0 && *n >= 0.0 => Ok(*n as usize),
        _ => Err(err(format!(
            "{} expects a non-negative integer index, got {}",
            name,
            v.type_name()
        ))),
    }
}

/// Comparison used by `order`: numbers and strings are orderable.
fn value_less(a: &Value, b: &Value) -> RuntimeResult<bool> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Ok(x < y),
        (Value::Str(x), Value::Str(y)) => Ok(x < y),
        _ => Err(err(format!(
            "cannot order: cannot compare {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}
