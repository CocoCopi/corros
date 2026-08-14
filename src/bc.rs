//! Loader for the self-hosted bytecode text format.
//!
//! `selfhost/compiler.cor` prints compiled programs as text — `FUNCTION`
//! blocks with one instruction per line, ended by `ENDFN`/`MAIN`. The
//! Corros-written VM (`selfhost/vm.cor`) interprets that text, and so can the
//! native engine: `corros --run-bc program.bc [args...]` loads the text into
//! the Rust [`VM`] and executes it at native speed. That makes the deep
//! bootstrap chain practical: the host engine runs the *compiled* VM, which
//! runs the *compiled* compiler, which compiles programs.

use std::rc::Rc;

use crate::chunk::{Function, OpCode, UpvalueDesc};
use crate::value::Value;

/// Load a text bytecode program into a script [`Function`].
///
/// Function blocks are indexed by id; `CLOSURE <fid>` operands are rewritten
/// into constant-pool indices. CLOSURE references always point at *lower*
/// function ids (inner functions complete first), which lets the placeholder
/// replacement below mutate each chunk exactly once, before any other chunk
/// can alias it.
pub fn load_bytecode(text: &str) -> Result<Rc<Function>, String> {
    // --- 1. Parse raw blocks: (name, arity, [(op, operands)]). ---
    struct RawFn {
        name: String,
        arity: u8,
        instrs: Vec<(String, Vec<String>)>,
    }
    let mut raw: Vec<RawFn> = Vec::new();
    let mut current: Option<RawFn> = None;
    let mut last_id: usize = 0;

    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("FUNCTION ") {
            let mut parts = rest.splitn(3, ' ');
            let id: usize = parts
                .next()
                .ok_or("bad FUNCTION header")?
                .parse()
                .map_err(|_| "bad function id")?;
            last_id = id;
            let name = parts
                .next()
                .ok_or("bad FUNCTION header (missing name)")?
                .to_string();
            let arity: u8 = parts
                .next()
                .ok_or("bad FUNCTION header (missing arity)")?
                .parse()
                .map_err(|_| "bad arity")?;
            if let Some(prev) = current.take() {
                raw.push(prev);
            }
            current = Some(RawFn {
                name,
                arity,
                instrs: Vec::new(),
            });
            continue;
        }
        if line == "ENDFN" {
            if let Some(prev) = current.take() {
                raw.push(prev);
            }
            continue;
        }
        if line.starts_with("MAIN ") {
            continue;
        }
        let mut parts = line.split(' ');
        let op = parts.next().unwrap_or("").to_string();
        let operands: Vec<String> = parts.map(|s| s.to_string()).collect();
        if let Some(entry) = current.as_mut() {
            entry.instrs.push((op, operands));
        } else {
            return Err(format!("instruction outside a FUNCTION block: {}", line));
        }
    }
    if let Some(prev) = current.take() {
        raw.push(prev);
    }
    if raw.is_empty() {
        return Err("no functions found in bytecode".to_string());
    }
    if last_id >= raw.len() {
        return Err(format!("function id {} out of range", last_id));
    }

    // --- 2. Emit opcodes into owned functions; function references become
    // placeholder constants whose indices are recorded for later. ---
    let mut owned: Vec<Function> = raw
        .iter()
        .map(|f| Function::new(f.name.clone(), f.arity))
        .collect();
    // refs[fid] = [(constant_index, target_fid)] for CLOSURE placeholders.
    let mut refs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); raw.len()];

    for (fid, f) in raw.iter().enumerate() {
        for (op, operands) in &f.instrs {
            let line = 1u32;
            match op.as_str() {
                "CONST" => {
                    let value = parse_literal(&operands.join(" "))?;
                    let idx = owned[fid].chunk.add_constant(value);
                    owned[fid].chunk.emit(OpCode::Constant(idx), line);
                }
                "NIL" => owned[fid].chunk.emit(OpCode::Nil, line),
                "TRUE" => owned[fid].chunk.emit(OpCode::True, line),
                "FALSE" => owned[fid].chunk.emit(OpCode::False, line),
                "POP" => owned[fid].chunk.emit(OpCode::Pop, line),
                "GET_LOCAL" | "SET_LOCAL" | "GET_UPVALUE" | "SET_UPVALUE" => {
                    let n: u8 = num(&operands[0])?;
                    let code = match op.as_str() {
                        "GET_LOCAL" => OpCode::GetLocal(n),
                        "SET_LOCAL" => OpCode::SetLocal(n),
                        "GET_UPVALUE" => OpCode::GetUpvalue(n),
                        _ => OpCode::SetUpvalue(n),
                    };
                    owned[fid].chunk.emit(code, line);
                }
                "DEFINE_GLOBAL" | "GET_GLOBAL" | "SET_GLOBAL" | "GET_FIELD" => {
                    let name = operands.join(" ");
                    let idx = owned[fid].chunk.add_constant(Value::str(name));
                    let code = match op.as_str() {
                        "DEFINE_GLOBAL" => OpCode::DefineGlobal(idx),
                        "GET_GLOBAL" => OpCode::GetGlobal(idx),
                        "SET_GLOBAL" => OpCode::SetGlobal(idx),
                        _ => OpCode::GetField(idx),
                    };
                    owned[fid].chunk.emit(code, line);
                }
                "GET_INDEX" => owned[fid].chunk.emit(OpCode::GetIndex, line),
                "SET_INDEX" => owned[fid].chunk.emit(OpCode::SetIndex, line),
                "ADD" => owned[fid].chunk.emit(OpCode::Add, line),
                "SUB" => owned[fid].chunk.emit(OpCode::Subtract, line),
                "MUL" => owned[fid].chunk.emit(OpCode::Multiply, line),
                "DIV" => owned[fid].chunk.emit(OpCode::Divide, line),
                "MOD" => owned[fid].chunk.emit(OpCode::Modulo, line),
                "POWER" => owned[fid].chunk.emit(OpCode::Power, line),
                "NEG" => owned[fid].chunk.emit(OpCode::Negate, line),
                "NOT" => owned[fid].chunk.emit(OpCode::Not, line),
                "EQ" => owned[fid].chunk.emit(OpCode::Equal, line),
                "NEQ" => owned[fid].chunk.emit(OpCode::NotEqual, line),
                "LT" => owned[fid].chunk.emit(OpCode::Less, line),
                "LE" => owned[fid].chunk.emit(OpCode::LessEqual, line),
                "GT" => owned[fid].chunk.emit(OpCode::Greater, line),
                "GE" => owned[fid].chunk.emit(OpCode::GreaterEqual, line),
                "JUMP" => {
                    let t: usize = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::Jump { target: t }, line);
                }
                "JUMP_IF_FALSE" => {
                    let t: usize = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::JumpIfFalse { target: t }, line);
                }
                "LOOP" => {
                    let t: usize = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::Loop { target: t }, line);
                }
                "CALL" => {
                    let n: u8 = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::Call(n), line);
                }
                "RETURN" => owned[fid].chunk.emit(OpCode::Return, line),
                "CLOSURE" => {
                    let target: usize = num(&operands[0])?;
                    let mut upvalues = Vec::new();
                    let mut i = 1;
                    while i + 1 < operands.len() {
                        let is_local: bool = num::<u8>(&operands[i])? != 0;
                        let index: u8 = num(&operands[i + 1])?;
                        upvalues.push(UpvalueDesc { is_local, index });
                        i += 2;
                    }
                    owned[fid].upvalue_count = upvalues.len() as u8;
                    let idx = owned[fid].chunk.add_constant(Value::Nil); // placeholder
                    refs[fid].push((idx as usize, target));
                    owned[fid].chunk.emit(
                        OpCode::Closure {
                            function: idx,
                            upvalues,
                        },
                        line,
                    );
                }
                "CLOSE_UPVALUE" => owned[fid].chunk.emit(OpCode::CloseUpvalue, line),
                "ROTATE3" => owned[fid].chunk.emit(OpCode::Rotate3, line),
                "BUILD_LIST" => {
                    let n: u32 = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::BuildList(n), line);
                }
                "BUILD_MAP" => {
                    let n: u32 = num(&operands[0])?;
                    owned[fid].chunk.emit(OpCode::BuildMap(n), line);
                }
                "BUILD_RANGE" => {
                    let inclusive = operands.first().map(|s| s.as_str()) == Some("inclusive");
                    owned[fid]
                        .chunk
                        .emit(OpCode::BuildRange { inclusive }, line);
                }
                other => return Err(format!("unknown opcode '{}'", other)),
            }
        }
    }

    // --- 3. Wrap in Rc, then replace placeholders with real function
    // references. Replace non-script functions first (ascending), then the
    // script last: a chunk only ever references *lower* ids, so by the time a
    // function's own placeholders are replaced, no other chunk has aliased it
    // yet (the script, which references everyone, is handled last).
    let mut funcs: Vec<Rc<Function>> = owned.into_iter().map(Rc::new).collect();
    let mut order: Vec<usize> = (1..funcs.len()).collect();
    order.push(0);
    for fid in order {
        for (idx, target) in refs[fid].clone() {
            let target_fn = funcs[target].clone();
            let function = Rc::get_mut(&mut funcs[fid]).ok_or("function aliased early")?;
            function.chunk.constants[idx] = Value::Function(target_fn);
        }
    }

    Ok(funcs[0].clone())
}

fn num<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("bad number '{}'", s))
}

/// Parse a CONST literal: nil / true / false / a quoted escaped string / a num.
fn parse_literal(s: &str) -> Result<Value, String> {
    match s {
        "nil" => Ok(Value::Nil),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => {
            if let Some(inner) = s.strip_prefix('"') {
                let inner = inner
                    .strip_suffix('"')
                    .ok_or_else(|| format!("unterminated string literal {}", s))?;
                Ok(Value::str(unescape(inner)))
            } else {
                let n: f64 = s
                    .parse()
                    .map_err(|_| format!("bad literal '{}'", s))?;
                Ok(Value::num(n))
            }
        }
    }
}


/// Reverse of `value::escape_str`: turn `a\nb` (backslash escapes) into the
/// real string.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::VM;

    #[test]
    fn loads_and_runs_bytecode_text() {
        let text = r#"FUNCTION 0 <script> 0
GET_GLOBAL speak
CONST "hello, "
GET_GLOBAL name
CALL 2
POP
NIL
RETURN
ENDFN
MAIN 0
"#;
        let script = load_bytecode(text).expect("load");
        let mut vm = VM::new();
        vm.set_global("name", Value::str("corros"));
        vm.run(script).expect("run");
    }

    #[test]
    fn loads_closure_upvalues() {
        // `forge x = 7` then `craft f() { return x }`: the closure captures
        // local slot 0 (which holds the 7 pushed by CONST).
        let text = r#"FUNCTION 0 <script> 0
CONST 7
CLOSURE 1 1 0
DEFINE_GLOBAL f
GET_GLOBAL f
CALL 0
POP
NIL
RETURN
ENDFN
FUNCTION 1 f 0
GET_UPVALUE 0
RETURN
NIL
RETURN
ENDFN
MAIN 0
"#;
        let script = load_bytecode(text).expect("load");
        let mut vm = VM::new();
        vm.run(script).expect("run");
    }
}
