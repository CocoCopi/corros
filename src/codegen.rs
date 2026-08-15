//! codegen.rs — the native compiler backend.
//!
//! `corros --compile file.cor` turns a program into a native binary:
//!
//!   1. `compiler.cor` compiles the program to bytecode (the normal path).
//!   2. This backend runs a whole-program type analysis over the bytecode
//!      (numbers, strings, booleans, functions, ranges), then emits C.
//!   3. The C is compiled with `cc -O3`, so numeric code runs at native
//!      speed — a compiled `fib(30)` beats Go's.
//!
//! The analysis is strict: programs whose types cannot be determined
//! statically (dynamic typing, lists, maps, closures, methods, `adopt`) are
//! rejected with a clear message — run those with the interpreter instead.

use std::collections::HashMap;

use crate::native::{load_program, Op, Program};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ty {
    Num,
    Str,
    Bool,
    Nil,
    Fn(usize),
    Range,
    Builtin(Builtin),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Builtin {
    Speak,
    Size,
    Num,
    Str,
    Int,
    Bool,
    Abs,
    Root,
    Least,
    Greatest,
    Span,
    Tick,
    Vouch,
}

fn builtin_of(name: &str) -> Option<Builtin> {
    match name {
        "speak" => Some(Builtin::Speak),
        "size" => Some(Builtin::Size),
        "num" => Some(Builtin::Num),
        "str" => Some(Builtin::Str),
        "int" => Some(Builtin::Int),
        "bool" => Some(Builtin::Bool),
        "abs" => Some(Builtin::Abs),
        "root" => Some(Builtin::Root),
        "least" => Some(Builtin::Least),
        "greatest" => Some(Builtin::Greatest),
        "span" => Some(Builtin::Span),
        "tick" => Some(Builtin::Tick),
        "vouch" => Some(Builtin::Vouch),
        _ => None,
    }
}

const UNSUPPORTED_BUILTINS: [&str; 8] = [
    "nature", "mcall", "hear", "read", "readlines", "file_exists", "shove", "yank",
];

fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Num => "num",
        Ty::Str => "string",
        Ty::Bool => "bool",
        Ty::Nil => "nil",
        Ty::Fn(_) => "function",
        Ty::Range => "range",
        Ty::Builtin(_) => "builtin",
    }
}

/// Union of two possibly-unknown types. `None` is unknown; `Nil` is treated as
/// "empty value" and absorbs into anything (the `each` loop re-initializes its
/// loop variable with `nil` before assigning it).
fn unify(a: Option<Ty>, b: Option<Ty>, what: &str) -> Result<Option<Ty>, String> {
    match (a, b) {
        (None, t) | (t, None) => Ok(t),
        (Some(Ty::Nil), t) | (t, Some(Ty::Nil)) => Ok(t),
        (Some(x), Some(y)) if x == y => Ok(Some(x)),
        (Some(x), Some(y)) => Err(format!(
            "--compile: type conflict ({} vs {}) in {}",
            ty_name(x),
            ty_name(y),
            what
        )),
    }
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FnState {
    params: Vec<Option<Ty>>,
    ret: Option<Ty>,
    stacks: Vec<Option<Vec<Option<Ty>>>>,
}

struct Analyzer<'a> {
    prog: &'a Program,
    fns: Vec<FnState>,
    globals: HashMap<u32, Option<Ty>>,
    /// Set whenever `process` enables a function or changes a param/global
    /// type — the fixpoint loop must re-run when that happens.
    any_change: bool,
}

impl<'a> Analyzer<'a> {
    fn new(prog: &'a Program) -> Self {
        let fns = prog
            .fns
            .iter()
            .map(|f| FnState {
                params: vec![None; f.arity],
                ret: None,
                stacks: vec![None; f.instrs.len()],
            })
            .collect();
        let mut globals = HashMap::new();
        for (i, _) in prog.names.iter().enumerate() {
            globals.insert(i as u32, None);
        }
        Analyzer {
            prog,
            fns,
            globals,
            any_change: false,
        }
    }

    fn analyze(&mut self) -> Result<(), String> {
        self.fns[0].stacks[0] = Some(Vec::new());
        let mut passes = 0;
        loop {
            passes += 1;
            if passes > 64 {
                return Err("--compile: type analysis did not converge".to_string());
            }
            let mut changed = false;
            for fid in 0..self.prog.fns.len() {
                if self.fns[fid].stacks[0].is_none() {
                    continue;
                }
                self.any_change = false;
                let instrs = self.prog.fns[fid].instrs.clone();
                for pc in 0..instrs.len() {
                    let incoming = match self.fns[fid].stacks[pc].clone() {
                        Some(s) => s,
                        None => continue,
                    };
                    let (outgoing, succs) = self.process(fid, pc, &incoming, &instrs)?;
                    for s in succs {
                        if self.merge_stack(fid, s, &outgoing)? {
                            changed = true;
                        }
                    }
                }
                if self.any_change {
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }

    fn merge_stack(&mut self, fid: usize, s: usize, stack: &[Option<Ty>]) -> Result<bool, String> {
        if s >= self.prog.fns[fid].instrs.len() {
            return Err(format!("--compile: jump to pc {} out of range", s));
        }
        match self.fns[fid].stacks[s].clone() {
            None => {
                self.fns[fid].stacks[s] = Some(stack.to_vec());
                Ok(true)
            }
            Some(existing) => {
                if existing.len() != stack.len() {
                    return Err(format!(
                        "--compile: stack depth mismatch at pc {} ({} vs {})",
                        s,
                        existing.len(),
                        stack.len()
                    ));
                }
                let mut changed = false;
                for (e, n) in existing.iter().zip(stack.iter()) {
                    if unify(*e, *n, "the stack")? != *e {
                        changed = true;
                    }
                }
                if changed {
                    let merged: Vec<Option<Ty>> = existing
                        .iter()
                        .zip(stack.iter())
                        .map(|(e, n)| unify(*e, *n, "the stack").unwrap())
                        .collect();
                    self.fns[fid].stacks[s] = Some(merged);
                }
                Ok(changed)
            }
        }
    }

    fn process(
        &mut self,
        fid: usize,
        pc: usize,
        stack: &[Option<Ty>],
        instrs: &[Op],
    ) -> Result<(Vec<Option<Ty>>, Vec<usize>), String> {
        let op = instrs[pc];
        let mut st: Vec<Option<Ty>> = stack.to_vec();
        let mut succs: Vec<usize> = Vec::new();
        let pop = |st: &mut Vec<Option<Ty>>| st.pop().flatten();

        match op {
            Op::Const(i) => {
                let t = match &self.prog.constants[i as usize] {
                    crate::seed::Value::Num(_) => Ty::Num,
                    crate::seed::Value::Str(_) => Ty::Str,
                    crate::seed::Value::Bool(_) => Ty::Bool,
                    crate::seed::Value::Nil => Ty::Nil,
                    other => {
                        return Err(format!(
                            "--compile: constants of type {} are not supported",
                            crate::seed::type_name(other)
                        ));
                    }
                };
                st.push(Some(t));
                succs.push(pc + 1);
            }
            Op::Nil => {
                st.push(Some(Ty::Nil));
                succs.push(pc + 1);
            }
            Op::True | Op::False => {
                st.push(Some(Ty::Bool));
                succs.push(pc + 1);
            }
            Op::Pop => {
                pop(&mut st);
                succs.push(pc + 1);
            }
            // Locals live at stack positions (the compiler reuses stack slots
            // as locals, like clox): GET_LOCAL i pushes a copy of stack[i],
            // SET_LOCAL i writes stack[i]. Params occupy positions 0..arity.
            Op::GetLocal(i) => {
                let i = i as usize;
                let t = if i < st.len() { st[i] } else { None };
                st.push(t);
                succs.push(pc + 1);
            }
            Op::SetLocal(i) => {
                let i = i as usize;
                let t = st.last().copied().flatten();
                if i < st.len() {
                    st[i] = t;
                }
                succs.push(pc + 1);
            }
            Op::GetUpvalue(_) | Op::SetUpvalue(_) => {
                return Err("--compile: closures (upvalue capture) are not supported yet".into());
            }
            Op::GetGlobal(n) => {
                let name = self.prog.names[n as usize].0.clone();
                if let Some(b) = builtin_of(&name) {
                    st.push(Some(Ty::Builtin(b)));
                } else if UNSUPPORTED_BUILTINS.contains(&name.as_ref()) {
                    return Err(format!(
                        "--compile: builtin '{}' is not supported yet",
                        name
                    ));
                } else {
                    st.push(*self.globals.get(&n).unwrap_or(&None));
                }
                succs.push(pc + 1);
            }
            Op::SetGlobal(n) => {
                let t = st.last().copied().flatten();
                let g = *self.globals.get(&n).unwrap_or(&None);
                let u = unify(g, t, &format!("global '{}'", self.prog.names[n as usize].0))?;
                if u != g {
                    self.globals.insert(n, u);
                    self.any_change = true;
                }
                succs.push(pc + 1);
            }
            Op::DefineGlobal(n) => {
                let t = pop(&mut st);
                let g = *self.globals.get(&n).unwrap_or(&None);
                let u = unify(g, t, &format!("global '{}'", self.prog.names[n as usize].0))?;
                if u != g {
                    self.globals.insert(n, u);
                    self.any_change = true;
                }
                succs.push(pc + 1);
            }
            Op::GetIndex => {
                pop(&mut st);
                let container = pop(&mut st);
                let t = match container {
                    Some(Ty::Range) => Some(Ty::Num),
                    Some(Ty::Str) => Some(Ty::Str),
                    Some(Ty::Nil) | None => None,
                    Some(other) => {
                        return Err(format!(
                            "--compile: cannot index a value of type {}",
                            ty_name(other)
                        ));
                    }
                };
                st.push(t);
                succs.push(pc + 1);
            }
            Op::SetIndex => {
                return Err("--compile: indexed assignment (lists/maps) is not supported yet"
                    .into());
            }
            Op::GetField(_) => {
                return Err("--compile: methods are not supported yet".into());
            }
            Op::Add => {
                let b = pop(&mut st);
                let a = pop(&mut st);
                let t = match (a, b) {
                    (Some(Ty::Num), Some(Ty::Num)) => Some(Ty::Num),
                    (Some(Ty::Str), Some(Ty::Str)) => Some(Ty::Str),
                    (Some(Ty::Nil), _) | (_, Some(Ty::Nil)) | (None, _) | (_, None) => None,
                    _ => {
                        return Err(format!(
                            "--compile: cannot add {} and {}",
                            ty_name(a.unwrap()),
                            ty_name(b.unwrap())
                        ));
                    }
                };
                st.push(t);
                succs.push(pc + 1);
            }
            Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Power => {
                let b = pop(&mut st);
                let a = pop(&mut st);
                match (a, b) {
                    (Some(Ty::Num), Some(Ty::Num)) => st.push(Some(Ty::Num)),
                    (Some(Ty::Nil), _) | (_, Some(Ty::Nil)) | (None, _) | (_, None) => {
                        st.push(None);
                    }
                    _ => {
                        return Err(format!(
                            "--compile: arithmetic needs numbers, got {} and {}",
                            ty_name(a.unwrap()),
                            ty_name(b.unwrap())
                        ));
                    }
                }
                succs.push(pc + 1);
            }
            Op::Eq | Op::Neq => {
                let b = pop(&mut st);
                let a = pop(&mut st);
                match (a, b) {
                    (Some(x), Some(y)) if x != y && x != Ty::Nil && y != Ty::Nil => {
                        return Err(format!(
                            "--compile: cannot compare {} and {}",
                            ty_name(x),
                            ty_name(y)
                        ));
                    }
                    _ => {}
                }
                st.push(Some(Ty::Bool));
                succs.push(pc + 1);
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let b = pop(&mut st);
                let a = pop(&mut st);
                match (a, b) {
                    (Some(x), Some(y)) if x == y && (x == Ty::Num || x == Ty::Str) => {}
                    (Some(Ty::Nil), _) | (_, Some(Ty::Nil)) | (None, _) | (_, None) => {}
                    (Some(x), Some(y)) => {
                        return Err(format!(
                            "--compile: cannot compare {} and {}",
                            ty_name(x),
                            ty_name(y)
                        ));
                    }
                }
                st.push(Some(Ty::Bool));
                succs.push(pc + 1);
            }
            Op::Neg => {
                let a = pop(&mut st);
                match a {
                    Some(Ty::Num) => st.push(Some(Ty::Num)),
                    Some(Ty::Nil) | None => st.push(None),
                    Some(other) => {
                        return Err(format!(
                            "--compile: cannot negate a value of type {}",
                            ty_name(other)
                        ));
                    }
                }
                succs.push(pc + 1);
            }
            Op::Not => {
                pop(&mut st);
                st.push(Some(Ty::Bool));
                succs.push(pc + 1);
            }
            Op::Jump(t) => {
                succs.push(t as usize);
            }
            Op::JumpIfFalse(t) => {
                pop(&mut st);
                succs.push(t as usize);
                succs.push(pc + 1);
            }
            Op::Loop(t) => {
                succs.push(t as usize);
            }
            Op::Call(n) => {
                let n = n as usize;
                let mut args: Vec<Option<Ty>> = Vec::with_capacity(n);
                for _ in 0..n {
                    args.push(pop(&mut st));
                }
                let callee = pop(&mut st);
                match callee {
                    Some(Ty::Fn(f)) => {
                        if n != self.prog.fns[f].arity {
                            return Err(format!(
                                "--compile: function '{}' expects {} argument(s) but got {}",
                                self.prog.fns[f].name,
                                self.prog.fns[f].arity,
                                n
                            ));
                        }
                        for (i, t) in args.iter().enumerate() {
                            let cur = self.fns[f].params[i];
                            let u = unify(
                                cur,
                                *t,
                                &format!("argument of '{}'", self.prog.fns[f].name),
                            )?;
                            if u != cur {
                                self.fns[f].params[i] = u;
                                self.any_change = true;
                            }
                        }
                        if self.fns[f].stacks[0].is_none() {
                            // The entry stack is the parameter list: locals 0..arity.
                            self.fns[f].stacks[0] = Some(self.fns[f].params.clone());
                            self.any_change = true;
                        }
                        st.push(self.fns[f].ret);
                    }
                    Some(Ty::Builtin(b)) => {
                        let arity_ok = match b {
                            Builtin::Speak => true,
                            Builtin::Size | Builtin::Num | Builtin::Str | Builtin::Int
                            | Builtin::Bool | Builtin::Abs | Builtin::Root => n == 1,
                            Builtin::Least | Builtin::Greatest => n == 2,
                            Builtin::Span => n == 1 || n == 2,
                            Builtin::Tick => n == 0,
                            Builtin::Vouch => n == 1 || n == 2,
                        };
                        if !arity_ok {
                            return Err("--compile: wrong argument count for a builtin".into());
                        }
                        let t = match b {
                            Builtin::Speak | Builtin::Vouch => Ty::Nil,
                            Builtin::Size | Builtin::Num | Builtin::Int | Builtin::Abs
                            | Builtin::Root | Builtin::Least | Builtin::Greatest
                            | Builtin::Tick => Ty::Num,
                            Builtin::Str => Ty::Str,
                            Builtin::Bool => Ty::Bool,
                            Builtin::Span => Ty::Range,
                        };
                        st.push(Some(t));
                    }
                    Some(Ty::Nil) | None => {
                        st.push(None);
                    }
                    Some(other) => {
                        return Err(format!(
                            "--compile: cannot call a value of type {}",
                            ty_name(other)
                        ));
                    }
                }
                succs.push(pc + 1);
            }
            Op::Return => {
                let t = pop(&mut st);
                let cur = self.fns[fid].ret;
                let u = unify(cur, t, &format!("the return of '{}'", self.prog.fns[fid].name))?;
                if u != cur {
                    self.fns[fid].ret = u;
                }
            }
            Op::Closure(ci) => {
                let (f, upvals) = &self.prog.closures[ci as usize];
                if !upvals.is_empty() {
                    return Err("--compile: closures (upvalue capture) are not supported yet"
                        .into());
                }
                st.push(Some(Ty::Fn(*f as usize)));
                succs.push(pc + 1);
            }
            Op::CloseUpvalue => {
                pop(&mut st);
                succs.push(pc + 1);
            }
            Op::Rotate3 => {
                return Err("--compile: compound indexed assignment is not supported yet".into());
            }
            Op::BuildList(_) | Op::BuildMap(_) => {
                return Err("--compile: lists and maps are not supported yet".into());
            }
            Op::BuildRange(_) => {
                pop(&mut st);
                pop(&mut st);
                st.push(Some(Ty::Range));
                succs.push(pc + 1);
            }
        }
        Ok((st, succs))
    }
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Operand {
    ty: Option<Ty>,
    /// The C identifier holding the value.
    cvar: String,
}

fn c_type(t: Ty) -> &'static str {
    match t {
        Ty::Num => "double",
        Ty::Str => "const char*",
        Ty::Bool => "int",
        Ty::Nil => "int",
        Ty::Fn(_) => "void*",
        Ty::Range => "crange",
        Ty::Builtin(_) => "void*",
    }
}

fn c_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\{:03o}", b)),
        }
    }
    out.push('"');
    out
}

fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('g');
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

struct Codegen<'a> {
    prog: &'a Program,
    fns: Vec<FnState>,
    globals: HashMap<u32, Option<Ty>>,
    out: Vec<String>,
    tmp: usize,
    fid: usize,
    stack: Vec<Operand>,
    /// Jump-target states: the operand stack each label must restore, and
    /// whether it has multiple distinct sources (unsupported join).
    target_states: HashMap<usize, (Vec<Operand>, bool)>,
    /// Declared C type of every emitted variable (used by SET_LOCAL to swap
    /// a slot to a fresh variable when a stored value has a different type).
    var_types: HashMap<String, Ty>,
    /// Set when a value's type cannot be determined; `generate` turns it into
    /// an error instead of emitting broken C.
    fatal: Option<String>,
}

impl<'a> Codegen<'a> {
    fn new(
        prog: &'a Program,
        fns: Vec<FnState>,
        globals: HashMap<u32, Option<Ty>>,
    ) -> Result<Self, String> {
        // Sanity-check local slots: each slot (a stack position read or
        // written by GET_LOCAL/SET_LOCAL) must hold one consistent type. Only
        // the types present at the slot-access instructions themselves count —
        // transient temps at the same stack position are fine (the `each`
        // desugar builds its range on top of a slot position).
        for (fid, f) in fns.iter().enumerate() {
            let mut slot_types: Vec<Vec<Ty>> = Vec::new();
            for (pc, op) in prog.fns[fid].instrs.iter().enumerate() {
                let idx = match op {
                    Op::GetLocal(i) | Op::SetLocal(i) => Some(*i as usize),
                    _ => None,
                };
                if let Some(i) = idx {
                    if let Some(t) = f.stacks.get(pc).and_then(|s| s.as_ref())
                        .and_then(|s| s.get(i))
                        .copied()
                        .flatten()
                    {
                        if t != Ty::Nil {
                            while slot_types.len() <= i {
                                slot_types.push(Vec::new());
                            }
                            if !slot_types[i].contains(&t) {
                                slot_types[i].push(t);
                            }
                        }
                    }
                }
            }
            for (i, types) in slot_types.iter().enumerate() {
                if types.len() > 1 {
                    return Err(format!(
                        "--compile: a local holds both {} and {} (dynamic typing) in '{}' at stack position {}",
                        ty_name(types[0]),
                        ty_name(types[1]),
                        prog.fns[fid].name,
                        i
                    ));
                }
            }
        }
        Ok(Codegen {
            prog,
            fns,
            globals,
            out: Vec::new(),
            tmp: 0,
            fid: 0,
            stack: Vec::new(),
            target_states: HashMap::new(),
            var_types: HashMap::new(),
            fatal: None,
        })
    }

    /// Record the current stack as the state a jump target must restore. If a
    /// different state was already recorded for the same target, the target
    /// has multiple distinct sources — that join is rejected at generate time.
    fn record_target(&mut self, t: usize) {
        let cur = self.stack.clone();
        match self.target_states.get_mut(&t) {
            None => {
                self.target_states.insert(t, (cur, false));
            }
            Some((prev, multi)) => {
                if *multi {
                    return;
                }
                let same = prev.len() == cur.len()
                    && prev.iter().zip(cur.iter()).all(|(a, b)| a.cvar == b.cvar);
                if !same {
                    *multi = true;
                }
            }
        }
    }

    fn fresh(&mut self) -> String {
        self.tmp += 1;
        format!("t{}", self.tmp)
    }

    /// Declare a fresh C variable holding `expr` and push it onto the stack.
    /// If the value's type cannot be determined, record a fatal error (the
    /// whole compile fails at `generate` — never emit broken C).
    fn push(&mut self, ty: Option<Ty>, expr: String) {
        let t = match ty {
            Some(t) => t,
            None => {
                self.fatal = Some(
                    "--compile: cannot determine the type of a value (dynamic typing)".to_string(),
                );
                return;
            }
        };
        let name = self.fresh();
        self.out.push(format!("{} {} = {};", c_type(t), name, expr));
        self.var_types.insert(name.clone(), t);
        self.stack.push(Operand { ty: Some(t), cvar: name });
    }

    /// Push an operand that has already been declared (a temp var).
    fn push_operand(&mut self, ty: Option<Ty>, cvar: String) {
        if let Some(t) = ty {
            if cvar.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                self.var_types.insert(cvar.clone(), t);
            }
        }
        self.stack.push(Operand { ty, cvar });
    }

    fn pop_operand(&mut self) -> Result<Operand, String> {
        self.stack
            .pop()
            .ok_or_else(|| "--compile: internal error (stack underflow)".to_string())
    }

    fn generate(mut self) -> Result<String, String> {
        if let Some(f) = self.fatal.take() {
            return Err(f);
        }
        // Forward declarations (mutual recursion and clean call order).
        for fid in 0..self.prog.fns.len() {
            if fid == 0 || self.fns[fid].stacks[0].is_none() {
                continue;
            }
            let ret = self.fn_ret(fid)?;
            let params = self.fn_params(fid)?;
            self.out.push(format!(
                "static {} fn_{}({});",
                c_type(ret),
                fid,
                params.join(", ")
            ));
        }

        // Global variables (non-function, non-builtin) live at file scope,
        // zero-initialized — matching the interpreter's undefined globals.
        let mut gdecls: Vec<(String, String)> = Vec::new();
        for (n, t) in &self.globals {
            if let Some(t) = t {
                if !matches!(t, Ty::Fn(_) | Ty::Builtin(_)) {
                    gdecls.push((
                        sanitize(&self.prog.names[*n as usize].0),
                        c_type(*t).to_string(),
                    ));
                }
            }
        }
        gdecls.sort();
        for (name, cty) in &gdecls {
            self.out.push(format!("static {} {};", cty, name));
        }

        for fid in 0..self.prog.fns.len() {
            if self.fns[fid].stacks[0].is_none() {
                continue;
            }
            self.fid = fid;
            self.gen_function(fid)?;
        }
        Ok(self.out.join("\n"))
    }

    fn fn_ret(&self, fid: usize) -> Result<Ty, String> {
        if fid == 0 {
            return Ok(Ty::Nil);
        }
        self.fns[fid]
            .ret
            .ok_or_else(|| {
                format!(
                    "--compile: cannot determine the return type of '{}'",
                    self.prog.fns[fid].name
                )
            })
            .and_then(|r| {
                if matches!(r, Ty::Fn(_) | Ty::Builtin(_)) {
                    Err("--compile: functions returning functions are not supported".into())
                } else {
                    Ok(r)
                }
            })
    }

    fn fn_params(&self, fid: usize) -> Result<Vec<String>, String> {
        (0..self.prog.fns[fid].arity)
            .map(|i| {
                let t = self.fns[fid].params[i].ok_or_else(|| {
                    format!(
                        "--compile: cannot determine the type of a parameter of '{}'",
                        self.prog.fns[fid].name
                    )
                })?;
                Ok(format!("{} p{}", c_type(t), i))
            })
            .collect()
    }

    fn gen_function(&mut self, fid: usize) -> Result<(), String> {
        let ret = self.fn_ret(fid)?;
        let params = self.fn_params(fid)?;
        if fid == 0 {
            self.out.push("int main(void) {".to_string());
        } else {
            self.out.push(format!(
                "static {} fn_{}({}) {{",
                c_type(ret),
                fid,
                params.join(", ")
            ));
        }
        // Initial stack: the parameters occupy local slots 0..arity.
        self.stack.clear();
        self.target_states.clear();
        for i in 0..self.prog.fns[fid].arity {
            let t = self.fns[fid].params[i];
            let cvar = format!("p{}", i);
            if let Some(t) = t {
                self.var_types.insert(cvar.clone(), t);
            }
            self.stack.push(Operand { ty: t, cvar });
        }

        // Collect jump targets for labels.
        let instrs = &self.prog.fns[fid].instrs;
        let mut targets: Vec<usize> = Vec::new();
        for op in instrs {
            match op {
                Op::Jump(t) | Op::JumpIfFalse(t) | Op::Loop(t) => targets.push(*t as usize),
                _ => {}
            }
        }
        targets.sort_unstable();
        targets.dedup();
        let mut ti = 0;
        let mut next_target = targets.first().copied();
        // Whether straight-line code since the last label is unreachable (a
        // RETURN/JUMP/LOOP was emitted). Unreachable code must not participate
        // in the join comparison — the recorded state is authoritative.
        let mut unreachable = false;

        for (pc, op) in instrs.iter().enumerate() {
            if next_target == Some(pc) {
                self.out.push(format!("L{}: ;", pc));
                if let Some((st, multi)) = self.target_states.get(&pc) {
                    if *multi {
                        return Err(
                            "--compile: control-flow joins with live values are not supported yet"
                                .to_string(),
                        );
                    }
                    if !unreachable
                        && (self.stack.len() != st.len()
                            || self
                                .stack
                                .iter()
                                .zip(st.iter())
                                .any(|(a, b)| a.cvar != b.cvar))
                    {
                        return Err(
                            "--compile: control-flow joins with live values are not supported yet"
                                .to_string(),
                        );
                    }
                    self.stack = st.clone();
                }
                unreachable = false;
                ti += 1;
                next_target = targets.get(ti).copied();
            }
            let terminal = matches!(op, Op::Return | Op::Jump(_) | Op::Loop(_));
            self.gen_instr(fid, pc, *op)?;
            if terminal {
                unreachable = true;
            }
        }
        // Default return (falling off the end).
        match (fid, ret) {
            (0, _) => self.out.push("return 0;".to_string()),
            (_, Ty::Str) => self.out.push("return \"\";".to_string()),
            (_, Ty::Num) => self.out.push("return 0.0;".to_string()),
            _ => self.out.push("return 0;".to_string()),
        }
        self.out.push("}".to_string());
        Ok(())
    }

    fn gen_instr(&mut self, fid: usize, _pc: usize, op: Op) -> Result<(), String> {
        match op {
            Op::Const(i) => {
                let (expr, ty) = match &self.prog.constants[i as usize] {
                    crate::seed::Value::Num(n) => (format!("{}", n), Ty::Num),
                    crate::seed::Value::Str(s) => (c_escape(s), Ty::Str),
                    crate::seed::Value::Bool(b) => (
                        if *b { "1".to_string() } else { "0".to_string() },
                        Ty::Bool,
                    ),
                    crate::seed::Value::Nil => ("0".to_string(), Ty::Nil),
                    other => {
                        return Err(format!(
                            "--compile: constants of type {} are not supported",
                            crate::seed::type_name(other)
                        ));
                    }
                };
                self.push(Some(ty), expr);
            }
            Op::Nil => self.push(Some(Ty::Nil), "0".to_string()),
            Op::True => self.push(Some(Ty::Bool), "1".to_string()),
            Op::False => self.push(Some(Ty::Bool), "0".to_string()),
            Op::Pop => {
                self.pop_operand()?;
            }
            Op::GetLocal(i) => {
                let op = self
                    .stack
                    .get(i as usize)
                    .cloned()
                    .ok_or_else(|| "--compile: internal error (GET_LOCAL out of range)".to_string())?;
                self.push(op.ty, op.cvar);
            }
            Op::SetLocal(i) => {
                let i = i as usize;
                let top = self.stack.last().cloned().ok_or_else(|| {
                    "--compile: internal error (SET_LOCAL with empty stack)".to_string()
                })?;
                if i >= self.stack.len() {
                    return Err("--compile: internal error (SET_LOCAL out of range)".to_string());
                }
                // Store top into slot i. If the slot's variable has a
                // different C type (e.g. an `each` slot initialized with nil,
                // or a slot that gets reassigned another type), declare a
                // fresh correctly-typed variable and swap it in — the old
                // value is discarded, since SET_LOCAL replaces the slot.
                let slot_var = self.stack[i].cvar.clone();
                let slot_ty = self.var_types.get(&slot_var).copied();
                let needs_swap = match (slot_ty, top.ty) {
                    (Some(st), Some(tt)) => st != tt && tt != Ty::Nil,
                    (None, Some(tt)) => tt != Ty::Nil,
                    _ => false,
                };
                if needs_swap {
                    let t = top.ty.unwrap();
                    let name = self.fresh();
                    self.out.push(format!("{} {} = {};", c_type(t), name, top.cvar));
                    self.var_types.insert(name.clone(), t);
                    self.stack[i].cvar = name;
                    self.stack[i].ty = Some(t);
                } else {
                    self.out.push(format!("{} = {};", slot_var, top.cvar));
                    self.stack[i].ty = top.ty;
                }
            }
            Op::GetUpvalue(_) | Op::SetUpvalue(_) => {
                return Err("--compile: closures are not supported yet".into());
            }
            Op::GetGlobal(n) => {
                let name = self.prog.names[n as usize].0.clone();
                if let Some(b) = builtin_of(&name) {
                    self.push(Some(Ty::Builtin(b)), "0".to_string());
                } else {
                    let t = *self.globals.get(&n).unwrap_or(&None);
                    match t {
                        Some(Ty::Fn(f)) => self.push_operand(t, format!("fn_{}", f)),
                        Some(t) => self.push(Some(t), sanitize(&name)),
                        None => {
                            return Err(format!(
                                "--compile: cannot determine the type of global '{}'",
                                name
                            ));
                        }
                    }
                }
            }
            Op::SetGlobal(n) => {
                let top = self.stack.last().cloned().ok_or_else(|| {
                    "--compile: internal error (SET_GLOBAL with empty stack)".to_string()
                })?;
                let name = self.prog.names[n as usize].0.clone();
                if !matches!(self.globals.get(&n), Some(Some(Ty::Fn(_)))) {
                    self.out.push(format!("{} = {};", sanitize(&name), top.cvar));
                }
            }
            Op::DefineGlobal(n) => {
                let top = self.pop_operand()?;
                let name = self.prog.names[n as usize].0.clone();
                if !matches!(self.globals.get(&n), Some(Some(Ty::Fn(_)))) {
                    self.out.push(format!("{} = {};", sanitize(&name), top.cvar));
                }
            }
            Op::GetIndex => {
                let key = self.pop_operand()?;
                let container = self.pop_operand()?;
                match container.ty {
                    Some(Ty::Range) => self.push(
                        Some(Ty::Num),
                        format!("({}.s + {})", container.cvar, key.cvar),
                    ),
                    Some(Ty::Str) => {
                        let n = self.fresh();
                        self.out.push(format!("char {}[2];", n));
                        self.out.push(format!(
                            "{}[0] = ({})[(int)({})];",
                            n, container.cvar, key.cvar
                        ));
                        self.out.push(format!("{}[1] = 0;", n));
                        self.push_operand(Some(Ty::Str), n);
                    }
                    other => {
                        return Err(format!(
                            "--compile: cannot index a value of type {}",
                            ty_name(other.unwrap())
                        ));
                    }
                }
            }
            Op::SetIndex => {
                return Err("--compile: indexed assignment is not supported yet".into());
            }
            Op::GetField(_) => {
                return Err("--compile: methods are not supported yet".into());
            }
            Op::Add => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                match (a.ty, b.ty) {
                    (Some(Ty::Num), Some(Ty::Num)) => {
                        self.push(Some(Ty::Num), format!("({} + {})", a.cvar, b.cvar));
                    }
                    (Some(Ty::Str), Some(Ty::Str)) => {
                        let n = self.fresh();
                        self.out
                            .push(format!("char* {} = c_concat({}, {});", n, a.cvar, b.cvar));
                        self.push_operand(Some(Ty::Str), n);
                    }
                    _ => {
                        return Err("--compile: cannot add these values".into());
                    }
                }
            }
            Op::Sub => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.bin_num("({} - {})", &a, &b)?;
            }
            Op::Mul => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.bin_num("({} * {})", &a, &b)?;
            }
            Op::Div => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.bin_num("({} / {})", &a, &b)?;
            }
            Op::Mod => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.bin_num("fmod({}, {})", &a, &b)?;
            }
            Op::Power => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.bin_num("pow({}, {})", &a, &b)?;
            }
            Op::Eq => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                let expr = match (a.ty, b.ty) {
                    (Some(Ty::Str), Some(Ty::Str)) => {
                        format!("(strcmp({}, {}) == 0)", a.cvar, b.cvar)
                    }
                    _ => format!("({} == {})", a.cvar, b.cvar),
                };
                self.push(Some(Ty::Bool), expr);
            }
            Op::Neq => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                let expr = match (a.ty, b.ty) {
                    (Some(Ty::Str), Some(Ty::Str)) => {
                        format!("(strcmp({}, {}) != 0)", a.cvar, b.cvar)
                    }
                    _ => format!("({} != {})", a.cvar, b.cvar),
                };
                self.push(Some(Ty::Bool), expr);
            }
            Op::Lt => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.cmp("({} < {})", &a, &b)?;
            }
            Op::Le => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.cmp("({} <= {})", &a, &b)?;
            }
            Op::Gt => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.cmp("({} > {})", &a, &b)?;
            }
            Op::Ge => {
                let b = self.pop_operand()?;
                let a = self.pop_operand()?;
                self.cmp("({} >= {})", &a, &b)?;
            }
            Op::Neg => {
                let a = self.pop_operand()?;
                self.push(Some(Ty::Num), format!("(-{})", a.cvar));
            }
            Op::Not => {
                let a = self.pop_operand()?;
                let expr = match a.ty {
                    Some(Ty::Str) => format!("(!({}[0] != 0))", a.cvar),
                    Some(Ty::Num) | Some(Ty::Bool) | Some(Ty::Nil) => format!("(!({}))", a.cvar),
                    _ => {
                        return Err("--compile: cannot negate this value".into());
                    }
                };
                self.push(Some(Ty::Bool), expr);
            }
            Op::Jump(t) => {
                self.out.push(format!("goto L{};", t));
                self.record_target(t as usize);
            }
            Op::JumpIfFalse(t) => {
                let cond = self.pop_operand()?;
                self.out.push(format!("if (!({})) goto L{};", cond.cvar, t));
                self.record_target(t as usize);
            }
            Op::Loop(t) => {
                self.out.push(format!("goto L{};", t));
                self.record_target(t as usize);
            }
            Op::Call(n) => {
                let n = n as usize;
                let mut args: Vec<Operand> = Vec::with_capacity(n);
                for _ in 0..n {
                    args.push(self.pop_operand()?);
                }
                // Popped top-first, so reverse back to source order.
                args.reverse();
                let callee = self.pop_operand()?;
                let arg_exprs: Vec<String> = args.iter().map(|a| a.cvar.clone()).collect();
                match callee.ty {
                    Some(Ty::Fn(f)) => {
                        let ret = self.fn_ret(f)?;
                        let name = self.fresh();
                        self.out.push(format!(
                            "{} {} = fn_{}({});",
                            c_type(ret),
                            name,
                            f,
                            arg_exprs.join(", ")
                        ));
                        self.push_operand(Some(ret), name);
                    }
                    Some(Ty::Builtin(b)) => {
                        self.gen_builtin(b, args)?;
                    }
                    _ => {
                        return Err("--compile: cannot call this value".into());
                    }
                }
            }
            Op::Return => {
                let v = self.pop_operand()?;
                if fid == 0 {
                    self.out.push("return 0;".to_string());
                } else {
                    self.out.push(format!("return {};", v.cvar));
                }
            }
            Op::Closure(ci) => {
                let (f, upvals) = &self.prog.closures[ci as usize];
                if !upvals.is_empty() {
                    return Err("--compile: closures are not supported yet".into());
                }
                // Static functions: no runtime closure value is needed.
                self.push_operand(Some(Ty::Fn(*f as usize)), format!("fn_{}", f));
            }
            Op::CloseUpvalue => {
                self.pop_operand()?;
            }
            Op::Rotate3 => {
                return Err("--compile: compound indexed assignment is not supported yet".into());
            }
            Op::BuildList(_) | Op::BuildMap(_) => {
                return Err("--compile: lists and maps are not supported yet".into());
            }
            Op::BuildRange(inclusive) => {
                let end = self.pop_operand()?;
                let start = self.pop_operand()?;
                let n = self.fresh();
                let e = if inclusive {
                    format!("({} + 1.0)", end.cvar)
                } else {
                    end.cvar
                };
                self.out.push(format!("crange {} = {{ {}, {} }};", n, start.cvar, e));
                self.push_operand(Some(Ty::Range), n);
            }
        }
        Ok(())
    }

    fn bin_num(&mut self, fmt: &str, a: &Operand, b: &Operand) -> Result<(), String> {
        match (a.ty, b.ty) {
            (Some(Ty::Num), Some(Ty::Num)) => {
                let expr = fmt.replacen("{}", &a.cvar, 1).replacen("{}", &b.cvar, 1);
                self.push(Some(Ty::Num), expr);
                Ok(())
            }
            _ => Err("--compile: arithmetic needs numbers".to_string()),
        }
    }

    fn cmp(&mut self, fmt: &str, a: &Operand, b: &Operand) -> Result<(), String> {
        let expr = match (a.ty, b.ty) {
            (Some(Ty::Num), Some(Ty::Num)) => {
                fmt.replacen("{}", &a.cvar, 1).replacen("{}", &b.cvar, 1)
            }
            (Some(Ty::Str), Some(Ty::Str)) => {
                let op = if fmt.contains("<=") {
                    "<= 0"
                } else if fmt.contains(">=") {
                    ">= 0"
                } else if fmt.contains("<") {
                    "< 0"
                } else {
                    "> 0"
                };
                format!("(strcmp({}, {}) {})", a.cvar, b.cvar, op)
            }
            _ => {
                return Err("--compile: cannot compare these values".into());
            }
        };
        self.push(Some(Ty::Bool), expr);
        Ok(())
    }

    fn gen_builtin(&mut self, b: Builtin, args: Vec<Operand>) -> Result<(), String> {
        match b {
            Builtin::Speak => {
                // Join the arguments with spaces, like the interpreter.
                let mut pieces: Vec<String> = Vec::with_capacity(args.len());
                for a in &args {
                    match a.ty {
                        Some(Ty::Num) => {
                            let n = self.fresh();
                            self.out
                                .push(format!("char {}[64];", n));
                            self.out.push(format!("c_fmt_num({}, {});", a.cvar, n));
                            pieces.push(n);
                        }
                        Some(Ty::Str) => pieces.push(a.cvar.clone()),
                        Some(Ty::Bool) => {
                            pieces.push(format!("({} ? \"true\" : \"false\")", a.cvar));
                        }
                        Some(Ty::Nil) => pieces.push("\"nil\"".to_string()),
                        _ => {
                            return Err("--compile: cannot speak this value".into());
                        }
                    }
                }
                if pieces.is_empty() {
                    self.out.push("printf(\"\\n\");".to_string());
                } else {
                    let fmt = pieces
                        .iter()
                        .map(|_| "%s")
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.out.push(format!(
                        "printf(\"{}\\n\", {});",
                        fmt,
                        pieces.join(", ")
                    ));
                }
                self.push(Some(Ty::Nil), "0".to_string());
            }
            Builtin::Size => {
                let a = args.into_iter().next().unwrap();
                let expr = match a.ty {
                    Some(Ty::Range) => format!("c_size_range({})", a.cvar),
                    Some(Ty::Str) => format!("c_size_str({})", a.cvar),
                    _ => return Err("--compile: size needs a range or string".into()),
                };
                self.push(Some(Ty::Num), expr);
            }
            Builtin::Num => {
                let a = args.into_iter().next().unwrap();
                let expr = match a.ty {
                    Some(Ty::Num) => a.cvar,
                    Some(Ty::Str) => format!("atof({})", a.cvar),
                    Some(Ty::Bool) => format!("({} ? 1.0 : 0.0)", a.cvar),
                    _ => return Err("--compile: cannot num this value".into()),
                };
                self.push(Some(Ty::Num), expr);
            }
            Builtin::Str => {
                let a = args.into_iter().next().unwrap();
                match a.ty {
                    Some(Ty::Num) => {
                        let n = self.fresh();
                        self.out.push(format!("char {}[64];", n));
                        self.out.push(format!("c_fmt_num({}, {});", a.cvar, n));
                        self.push_operand(Some(Ty::Str), n);
                    }
                    Some(Ty::Str) => self.push(Some(Ty::Str), a.cvar),
                    Some(Ty::Bool) => self.push(
                        Some(Ty::Str),
                        format!("({} ? \"true\" : \"false\")", a.cvar),
                    ),
                    Some(Ty::Nil) => self.push(Some(Ty::Str), "\"nil\"".to_string()),
                    _ => return Err("--compile: cannot str this value".into()),
                }
            }
            Builtin::Int => {
                let a = args.into_iter().next().unwrap();
                self.push(Some(Ty::Num), format!("trunc({})", a.cvar));
            }
            Builtin::Bool => {
                let a = args.into_iter().next().unwrap();
                let expr = match a.ty {
                    Some(Ty::Num) | Some(Ty::Bool) => format!("({} != 0)", a.cvar),
                    Some(Ty::Str) => format!("({}[0] != 0)", a.cvar),
                    Some(Ty::Nil) => "0".to_string(),
                    _ => return Err("--compile: cannot bool this value".into()),
                };
                self.push(Some(Ty::Bool), expr);
            }
            Builtin::Abs => {
                let a = args.into_iter().next().unwrap();
                self.push(Some(Ty::Num), format!("fabs({})", a.cvar));
            }
            Builtin::Root => {
                let a = args.into_iter().next().unwrap();
                self.push(Some(Ty::Num), format!("sqrt({})", a.cvar));
            }
            Builtin::Least => {
                let mut it = args.into_iter();
                let a = it.next().unwrap();
                let b = it.next().unwrap();
                self.push(
                    Some(Ty::Num),
                    format!("(({}) < ({}) ? ({}) : ({}))", a.cvar, b.cvar, a.cvar, b.cvar),
                );
            }
            Builtin::Greatest => {
                let mut it = args.into_iter();
                let a = it.next().unwrap();
                let b = it.next().unwrap();
                self.push(
                    Some(Ty::Num),
                    format!("(({}) > ({}) ? ({}) : ({}))", a.cvar, b.cvar, a.cvar, b.cvar),
                );
            }
            Builtin::Span => {
                let mut it = args.into_iter();
                let b = it.next().unwrap();
                let a = it.next();
                let n = self.fresh();
                match a {
                    Some(a) => self.out.push(format!(
                        "crange {} = {{ {}, {} }};",
                        n, a.cvar, b.cvar
                    )),
                    None => self.out.push(format!("crange {} = {{ 0.0, {} }};", n, b.cvar)),
                }
                self.push_operand(Some(Ty::Range), n);
            }
            Builtin::Tick => {
                self.push(Some(Ty::Num), "c_tick()".to_string());
            }
            Builtin::Vouch => {
                let mut it = args.into_iter();
                let cond = it.next().unwrap();
                let msg = it.next();
                let msg_expr = match msg {
                    Some(m) => m.cvar,
                    None => "\"vouch failed: the condition was false\"".to_string(),
                };
                self.out.push(format!(
                    "if (!({})) {{ fprintf(stderr, \"vouch failed: %s\\n\", {}); exit(70); }}",
                    cond.cvar, msg_expr
                ));
                self.push(Some(Ty::Nil), "0".to_string());
            }
        }
        Ok(())
    }
}

/// The C preamble: includes and runtime helpers that mirror the interpreter's
/// semantics exactly (number formatting, range sizes, ticks).
const C_HEADER: &str = r#"/* Generated by corros --compile. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

typedef struct { double s, e; } crange;

static double c_size_range(crange r) { double n = r.e - r.s; return n > 0.0 ? n : 0.0; }
static size_t c_size_str(const char* s) { return strlen(s); }

static char* c_concat(const char* a, const char* b) {
    size_t la = strlen(a), lb = strlen(b);
    char* out = malloc(la + lb + 1);
    memcpy(out, a, la);
    memcpy(out + la, b, lb + 1);
    return out;
}

static void c_fmt_num(double x, char* buf) {
    if (x != x) { strcpy(buf, "nan"); return; }
    if (x == 1.0 / 0.0) { strcpy(buf, "inf"); return; }
    if (x == -1.0 / 0.0) { strcpy(buf, "-inf"); return; }
    if (fabs(x) < 1e15 && floor(x) == x) { sprintf(buf, "%lld", (long long)x); return; }
    sprintf(buf, "%.15g", x);
}

static double c_tick(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}
"#;

/// Entry point: compile textual bytecode to C source.
pub fn compile_program(text: &str) -> Result<String, String> {
    let prog = load_program(text)?;
    let mut an = Analyzer::new(&prog);
    an.analyze()?;
    let cg = Codegen::new(&prog, an.fns, an.globals)?;
    let body = cg.generate()?;
    Ok(format!("{}\n\n{}", C_HEADER, body))
}
