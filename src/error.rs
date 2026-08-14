//! Error types for Corros, with source-aware rendering.
//!
//! Compile errors point at a specific token (file / line / column) and can be
//! rendered with a caret pointing at the offending source. Runtime errors carry
//! a stack traceback of Corros function calls.

use std::collections::HashMap;

/// A map from file path to its full source text, used to render errors with
/// the offending source line.
#[derive(Debug, Default, Clone)]
pub struct SourceMap {
    files: HashMap<String, String>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: String, source: String) {
        self.files.insert(path, source);
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|s| s.as_str())
    }

    /// Return the text of line `line` (1-based) in `path`, if available.
    pub fn line(&self, path: &str, line: u32) -> Option<&str> {
        let source = self.files.get(path)?;
        source.lines().nth(line.saturating_sub(1) as usize)
    }
}

/// An error produced while lexing, parsing, or compiling Corros source.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub message: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    /// True when the error is an unexpected end of input. The REPL uses this
    /// to detect multi-line input that should keep reading.
    pub unexpected_eof: bool,
}

impl CompileError {
    pub fn new(message: impl Into<String>, file: &str, line: u32, column: u32) -> Self {
        CompileError {
            message: message.into(),
            file: file.to_string(),
            line,
            column,
            unexpected_eof: false,
        }
    }

    pub fn eof(message: impl Into<String>, file: &str, line: u32, column: u32) -> Self {
        CompileError {
            message: message.into(),
            file: file.to_string(),
            line,
            column,
            unexpected_eof: true,
        }
    }

    /// Render the error as a multi-line, caret-annotated diagnostic.
    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = String::new();
        out.push_str("Error: ");
        out.push_str(&self.message);
        out.push('\n');
        out.push_str(&format!("  --> {}:{}:{}\n", self.file, self.line, self.column));

        if let Some(line_text) = sources.line(&self.file, self.line) {
            let line_no = self.line.to_string();
            let gutter = " ".repeat(line_no.len());
            out.push_str(&format!("{} |\n", gutter));
            out.push_str(&format!("{} | {}\n", line_no, line_text));
            let caret_pad = " ".repeat(self.column.saturating_sub(1) as usize);
            out.push_str(&format!("{} | {}^\n", gutter, caret_pad));
        }
        out
    }
}

/// One entry in a Corros runtime traceback.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceFrame {
    pub function: String,
    pub file: String,
    pub line: u32,
}

/// A runtime error raised by the VM, with a stack traceback of Corros calls.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub trace: Vec<TraceFrame>,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        RuntimeError {
            message: message.into(),
            trace: Vec::new(),
        }
    }

    /// Render as "error: message" followed by a traceback of Corros frames.
    pub fn render(&self) -> String {
        let mut out = format!("error: {}\n", self.message);
        if !self.trace.is_empty() {
            out.push_str("stack traceback:\n");
            for frame in &self.trace {
                out.push_str(&format!(
                    "  at {} ({}:{})\n",
                    frame.function, frame.file, frame.line
                ));
            }
        }
        out
    }
}

pub type CompileResult<T> = Result<T, CompileError>;
pub type RuntimeResult<T> = Result<T, RuntimeError>;
