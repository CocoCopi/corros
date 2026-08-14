//! The interactive read-eval-print loop.
//!
//! Each line is first tried as an expression (so `1 + 1` prints `2`); if that
//! fails it is compiled as statements. Input that ends mid-construct (like an
//! unclosed brace) keeps reading on continuation lines.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::compiler;
use crate::error::SourceMap;
use crate::loader;
use crate::vm::VM;

pub fn run(vm: &mut VM, sources: &mut SourceMap) {
    let declared = Rc::new(RefCell::new(HashSet::new()));
    let stdin = std::io::stdin();
    let mut buffer = String::new();

    println!(
        "Corros {} — a language forged from scratch. Type 'exit' to quit.",
        env!("CARGO_PKG_VERSION")
    );

    loop {
        if buffer.is_empty() {
            print!("corros> ");
        } else {
            print!("..... ");
        }
        std::io::stdout().flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if buffer.is_empty() && matches!(trimmed, "exit" | "quit" | ".exit") {
            break;
        }
        buffer.push_str(&line);
        if !buffer.ends_with('\n') {
            buffer.push('\n');
        }

        // Try as an expression first, with an isolated global-declaration set
        // so a failed attempt can't pollute the real one. Only non-nil results
        // are echoed (like Python's REPL), so `print(x)` doesn't print a
        // trailing `nil`.
        let trial = Rc::new(RefCell::new(HashSet::new()));
        let expr_src = format!(
            "forge __repl_result__ = ({}); when __repl_result__ != nil {{ speak(__repl_result__) }}",
            buffer
        );
        if let Ok(tokens) = loader::preprocess(&expr_src, "<repl>", ".", sources) {
            if let Ok(function) = compiler::compile(tokens, trial.clone()) {
                for name in trial.borrow().iter() {
                    declared.borrow_mut().insert(name.clone());
                }
                sources.insert("<repl>".to_string(), buffer.clone());
                if let Err(e) = vm.run(function) {
                    eprint!("{}", e.render());
                }
                buffer.clear();
                continue;
            }
        }

        // Otherwise compile as statements.
        let preprocessed = match loader::preprocess(&buffer, "<repl>", ".", sources) {
            Ok(t) => t,
            Err(e) => {
                if e.unexpected_eof && buffer.lines().count() < 200 {
                    continue; // keep reading multi-line input
                }
                eprint!("{}", e.render(sources));
                buffer.clear();
                continue;
            }
        };
        match compiler::compile(preprocessed, declared.clone()) {
            Ok(function) => {
                if let Err(e) = vm.run(function) {
                    eprint!("{}", e.render());
                }
                buffer.clear();
            }
            Err(e) => {
                if e.unexpected_eof && buffer.lines().count() < 200 {
                    continue; // keep reading multi-line input
                }
                eprint!("{}", e.render(sources));
                buffer.clear();
            }
        }
    }
}
