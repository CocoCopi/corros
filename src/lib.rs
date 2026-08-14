//! Corros: a scripting language with its own lexer, bytecode compiler, and
//! virtual machine, written in Rust.
//!
//! The pipeline is: [`lexer`] → [`compiler`] (emits bytecode directly) →
//! [`vm`] (a stack-based interpreter). [`loader`] handles files and `adopt`
//! splicing, [`stdlib`] provides builtins and methods, and [`repl`] provides
//! the interactive shell.

pub mod bc;
pub mod chunk;
pub mod compiler;
pub mod error;
pub mod lexer;
pub mod loader;
pub mod repl;
pub mod stdlib;
pub mod value;
pub mod vm;
