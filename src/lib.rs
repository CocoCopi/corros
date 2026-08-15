//! Corros: a scripting language forged from scratch.
//!
//! The language itself is implemented in Corros (`src/compiler.cor`, the
//! lexer + bytecode compiler; `src/vm.cor`, the virtual machine; and
//! `src/prelude.cor`, the standard library). This crate is only the bootstrap
//! seed — the small interpreter in [`seed`] that can boot the Corros-written
//! interpreter, in the same way rustc's first compiler was written in OCaml.
//! [`lexer`] provides the tokenizer the seed needs to read the Corros sources.

pub mod codegen;
pub mod error;
pub mod lexer;
pub mod native;
pub mod seed;

pub use seed::{run_file, run_source};
