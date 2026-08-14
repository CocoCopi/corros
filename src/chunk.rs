//! Bytecode for the Corros virtual machine.
//!
//! Corros compiles source to a flat list of instructions (`OpCode`) stored in
//! a [`Chunk`], alongside the constants each instruction references. The VM
//! then interprets the chunk with a stack machine. Instructions use wide
//! operands for clarity; a production VM would pack them into bytes.

use std::fmt;

use crate::value::Value;

/// How a closure captures a variable from an enclosing function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpvalueDesc {
    /// True if the variable lives in the enclosing function's locals
    /// (a stack slot); false if it is itself an upvalue of the enclosing
    /// function.
    pub is_local: bool,
    /// Index into the enclosing function's locals (if `is_local`) or its
    /// upvalue list (otherwise).
    pub index: u8,
}

/// A single bytecode instruction. Operands are stored inline.
#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    /// Push a constant from the chunk's constant pool.
    Constant(u32),
    Nil,
    True,
    False,
    /// Pop the top value and discard it.
    Pop,
    /// Push a copy of a local variable (stack slot relative to frame base).
    GetLocal(u8),
    /// Pop the top value and store it into a local variable.
    SetLocal(u8),
    /// Define a global variable (pop value, bind to name in constant pool).
    DefineGlobal(u32),
    /// Push the value of a global variable (name in constant pool).
    GetGlobal(u32),
    /// Pop the top value and store it into a global variable.
    SetGlobal(u32),
    /// Push a copy of an upvalue (captured variable from an enclosing scope).
    GetUpvalue(u8),
    /// Pop the top value and store it into an upvalue.
    SetUpvalue(u8),
    /// Pop a key and a container, push `container[key]`.
    GetIndex,
    /// Pop a value, a key, and a container; store `container[key] = value`.
    SetIndex,
    /// Pop a value, look up a method by name (constant pool), push a bound
    /// method value.
    GetField(u32),
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
    /// Unary minus.
    Negate,
    /// Logical not: pop a value, push its boolean negation.
    Not,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    /// Unconditional jump to an absolute instruction index.
    Jump { target: usize },
    /// Pop the top value; jump if it is falsy.
    JumpIfFalse { target: usize },
    /// Jump backwards to an absolute instruction index (loops).
    Loop { target: usize },
    /// Call a callable with n arguments.
    Call(u8),
    /// Return from the current function (pops the return value).
    Return,
    /// Create a closure from a function constant, capturing upvalues.
    Closure { function: u32, upvalues: Vec<UpvalueDesc> },
    /// Close the open upvalue pointing at the top of the stack, then pop.
    CloseUpvalue,
    /// Rotate the top three stack values: `[a, b, c] -> [b, c, a]`.
    Rotate3,
    /// Pop n values and build a list.
    BuildList(u32),
    /// Pop 2n values and build a map from key/value pairs.
    BuildMap(u32),
    /// Pop a start and an end value and build a range.
    BuildRange { inclusive: bool },
}

/// A compiled Corros function: bytecode plus metadata.
#[derive(Debug)]
pub struct Function {
    /// The function's name, or "<script>" for the top-level program.
    pub name: String,
    /// Source file the function was compiled from (for tracebacks).
    pub file: String,
    /// Number of parameters.
    pub arity: u8,
    pub chunk: Chunk,
    /// Number of upvalues captured from enclosing scopes.
    pub upvalue_count: u8,
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<craft {}>", self.name)
    }
}

/// A runnable unit of bytecode: instructions, line info, and constants.
#[derive(Debug, Default)]
pub struct Chunk {
    pub code: Vec<OpCode>,
    /// Source line for each instruction (parallel to `code`).
    pub lines: Vec<u32>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, op: OpCode, line: u32) {
        self.code.push(op);
        self.lines.push(line);
    }

    /// Add a constant to the pool, returning its index.
    pub fn add_constant(&mut self, value: Value) -> u32 {
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }
}

impl Function {
    pub fn new(name: impl Into<String>, arity: u8) -> Self {
        Function {
            name: name.into(),
            file: String::new(),
            arity,
            chunk: Chunk::new(),
            upvalue_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Disassembler
// ---------------------------------------------------------------------------

/// Disassemble a whole chunk into a readable listing.
pub fn disassemble_chunk(chunk: &Chunk, name: &str) -> String {
    let mut out = format!("== {} ==\n", name);
    for (i, op) in chunk.code.iter().enumerate() {
        out.push_str(&disassemble_instruction(chunk, i, op));
    }
    out
}

fn disassemble_instruction(chunk: &Chunk, index: usize, op: &OpCode) -> String {
    let line = chunk.lines.get(index).copied().unwrap_or(0);
    let mut out = format!("{:04} {:>5}  ", index, line);
    match op {
        OpCode::Constant(c) => {
            out.push_str(&format!(
                "OP_CONSTANT {:>4} '{}'\n",
                c,
                chunk.constants.get(*c as usize).map(Value::repr).unwrap_or_default()
            ));
        }
        OpCode::GetLocal(s) | OpCode::SetLocal(s) => {
            out.push_str(&format!("{} {:>4}\n", op_name(op), s));
        }
        OpCode::GetUpvalue(s) | OpCode::SetUpvalue(s) => {
            out.push_str(&format!("{} {:>4}\n", op_name(op), s));
        }
        OpCode::DefineGlobal(c) | OpCode::GetGlobal(c) | OpCode::SetGlobal(c) | OpCode::GetField(c) => {
            let name = chunk
                .constants
                .get(*c as usize)
                .map(Value::repr)
                .unwrap_or_default();
            out.push_str(&format!("{} {:>4} '{}'\n", op_name(op), c, name));
        }
        OpCode::Jump { target } | OpCode::JumpIfFalse { target } => {
            out.push_str(&format!("{} {:>4} -> {:04}\n", op_name(op), *target as i32, target));
        }
        OpCode::Loop { target } => {
            out.push_str(&format!("{} {:>4} -> {:04}\n", op_name(op), *target as i32, target));
        }
        OpCode::Call(n) => {
            out.push_str(&format!("OP_CALL {:>4}\n", n));
        }
        OpCode::Rotate3 => {
            out.push_str("OP_ROTATE3\n");
        }
        OpCode::BuildList(n) => {
            out.push_str(&format!("OP_BUILD_LIST {:>4}\n", n));
        }
        OpCode::BuildMap(n) => {
            out.push_str(&format!("OP_BUILD_MAP {:>4}\n", n));
        }
        OpCode::BuildRange { inclusive } => {
            out.push_str(&format!(
                "OP_BUILD_RANGE {}\n",
                if *inclusive { "inclusive" } else { "exclusive" }
            ));
        }
        OpCode::Closure { function, upvalues } => {
            let name = chunk
                .constants
                .get(*function as usize)
                .map(Value::repr)
                .unwrap_or_default();
            out.push_str(&format!(
                "OP_CLOSURE {:>4} '{}'\n",
                function, name
            ));
            for (i, up) in upvalues.iter().enumerate() {
                out.push_str(&format!(
                    "      {:04}    | {} {} upvalue {}\n",
                    index,
                    i,
                    if up.is_local { "local" } else { "upvalue" },
                    up.index
                ));
            }
        }
        _ => {
            out.push_str(op_name(op));
            out.push('\n');
        }
    }
    out
}

fn op_name(op: &OpCode) -> &'static str {
    match op {
        OpCode::Constant(_) => "OP_CONSTANT",
        OpCode::Nil => "OP_NIL",
        OpCode::True => "OP_TRUE",
        OpCode::False => "OP_FALSE",
        OpCode::Pop => "OP_POP",
        OpCode::GetLocal(_) => "OP_GET_LOCAL",
        OpCode::SetLocal(_) => "OP_SET_LOCAL",
        OpCode::DefineGlobal(_) => "OP_DEFINE_GLOBAL",
        OpCode::GetGlobal(_) => "OP_GET_GLOBAL",
        OpCode::SetGlobal(_) => "OP_SET_GLOBAL",
        OpCode::GetUpvalue(_) => "OP_GET_UPVALUE",
        OpCode::SetUpvalue(_) => "OP_SET_UPVALUE",
        OpCode::GetIndex => "OP_GET_INDEX",
        OpCode::SetIndex => "OP_SET_INDEX",
        OpCode::GetField(_) => "OP_GET_FIELD",
        OpCode::Add => "OP_ADD",
        OpCode::Subtract => "OP_SUBTRACT",
        OpCode::Multiply => "OP_MULTIPLY",
        OpCode::Divide => "OP_DIVIDE",
        OpCode::Modulo => "OP_MODULO",
        OpCode::Power => "OP_POWER",
        OpCode::Negate => "OP_NEGATE",
        OpCode::Not => "OP_NOT",
        OpCode::Equal => "OP_EQUAL",
        OpCode::NotEqual => "OP_NOT_EQUAL",
        OpCode::Less => "OP_LESS",
        OpCode::LessEqual => "OP_LESS_EQUAL",
        OpCode::Greater => "OP_GREATER",
        OpCode::GreaterEqual => "OP_GREATER_EQUAL",
        OpCode::Jump { .. } => "OP_JUMP",
        OpCode::JumpIfFalse { .. } => "OP_JUMP_IF_FALSE",
        OpCode::Loop { .. } => "OP_LOOP",
        OpCode::Call(_) => "OP_CALL",
        OpCode::Rotate3 => "OP_ROTATE3",
        OpCode::Return => "OP_RETURN",
        OpCode::Closure { .. } => "OP_CLOSURE",
        OpCode::CloseUpvalue => "OP_CLOSE_UPVALUE",
        OpCode::BuildList(_) => "OP_BUILD_LIST",
        OpCode::BuildMap(_) => "OP_BUILD_MAP",
        OpCode::BuildRange { .. } => "OP_BUILD_RANGE",
    }
}
