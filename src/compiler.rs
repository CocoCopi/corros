//! The Corros compiler: parses source tokens and emits bytecode in a single pass.
//!
//! This is a recursive-descent parser that compiles straight to [`OpCode`]
//! instructions (the same design as Lua and CPython), so there is no separate
//! AST. It handles scoping, closures (upvalue capture), control flow, globals,
//! and all of Corros's expressions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::chunk::{Chunk, Function, OpCode, UpvalueDesc};
use crate::error::{CompileError, CompileResult};
use crate::lexer::{Token, TokenKind};
use crate::value::Value;

/// What kind of function is being compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FnType {
    Script,
    Function,
}

/// A local variable currently in scope.
#[derive(Debug)]
struct Local {
    name: String,
    depth: usize,
    is_captured: bool,
}

/// An upvalue entry of the function being compiled. `desc` is what gets
/// encoded into `OP_CLOSURE`; the name is kept for resolution by name.
#[derive(Debug)]
struct UpvalueEntry {
    name: String,
    desc: UpvalueDesc,
}

/// Loop metadata used to patch `break`/`continue` jumps.
#[derive(Debug)]
struct LoopInfo {
    continue_target: Option<usize>,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    /// Scope depth when the loop started; `break`/`onward` pop locals above
    /// this depth so the stack stays balanced when jumping out of the body.
    depth: usize,
}

impl LoopInfo {
    fn new() -> Self {
        LoopInfo {
            continue_target: None,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            depth: 0,
        }
    }
}

/// State for one function being compiled. Functions nest, so these live on a
/// stack inside [`Compiler`].
#[derive(Debug)]
struct FuncFrame {
    function: Function,
    locals: Vec<Local>,
    upvalues: Vec<UpvalueEntry>,
    scope_depth: usize,
    loops: Vec<LoopInfo>,
    fn_type: FnType,
}

/// An assignment target (`x` or `x[i][j]`). Key expressions are recorded as
/// token spans so they can be re-parsed when the value must be re-read (e.g.
/// `xs[i] += 1` reads `xs[i]` and then writes it back).
enum LValue {
    Name(String),
    Index {
        name: String,
        /// `(key_start, key_end)` token spans of each index expression.
        groups: Vec<(usize, usize)>,
        /// Token position to restore to after re-parsing the keys (the start
        /// of the right-hand side).
        after: usize,
    },
}

pub struct Compiler {
    tokens: Vec<Token>,
    pos: usize,
    frames: Vec<FuncFrame>,
    declared_globals: Rc<RefCell<HashSet<String>>>,
    /// When true, `emit`/`emit_constant` are no-ops. Used while scanning the
    /// key expressions of an indexed lvalue, which are re-parsed later.
    mute: bool,
}

pub fn compile(
    tokens: Vec<Token>,
    declared_globals: Rc<RefCell<HashSet<String>>>,
) -> CompileResult<Rc<Function>> {
    let mut c = Compiler {
        tokens,
        pos: 0,
        frames: Vec::new(),
        declared_globals,
        mute: false,
    };
    let file = c
        .tokens
        .first()
        .map(|t| t.file.clone())
        .unwrap_or_else(|| "<unknown>".to_string());
    let mut function = Function::new("<script>", 0);
    function.file = file;
    c.frames.push(FuncFrame {
        function,
        locals: Vec::new(),
        upvalues: Vec::new(),
        scope_depth: 0,
        loops: Vec::new(),
        fn_type: FnType::Script,
    });

    while !c.at_end() {
        c.statement()?;
    }
    c.emit(OpCode::Nil);
    c.emit(OpCode::Return);
    let frame = c.frames.pop().unwrap();
    Ok(Rc::new(frame.function))
}

impl Compiler {
    // -----------------------------------------------------------------------
    // Token helpers
    // -----------------------------------------------------------------------

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.pos - 1]
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
    }

    fn at_end(&self) -> bool {
        self.current().kind == TokenKind::Eof
    }

    fn check(&self, kind: &TokenKind) -> bool {
        self.current().kind == *kind
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, message: &str) -> CompileResult<()> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn expect_identifier(&mut self, message: &str) -> CompileResult<String> {
        match &self.current().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error(message)),
        }
    }

    fn error(&self, message: impl Into<String>) -> CompileError {
        let t = self.current();
        if t.kind == TokenKind::Eof {
            CompileError::eof(message, &t.file, t.line, t.column)
        } else {
            CompileError::new(message, &t.file, t.line, t.column)
        }
    }

    // -----------------------------------------------------------------------
    // Emission helpers
    // -----------------------------------------------------------------------

    fn chunk(&self) -> &Chunk {
        &self.frames.last().unwrap().function.chunk
    }

    fn chunk_mut(&mut self) -> &mut Chunk {
        &mut self.frames.last_mut().unwrap().function.chunk
    }

    fn emit(&mut self, op: OpCode) {
        if self.mute {
            return;
        }
        let line = self.current().line;
        self.chunk_mut().emit(op, line);
    }

    fn emit_constant(&mut self, value: Value) {
        if self.mute {
            return;
        }
        let idx = self.chunk_mut().add_constant(value);
        self.emit(OpCode::Constant(idx));
    }

    fn name_constant(&mut self, name: &str) -> u32 {
        self.chunk_mut().add_constant(Value::str(name))
    }

    fn current_instruction_index(&self) -> usize {
        self.chunk().code.len()
    }

    fn emit_jump(&mut self) -> usize {
        let idx = self.current_instruction_index();
        self.emit(OpCode::Jump { target: 0 });
        idx
    }

    fn emit_jump_if_false(&mut self) -> usize {
        let idx = self.current_instruction_index();
        self.emit(OpCode::JumpIfFalse { target: 0 });
        idx
    }

    fn patch_jump(&mut self, idx: usize) {
        let target = self.current_instruction_index();
        let is_jump = matches!(self.chunk().code[idx], OpCode::Jump { .. });
        let is_jump_if_false = matches!(self.chunk().code[idx], OpCode::JumpIfFalse { .. });
        if is_jump {
            self.chunk_mut().code[idx] = OpCode::Jump { target };
        } else if is_jump_if_false {
            self.chunk_mut().code[idx] = OpCode::JumpIfFalse { target };
        } else {
            unreachable!("patch_jump called on a non-jump instruction");
        }
    }

    fn emit_global_get(&mut self, name: &str) {
        let c = self.name_constant(name);
        self.emit(OpCode::GetGlobal(c));
    }

    fn emit_global_set(&mut self, name: &str) {
        let c = self.name_constant(name);
        self.emit(OpCode::SetGlobal(c));
    }

    fn emit_define_global(&mut self, name: &str) {
        let c = self.name_constant(name);
        self.emit(OpCode::DefineGlobal(c));
    }

    // -----------------------------------------------------------------------
    // Scoping
    // -----------------------------------------------------------------------

    fn is_top_level(&self) -> bool {
        self.frames.len() == 1 && self.frames[0].scope_depth == 0
    }

    fn begin_scope(&mut self) {
        self.frames.last_mut().unwrap().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        let depth = self.frames.last().unwrap().scope_depth;
        loop {
            let (is_captured, in_scope) = {
                let frame = self.frames.last().unwrap();
                match frame.locals.last() {
                    Some(local) => (local.is_captured, local.depth >= depth),
                    None => (false, false),
                }
            };
            if !in_scope {
                break;
            }
            if is_captured {
                self.emit(OpCode::CloseUpvalue);
            } else {
                self.emit(OpCode::Pop);
            }
            self.frames.last_mut().unwrap().locals.pop();
        }
        self.frames.last_mut().unwrap().scope_depth -= 1;
    }

    /// Emit pops (or closes) for every local declared deeper than `depth`,
    /// without touching the compile-time locals list: the loop body's own
    /// end_scope still owns those entries (it runs on the fallthrough path),
    /// while these pops run only on the `break`/`onward` jump paths.
    fn pop_locals_above(&mut self, depth: usize) {
        let above: Vec<bool> = self
            .frames
            .last()
            .unwrap()
            .locals
            .iter()
            .filter(|l| l.depth > depth)
            .map(|l| l.is_captured)
            .collect();
        for is_captured in above {
            if is_captured {
                self.emit(OpCode::CloseUpvalue);
            } else {
                self.emit(OpCode::Pop);
            }
        }
    }

    fn declare_local(&mut self, name: String) -> CompileResult<u8> {
        let (too_many, redeclared, depth, slot) = {
            let frame = self.frames.last().unwrap();
            if frame.locals.len() >= 255 {
                (true, false, 0, 0)
            } else {
                let redeclared = frame
                    .locals
                    .last()
                    .map(|l| l.name == name && l.depth == frame.scope_depth)
                    .unwrap_or(false);
                (false, redeclared, frame.scope_depth, frame.locals.len() as u8)
            }
        };
        if too_many {
            return Err(CompileError::new(
                "too many local variables in function (max 255)",
                &self.current().file,
                self.current().line,
                self.current().column,
            ));
        }
        if redeclared {
            return Err(self.error(format!(
                "variable '{}' is already declared in this scope",
                name
            )));
        }
        self.frames.last_mut().unwrap().locals.push(Local {
            name,
            depth,
            is_captured: false,
        });
        Ok(slot)
    }

    fn local_in_current_scope(&self, name: &str) -> bool {
        let frame = self.frames.last().unwrap();
        frame
            .locals
            .iter()
            .rev()
            .any(|l| l.name == name && l.depth == frame.scope_depth)
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        let frame = self.frames.last().unwrap();
        frame
            .locals
            .iter()
            .rposition(|l| l.name == name)
            .map(|i| i as u8)
    }

    fn resolve_upvalue(&mut self, name: &str) -> CompileResult<Option<u8>> {
        if self.frames.len() <= 1 {
            return Ok(None);
        }
        for i in (0..self.frames.len() - 1).rev() {
            // Is it a local of an enclosing function?
            if let Some(slot) = self.frames[i].locals.iter().rposition(|l| l.name == name) {
                self.frames[i].locals[slot].is_captured = true;
                let desc = UpvalueDesc {
                    is_local: true,
                    index: slot as u8,
                };
                return Ok(Some(self.add_upvalue(desc, name)?));
            }
            // Is it an upvalue of an enclosing function?
            if let Some(entry) = self.frames[i].upvalues.iter().find(|e| e.name == name) {
                let desc = UpvalueDesc {
                    is_local: false,
                    index: entry.desc.index,
                };
                return Ok(Some(self.add_upvalue(desc, name)?));
            }
        }
        Ok(None)
    }

    fn add_upvalue(&mut self, desc: UpvalueDesc, name: &str) -> CompileResult<u8> {
        let frame = self.frames.last_mut().unwrap();
        if let Some(pos) = frame.upvalues.iter().position(|e| e.desc == desc) {
            return Ok(pos as u8);
        }
        if frame.upvalues.len() >= 255 {
            return Err(CompileError::new(
                "too many upvalues in closure (max 255)",
                &self.current().file,
                self.current().line,
                self.current().column,
            ));
        }
        frame
            .upvalues
            .push(UpvalueEntry { name: name.to_string(), desc });
        Ok((frame.upvalues.len() - 1) as u8)
    }

    fn emit_name_get(&mut self, name: &str) -> CompileResult<()> {
        if let Some(slot) = self.resolve_local(name) {
            self.emit(OpCode::GetLocal(slot));
        } else if let Some(idx) = self.resolve_upvalue(name)? {
            self.emit(OpCode::GetUpvalue(idx));
        } else {
            self.emit_global_get(name);
        }
        Ok(())
    }

    fn emit_name_set(&mut self, name: &str) -> CompileResult<()> {
        if let Some(slot) = self.resolve_local(name) {
            self.emit(OpCode::SetLocal(slot));
        } else if let Some(idx) = self.resolve_upvalue(name)? {
            self.emit(OpCode::SetUpvalue(idx));
        } else {
            // Assigning to an unknown name creates the global (like JS).
            self.emit_global_set(name);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn statement(&mut self) -> CompileResult<()> {
        match &self.current().kind {
            TokenKind::Forge => self.forge_declaration()?,
            TokenKind::When => self.when_statement()?,
            TokenKind::Whilst => self.whilst_statement()?,
            TokenKind::Each => self.each_statement()?,
            TokenKind::Craft => self.craft_declaration()?,
            TokenKind::Return => self.return_statement()?,
            TokenKind::Break => self.break_statement()?,
            TokenKind::Onward => self.onward_statement()?,
            TokenKind::LBrace => {
                self.block()?;
            }
            TokenKind::Semicolon => {
                self.advance();
            }
            TokenKind::Eof => {}
            _ => {
                // Every expression statement leaves a value (assignment
                // instructions keep it on the stack), so pop it uniformly.
                self.expression()?;
                self.emit(OpCode::Pop);
            }
        }
        self.match_token(&TokenKind::Semicolon);
        Ok(())
    }

    fn block(&mut self) -> CompileResult<()> {
        self.expect(&TokenKind::LBrace, "expected '{' to begin a block")?;
        self.begin_scope();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            self.statement()?;
        }
        self.expect(&TokenKind::RBrace, "expected '}' after block")?;
        self.end_scope();
        Ok(())
    }

    fn forge_declaration(&mut self) -> CompileResult<()> {
        self.advance(); // 'forge'
        let name = self.expect_identifier("expected variable name after 'forge'")?;
        if self.is_top_level() {
            if self.declared_globals.borrow().contains(&name) {
                return Err(self.error(format!("variable '{}' is already declared", name)));
            }
        } else if self.local_in_current_scope(&name) {
            return Err(self.error(format!(
                "variable '{}' is already declared in this scope",
                name
            )));
        }
        if self.match_token(&TokenKind::Equal) {
            self.expression()?;
        } else {
            self.emit(OpCode::Nil);
        }
        if self.is_top_level() {
            self.declared_globals.borrow_mut().insert(name.clone());
            self.emit_define_global(&name);
        } else {
            self.declare_local(name)?;
        }
        Ok(())
    }

    fn craft_declaration(&mut self) -> CompileResult<()> {
        self.advance(); // 'craft'
        let name = self.expect_identifier("expected function name after 'craft'")?;
        let params = self.parameter_list()?;
        let (function, upvalues) = self.compile_function(name.clone(), params)?;
        let idx = self.chunk_mut().add_constant(Value::Function(function));
        self.emit(OpCode::Closure {
            function: idx,
            upvalues,
        });
        if self.is_top_level() {
            if self.declared_globals.borrow().contains(&name) {
                return Err(self.error(format!("function '{}' is already declared", name)));
            }
            self.declared_globals.borrow_mut().insert(name.clone());
            self.emit_define_global(&name);
        } else {
            self.declare_local(name)?;
        }
        Ok(())
    }

    fn return_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'return'
        if self.frames.len() == 1 && self.frames[0].fn_type == FnType::Script {
            return Err(self.error("'return' is only allowed inside functions"));
        }
        let ends_here = matches!(
            self.current().kind,
            TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Eof
        );
        if ends_here {
            self.emit(OpCode::Nil);
        } else {
            self.expression()?;
        }
        self.emit(OpCode::Return);
        Ok(())
    }

    fn break_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'break'
        let has_loop = !self.frames.last().unwrap().loops.is_empty();
        if !has_loop {
            return Err(self.error("'break' is only allowed inside a loop"));
        }
        // Pop the loop body's locals here: the body's end_scope pops live
        // below the jump that carries us out of the loop.
        let loop_depth = self.frames.last().unwrap().loops.last().unwrap().depth;
        self.pop_locals_above(loop_depth);
        let jump = self.emit_jump();
        self.frames
            .last_mut()
            .unwrap()
            .loops
            .last_mut()
            .unwrap()
            .break_jumps
            .push(jump);
        Ok(())
    }

    fn onward_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'onward'
        let has_loop = !self.frames.last().unwrap().loops.is_empty();
        if !has_loop {
            return Err(self.error("'onward' is only allowed inside a loop"));
        }
        // Same as break: drop the body's locals before continuing the loop.
        let loop_depth = self.frames.last().unwrap().loops.last().unwrap().depth;
        self.pop_locals_above(loop_depth);
        let target = self
            .frames
            .last()
            .unwrap()
            .loops
            .last()
            .unwrap()
            .continue_target;
        match target {
            Some(target) => {
                self.emit(OpCode::Jump { target });
            }
            None => {
                let jump = self.emit_jump();
                self.frames
                    .last_mut()
                    .unwrap()
                    .loops
                    .last_mut()
                    .unwrap()
                    .continue_jumps
                    .push(jump);
            }
        }
        Ok(())
    }

    fn when_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'when'
        self.expression()?;
        let exit_jump = self.emit_jump_if_false();
        self.block()?;
        if self.match_token(&TokenKind::Else) {
            let else_jump = self.emit_jump();
            self.patch_jump(exit_jump);
            if self.check(&TokenKind::When) {
                self.when_statement()?;
            } else {
                self.block()?;
            }
            self.patch_jump(else_jump);
        } else {
            self.patch_jump(exit_jump);
        }
        Ok(())
    }

    fn whilst_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'whilst'
        let loop_start = self.current_instruction_index();
        let depth = self.frames.last().unwrap().scope_depth;
        self.frames
            .last_mut()
            .unwrap()
            .loops
            .push(LoopInfo {
                continue_target: Some(loop_start),
                depth,
                ..LoopInfo::new()
            });
        self.expression()?;
        let exit_jump = self.emit_jump_if_false();
        self.block()?;
        self.emit(OpCode::Loop { target: loop_start });
        self.patch_jump(exit_jump);
        let li = self.frames.last_mut().unwrap().loops.pop().unwrap();
        for jump in li.break_jumps {
            self.patch_jump(jump);
        }
        Ok(())
    }

    fn each_statement(&mut self) -> CompileResult<()> {
        self.advance(); // 'each'
        let name = self.expect_identifier("expected loop variable name after 'each'")?;
        self.expect(&TokenKind::In, "expected 'in' after loop variable")?;
        self.begin_scope();

        // Hidden locals: the iterator, the counter, and a Nil placeholder for
        // the loop variable's slot (so SetLocal below always has a slot to
        // write into, keeping the stack at a fixed depth across iterations).
        let it_slot = self.declare_local("$it".to_string())?;
        self.expression()?;
        let i_slot = self.declare_local("$i".to_string())?;
        self.emit_constant(Value::num(0.0));
        let name_slot = self.declare_local(name)?;
        self.emit(OpCode::Nil);

        let loop_start = self.current_instruction_index();
        // Condition: $i < size($it)
        self.emit(OpCode::GetLocal(i_slot));
        self.emit_global_get("size");
        self.emit(OpCode::GetLocal(it_slot));
        self.emit(OpCode::Call(1));
        self.emit(OpCode::Less);
        let exit_jump = self.emit_jump_if_false();

        // Loop variable: $it[$i], written into its slot in place each pass.
        // SetLocal keeps the value on the stack, so a Pop restores balance.
        self.emit(OpCode::GetLocal(it_slot));
        self.emit(OpCode::GetLocal(i_slot));
        self.emit(OpCode::GetIndex);
        self.emit(OpCode::SetLocal(name_slot));
        self.emit(OpCode::Pop);

        let depth = self.frames.last().unwrap().scope_depth;
        self.frames
            .last_mut()
            .unwrap()
            .loops
            .push(LoopInfo {
                depth,
                ..LoopInfo::new()
            });
        self.block()?;

        // Continue target: the increment.
        let continue_target = self.current_instruction_index();
        {
            let li = self.frames.last_mut().unwrap().loops.last_mut().unwrap();
            li.continue_target = Some(continue_target);
            let jumps = std::mem::take(&mut li.continue_jumps);
            for jump in jumps {
                self.patch_jump(jump);
            }
        }
        self.emit(OpCode::GetLocal(i_slot));
        self.emit_constant(Value::num(1.0));
        self.emit(OpCode::Add);
        self.emit(OpCode::SetLocal(i_slot));
        self.emit(OpCode::Pop); // SetLocal keeps the value; drop it.
        self.emit(OpCode::Loop { target: loop_start });
        self.patch_jump(exit_jump);

        let li = self.frames.last_mut().unwrap().loops.pop().unwrap();
        for jump in li.break_jumps {
            self.patch_jump(jump);
        }
        self.end_scope();
        Ok(())
    }

    fn compile_function(
        &mut self,
        name: String,
        params: Vec<String>,
    ) -> CompileResult<(Rc<Function>, Vec<UpvalueDesc>)> {
        let file = self.current().file.clone();
        let mut function = Function::new(name, params.len() as u8);
        function.file = file;
        let mut locals = Vec::with_capacity(params.len());
        for p in &params {
            if locals.iter().any(|l: &Local| l.name == *p) {
                return Err(self.error(format!("duplicate parameter '{}'", p)));
            }
            locals.push(Local {
                name: p.clone(),
                depth: 0,
                is_captured: false,
            });
        }
        self.frames.push(FuncFrame {
            function,
            locals,
            upvalues: Vec::new(),
            scope_depth: 0,
            loops: Vec::new(),
            fn_type: FnType::Function,
        });
        // Body is a block.
        self.block()?;
        self.emit(OpCode::Nil);
        self.emit(OpCode::Return);
        let frame = self.frames.pop().unwrap();
        let upvalues: Vec<UpvalueDesc> = frame.upvalues.iter().map(|e| e.desc).collect();
        let mut function = frame.function;
        function.upvalue_count = upvalues.len() as u8;
        Ok((Rc::new(function), upvalues))
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    fn expression(&mut self) -> CompileResult<()> {
        self.assignment()
    }

    fn is_assignment_op(kind: &TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Equal
                | TokenKind::PlusEqual
                | TokenKind::MinusEqual
                | TokenKind::StarEqual
                | TokenKind::SlashEqual
                | TokenKind::PercentEqual
                | TokenKind::PowerEqual
        )
    }

    /// Look ahead to see whether the current token begins an assignment
    /// statement (`name` or `name[...][...]` followed by an assignment op).
    fn is_lvalue_assignment(&self) -> bool {
        if !matches!(self.current().kind, TokenKind::Identifier(_)) {
            return false;
        }
        let mut i = self.pos + 1;
        let mut depth = 0i32;
        loop {
            let kind = match self.tokens.get(i) {
                Some(t) => &t.kind,
                None => return false,
            };
            match kind {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => depth -= 1,
                TokenKind::Eof => return false,
                _ if depth == 0 => return Self::is_assignment_op(kind),
                _ => {}
            }
            i += 1;
        }
    }

    fn assignment(&mut self) -> CompileResult<()> {
        if self.is_lvalue_assignment() {
            let mut lv = self.parse_lvalue()?;
            self.advance(); // consume the assignment operator
            let op = self.previous().kind.clone();
            // Reparsing an indexed target rewinds the token stream; record
            // where the right-hand side begins so it can be restored.
            if let LValue::Index { after, .. } = &mut lv {
                *after = self.pos;
            }
            match op {
                TokenKind::Equal => {
                    match &lv {
                        LValue::Name(_) => {}
                        LValue::Index { .. } => self.emit_lvalue_target(&lv)?,
                    }
                    self.expression()?;
                    self.emit_lvalue_write(&lv)?;
                }
                op => {
                    // Compound assignment: target = target OP rhs.
                    self.emit_lvalue_get(&lv)?;
                    self.expression()?;
                    self.emit_binary_op(&op)?;
                    // Indexed targets re-emit container+keys for the write;
                    // Rotate3 puts the result on top of them for SetIndex.
                    if let LValue::Index { .. } = &lv {
                        self.emit_lvalue_target(&lv)?;
                        self.emit(OpCode::Rotate3);
                    }
                    self.emit_lvalue_write(&lv)?;
                }
            }
        } else {
            self.parse_or()?;
            if Self::is_assignment_op(&self.current().kind) {
                return Err(self.error("invalid assignment target"));
            }
        }
        Ok(())
    }

    /// Parse an assignment target without emitting any code.
    fn parse_lvalue(&mut self) -> CompileResult<LValue> {
        let name = self.expect_identifier("expected variable name")?;
        if !self.check(&TokenKind::LBracket) {
            return Ok(LValue::Name(name));
        }
        let mut groups = Vec::new();
        while self.match_token(&TokenKind::LBracket) {
            let key_start = self.pos;
            // Scan the key expression muted: its code is emitted later when
            // the target is re-parsed for the actual read/write.
            self.mute = true;
            self.expression()?;
            self.mute = false;
            let key_end = self.pos;
            self.expect(&TokenKind::RBracket, "expected ']' after index")?;
            groups.push((key_start, key_end));
        }
        let after = self.pos;
        Ok(LValue::Index {
            name,
            groups,
            after,
        })
    }

    /// Re-emit the container and key expressions of an indexed lvalue from
    /// their recorded token spans, folding intermediates with GetIndex, and
    /// restore the token position afterwards.
    ///
    /// With `fold_all` the final value is left on the stack
    /// (`[container[k1][k2]...]`, a read). Otherwise the last key is left
    /// exposed (`[container[k1][k2..], keyN]`) for a SetIndex write.
    fn reparse_keys(&mut self, lv: &LValue, fold_all: bool) -> CompileResult<()> {
        if let LValue::Index {
            name,
            groups,
            after,
        } = lv
        {
            self.emit_name_get(name)?;
            for (i, (key_start, key_end)) in groups.iter().enumerate() {
                self.pos = *key_start;
                self.expression()?;
                self.pos = *key_end;
                if fold_all || i + 1 < groups.len() {
                    self.emit(OpCode::GetIndex);
                }
            }
            self.pos = *after;
        }
        Ok(())
    }

    /// Emit `[container, key...]` for a subsequent SetIndex write.
    fn emit_lvalue_target(&mut self, lv: &LValue) -> CompileResult<()> {
        match lv {
            LValue::Name(name) => self.emit_name_get(name)?,
            LValue::Index { .. } => self.reparse_keys(lv, false)?,
        }
        Ok(())
    }

    /// Emit a full read of the lvalue, leaving `[value]` on the stack.
    fn emit_lvalue_get(&mut self, lv: &LValue) -> CompileResult<()> {
        match lv {
            LValue::Name(name) => self.emit_name_get(name)?,
            LValue::Index { .. } => self.reparse_keys(lv, true)?,
        }
        Ok(())
    }

    fn emit_lvalue_write(&mut self, lv: &LValue) -> CompileResult<()> {
        match lv {
            LValue::Name(name) => self.emit_name_set(name)?,
            LValue::Index { .. } => self.emit(OpCode::SetIndex),
        }
        Ok(())
    }

    fn emit_binary_op(&mut self, op: &TokenKind) -> CompileResult<()> {
        let code = match op {
            TokenKind::PlusEqual => OpCode::Add,
            TokenKind::MinusEqual => OpCode::Subtract,
            TokenKind::StarEqual => OpCode::Multiply,
            TokenKind::SlashEqual => OpCode::Divide,
            TokenKind::PercentEqual => OpCode::Modulo,
            TokenKind::PowerEqual => OpCode::Power,
            _ => unreachable!("not a compound assignment"),
        };
        self.emit(code);
        Ok(())
    }

    fn parse_or(&mut self) -> CompileResult<()> {
        self.parse_and()?;
        while self.match_token(&TokenKind::Or) {
            let j = self.emit_jump_if_false();
            self.emit(OpCode::True);
            let j2 = self.emit_jump();
            self.patch_jump(j);
            self.parse_and()?;
            self.emit(OpCode::Not);
            self.emit(OpCode::Not);
            self.patch_jump(j2);
        }
        Ok(())
    }

    fn parse_and(&mut self) -> CompileResult<()> {
        self.parse_range()?;
        while self.match_token(&TokenKind::And) {
            let j = self.emit_jump_if_false();
            self.parse_range()?;
            self.emit(OpCode::Not);
            self.emit(OpCode::Not);
            let j2 = self.emit_jump();
            self.patch_jump(j);
            self.emit(OpCode::False);
            self.patch_jump(j2);
        }
        Ok(())
    }

    fn parse_range(&mut self) -> CompileResult<()> {
        self.parse_equality()?;
        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEqual) {
            let inclusive = self.check(&TokenKind::DotDotEqual);
            self.advance();
            self.parse_or()?;
            self.emit(OpCode::BuildRange { inclusive });
        }
        Ok(())
    }

    fn parse_equality(&mut self) -> CompileResult<()> {
        self.parse_comparison()?;
        while self.match_token(&TokenKind::EqualEqual) || self.match_token(&TokenKind::BangEqual) {
            let is_eq = self.previous().kind == TokenKind::EqualEqual;
            self.parse_comparison()?;
            self.emit(if is_eq {
                OpCode::Equal
            } else {
                OpCode::NotEqual
            });
        }
        Ok(())
    }

    fn parse_comparison(&mut self) -> CompileResult<()> {
        self.parse_term()?;
        while self.check(&TokenKind::Less)
            || self.check(&TokenKind::LessEqual)
            || self.check(&TokenKind::Greater)
            || self.check(&TokenKind::GreaterEqual)
        {
            let op = self.current().kind.clone();
            self.advance();
            self.parse_term()?;
            let code = match op {
                TokenKind::Less => OpCode::Less,
                TokenKind::LessEqual => OpCode::LessEqual,
                TokenKind::Greater => OpCode::Greater,
                TokenKind::GreaterEqual => OpCode::GreaterEqual,
                _ => unreachable!(),
            };
            self.emit(code);
        }
        Ok(())
    }

    fn parse_term(&mut self) -> CompileResult<()> {
        self.parse_factor()?;
        while self.match_token(&TokenKind::Plus) || self.match_token(&TokenKind::Minus) {
            let is_plus = self.previous().kind == TokenKind::Plus;
            self.parse_factor()?;
            self.emit(if is_plus {
                OpCode::Add
            } else {
                OpCode::Subtract
            });
        }
        Ok(())
    }

    fn parse_factor(&mut self) -> CompileResult<()> {
        self.parse_unary()?;
        while self.match_token(&TokenKind::Star)
            || self.match_token(&TokenKind::Slash)
            || self.match_token(&TokenKind::Percent)
        {
            let op = self.previous().kind.clone();
            self.parse_unary()?;
            let code = match op {
                TokenKind::Star => OpCode::Multiply,
                TokenKind::Slash => OpCode::Divide,
                TokenKind::Percent => OpCode::Modulo,
                _ => unreachable!(),
            };
            self.emit(code);
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> CompileResult<()> {
        if self.match_token(&TokenKind::Bang) {
            self.parse_unary()?;
            self.emit(OpCode::Not);
            Ok(())
        } else if self.match_token(&TokenKind::Minus) {
            self.parse_unary()?;
            self.emit(OpCode::Negate);
            Ok(())
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> CompileResult<()> {
        self.parse_postfix()?;
        if self.match_token(&TokenKind::Power) {
            // Right-associative; right operand parses at unary level so that
            // `2 ** -2` works, and `-2 ** 2` is -(2 ** 2), like Python.
            self.parse_unary()?;
            self.emit(OpCode::Power);
        }
        Ok(())
    }

    fn parse_postfix(&mut self) -> CompileResult<()> {
        self.parse_primary()?;
        loop {
            if self.check(&TokenKind::LParen) {
                let arg_count = self.argument_list()?;
                self.emit(OpCode::Call(arg_count));
            } else if self.match_token(&TokenKind::LBracket) {
                self.expression()?;
                self.expect(&TokenKind::RBracket, "expected ']' after index")?;
                self.emit(OpCode::GetIndex);
            } else if self.match_token(&TokenKind::Dot) {
                let name = self.expect_identifier("expected method name after '.'")?;
                let c = self.name_constant(&name);
                self.emit(OpCode::GetField(c));
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_primary(&mut self) -> CompileResult<()> {
        match self.current().kind.clone() {
            TokenKind::Number(n) => {
                self.advance();
                self.emit_constant(Value::num(n));
            }
            TokenKind::Str(s) => {
                self.advance();
                self.emit_constant(Value::str(s));
            }
            TokenKind::True => {
                self.advance();
                self.emit(OpCode::True);
            }
            TokenKind::False => {
                self.advance();
                self.emit(OpCode::False);
            }
            TokenKind::Nil => {
                self.advance();
                self.emit(OpCode::Nil);
            }
            TokenKind::Identifier(name) => {
                self.advance();
                self.emit_name_get(&name)?;
            }
            TokenKind::LParen => {
                self.advance();
                self.expression()?;
                self.expect(&TokenKind::RParen, "expected ')' after expression")?;
            }
            TokenKind::LBracket => self.list_literal()?,
            TokenKind::LBrace => self.map_literal()?,
            TokenKind::Craft => {
                self.advance();
                let params = self.parameter_list()?;
                let (function, upvalues) = self.compile_function("(anonymous)".to_string(), params)?;
                let idx = self.chunk_mut().add_constant(Value::Function(function));
                self.emit(OpCode::Closure {
                    function: idx,
                    upvalues,
                });
            }
            _ => {
                return Err(self.error(format!(
                    "expected expression, got {}",
                    describe_token(&self.current().kind)
                )));
            }
        }
        Ok(())
    }

    fn list_literal(&mut self) -> CompileResult<()> {
        self.advance(); // '['
        let mut count = 0u32;
        if !self.check(&TokenKind::RBracket) {
            loop {
                self.expression()?;
                count += 1;
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RBracket) {
                    break; // trailing comma
                }
            }
        }
        self.expect(&TokenKind::RBracket, "expected ']' after list")?;
        self.emit(OpCode::BuildList(count));
        Ok(())
    }

    fn map_literal(&mut self) -> CompileResult<()> {
        self.advance(); // '{'
        let mut count = 0u32;
        if !self.check(&TokenKind::RBrace) {
            loop {
                self.expression()?;
                self.expect(&TokenKind::Colon, "expected ':' after map key")?;
                self.expression()?;
                count += 1;
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RBrace) {
                    break; // trailing comma
                }
            }
        }
        self.expect(&TokenKind::RBrace, "expected '}' after map")?;
        self.emit(OpCode::BuildMap(count));
        Ok(())
    }

    fn parameter_list(&mut self) -> CompileResult<Vec<String>> {
        self.expect(&TokenKind::LParen, "expected '(' before parameters")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let name = self.expect_identifier("expected parameter name")?;
                if params.len() >= 255 {
                    return Err(self.error("too many parameters (max 255)"));
                }
                params.push(name);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RParen) {
                    break; // trailing comma
                }
            }
        }
        self.expect(&TokenKind::RParen, "expected ')' after parameters")?;
        Ok(params)
    }

    fn argument_list(&mut self) -> CompileResult<u8> {
        self.advance(); // '('
        let mut count = 0u8;
        if !self.check(&TokenKind::RParen) {
            loop {
                if count == 255 {
                    return Err(self.error("too many arguments (max 255)"));
                }
                self.expression()?;
                count += 1;
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RParen) {
                    break; // trailing comma
                }
            }
        }
        self.expect(&TokenKind::RParen, "expected ')' after arguments")?;
        Ok(count)
    }
}

fn describe_token(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Eof => "end of input".to_string(),
        TokenKind::Number(_) => "a number".to_string(),
        TokenKind::Str(_) => "a string".to_string(),
        TokenKind::Identifier(_) => "an identifier".to_string(),
        k => format!("'{}'", token_lexeme(k)),
    }
}

fn token_lexeme(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::LParen => "(",
        TokenKind::RParen => ")",
        TokenKind::LBrace => "{",
        TokenKind::RBrace => "}",
        TokenKind::LBracket => "[",
        TokenKind::RBracket => "]",
        TokenKind::Comma => ",",
        TokenKind::Dot => ".",
        TokenKind::Semicolon => ";",
        TokenKind::Colon => ":",
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::Bang => "!",
        TokenKind::BangEqual => "!=",
        TokenKind::Equal => "=",
        TokenKind::EqualEqual => "==",
        TokenKind::Greater => ">",
        TokenKind::GreaterEqual => ">=",
        TokenKind::Less => "<",
        TokenKind::LessEqual => "<=",
        TokenKind::And => "&&",
        TokenKind::Or => "||",
        TokenKind::PlusEqual => "+=",
        TokenKind::MinusEqual => "-=",
        TokenKind::StarEqual => "*=",
        TokenKind::SlashEqual => "/=",
        TokenKind::PercentEqual => "%=",
        TokenKind::Power => "**",
        TokenKind::PowerEqual => "**=",
        TokenKind::DotDot => "..",
        TokenKind::DotDotEqual => "..=",
        TokenKind::Forge => "forge",
        TokenKind::Craft => "craft",
        TokenKind::When => "when",
        TokenKind::Else => "else",
        TokenKind::Whilst => "whilst",
        TokenKind::Each => "each",
        TokenKind::In => "in",
        TokenKind::Return => "return",
        TokenKind::Break => "break",
        TokenKind::Onward => "onward",
        TokenKind::True => "true",
        TokenKind::False => "false",
        TokenKind::Nil => "nil",
        TokenKind::Adopt => "adopt",
        TokenKind::Number(_) => "number",
        TokenKind::Str(_) => "string",
        TokenKind::Identifier(_) => "identifier",
        TokenKind::Eof => "end of input",
    }
}
