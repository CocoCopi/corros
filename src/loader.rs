//! Source loading: reads files, lexes them, and splices `include` statements.
//!
//! `include "path"` behaves like a text-level splice: the included file's
//! tokens replace the include statement, exactly as if it were pasted in.
//! Includes resolve relative to the including file, and cycles are detected.

use std::path::{Path, PathBuf};

use crate::error::{CompileError, CompileResult, SourceMap};
use crate::lexer::{lex, Token, TokenKind};

/// Read, lex, and preprocess a main program file.
pub fn load_program(path: &str, sources: &mut SourceMap) -> CompileResult<Vec<Token>> {
    let mut stack: Vec<String> = Vec::new();
    let mut tokens = load_file(path, sources, &mut stack)?;
    prepend_prelude(&mut tokens, sources)?;
    Ok(tokens)
}

/// Lex a source string (used by the REPL and tests) and splice includes
/// relative to `base_dir`.
pub fn preprocess(
    source: &str,
    file: &str,
    base_dir: &str,
    sources: &mut SourceMap,
) -> CompileResult<Vec<Token>> {
    sources.insert(file.to_string(), source.to_string());
    let mut tokens = lex(source, file)?;
    let mut stack: Vec<String> = Vec::new();
    splice_includes(&mut tokens, base_dir, sources, &mut stack)?;
    prepend_prelude(&mut tokens, sources)?;
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// The prelude: the standard library written in Corros (lib/prelude.cor).
// It is spliced in front of every program so method calls — which desugar to
// `$method(recv, name, args)` — are implemented in Corros itself.
// ---------------------------------------------------------------------------

/// Locate the prelude source. Search order: $CORROS_PRELUDE, the directory
/// next to the running binary, `../lib` from it, the working directory, and
/// the crate directory (development/tests). Returns `None` when not found,
/// in which case programs compile without the prelude (method calls then fail
/// at runtime with "undefined variable '$method'").
fn read_prelude() -> Option<String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("CORROS_PRELUDE") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(dir) = &exe_dir {
        candidates.push(dir.join("prelude.cor"));
        candidates.push(dir.join("..").join("lib").join("prelude.cor"));
    }
    candidates.push(Path::new("lib/prelude.cor").to_path_buf());
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("lib/prelude.cor"));
    for path in candidates {
        if let Ok(source) = std::fs::read_to_string(&path) {
            return Some(source);
        }
    }
    None
}

fn prepend_prelude(tokens: &mut Vec<Token>, sources: &mut SourceMap) -> CompileResult<()> {
    let source = match read_prelude() {
        Some(s) => s,
        None => return Ok(()),
    };
    sources.insert("<prelude>".to_string(), source.clone());
    let mut pre = lex(&source, "<prelude>")?;
    // Drop the prelude's own Eof token; the program's Eof still ends the file.
    if pre.last().map(|t| t.kind == TokenKind::Eof).unwrap_or(false) {
        pre.pop();
    }
    pre.append(tokens);
    *tokens = pre;
    Ok(())
}

fn load_file(path: &str, sources: &mut SourceMap, stack: &mut Vec<String>) -> CompileResult<Vec<Token>> {
    let source = std::fs::read_to_string(path).map_err(|_| {
        CompileError::new(
            format!("could not open file '{}'", path),
            path,
            1,
            1,
        )
    })?;
    sources.insert(path.to_string(), source.clone());
    let tokens = lex(&source, path)?;
    let base = Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    let mut tokens = tokens;
    splice_includes(&mut tokens, &base, sources, stack)?;
    Ok(tokens)
}

fn splice_includes(
    tokens: &mut Vec<Token>,
    base_dir: &str,
    sources: &mut SourceMap,
    stack: &mut Vec<String>,
) -> CompileResult<()> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.kind != TokenKind::Adopt {
            out.push(token.clone());
            i += 1;
            continue;
        }
        // adopt <string> [;]
        let path_token = tokens.get(i + 1).ok_or_else(|| {
            CompileError::new(
                "expected a file path after 'adopt'",
                &token.file,
                token.line,
                token.column,
            )
        })?;
        let include_path = match &path_token.kind {
            TokenKind::Str(s) => s.clone(),
            _ => {
                return Err(CompileError::new(
                    "expected a quoted file path after 'adopt'",
                    &path_token.file,
                    path_token.line,
                    path_token.column,
                ));
            }
        };
        let full = resolve_path(base_dir, &include_path);
        let full_str = full.to_string_lossy().to_string();
        if stack.contains(&full_str) {
            return Err(CompileError::new(
                format!("circular adopt of '{}'", include_path),
                &path_token.file,
                path_token.line,
                path_token.column,
            ));
        }
        stack.push(full_str.clone());
        let mut included = load_file(&full_str, sources, stack)?;
        stack.pop();
        // Strip the included file's Eof token: it would otherwise terminate
        // compilation of the whole program mid-stream.
        if included.last().map(|t| t.kind == TokenKind::Eof).unwrap_or(false) {
            included.pop();
        }
        out.append(&mut included);
        i += 2;
        if tokens.get(i).map(|t| t.kind == TokenKind::Semicolon).unwrap_or(false) {
            i += 1;
        }
    }
    *tokens = out;
    Ok(())
}

fn resolve_path(base_dir: &str, include_path: &str) -> PathBuf {
    let p = Path::new(include_path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(base_dir).join(p)
    }
}
