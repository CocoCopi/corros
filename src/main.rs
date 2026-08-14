//! Corros's command-line interface.
//!
//! Usage:
//!   corros [options] [file.cor] [args...]
//!   corros          start the REPL
//!   corros --dump f disassemble the compiled bytecode of f
//!   corros --run-bc f.bc [args...]  run compiled bytecode natively
//!   corros -v       print the version

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use corros::bc;
use corros::chunk;
use corros::compiler;
use corros::error::SourceMap;
use corros::loader;
use corros::value::Value;
use corros::vm::VM;
use corros::repl;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut dump = false;
    let mut run_bc = false;
    let mut path: Option<String> = None;
    let mut script_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "-v" | "--version" => {
                println!("corros {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--dump" => dump = true,
            "--run-bc" => run_bc = true,
            s if s.starts_with('-') => {
                eprintln!("corros: unknown option '{}'", s);
                print_usage();
                std::process::exit(2);
            }
            _ => {
                path = Some(args[i].clone());
                script_args = args[i + 1..].to_vec();
                break;
            }
        }
        i += 1;
    }

    match path {
        None => {
            let mut vm = VM::new();
            vm.echo = true;
            let mut sources = SourceMap::new();
            repl::run(&mut vm, &mut sources);
        }
        Some(file) => {
            let mut vm = VM::new();
            vm.echo = true;
            let args_list = Value::List(Rc::new(RefCell::new(
                script_args
                    .into_iter()
                    .map(Value::str)
                    .collect::<Vec<Value>>(),
            )));
            vm.set_global("args", args_list);

            if run_bc {
                // Execute pre-compiled bytecode text (self-hosting chain).
                let text = match std::fs::read_to_string(&file) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("corros: could not open '{}': {}", file, e);
                        std::process::exit(65);
                    }
                };
                let function = match bc::load_bytecode(&text) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("corros: {}", e);
                        std::process::exit(65);
                    }
                };
                if let Err(e) = vm.run(function) {
                    eprint!("{}", e.render());
                    std::process::exit(70);
                }
                return;
            }

            let mut sources = SourceMap::new();
            let tokens = match loader::load_program(&file, &mut sources) {
                Ok(t) => t,
                Err(e) => {
                    eprint!("{}", e.render(&sources));
                    std::process::exit(65);
                }
            };
            let declared = Rc::new(RefCell::new(HashSet::new()));
            let function = match compiler::compile(tokens, declared) {
                Ok(f) => f,
                Err(e) => {
                    eprint!("{}", e.render(&sources));
                    std::process::exit(65);
                }
            };

            if dump {
                dump_functions(&function);
            }

            if let Err(e) = vm.run(function) {
                eprint!("{}", e.render());
                std::process::exit(70);
            }
        }
    }
}

fn dump_functions(function: &std::rc::Rc<corros::chunk::Function>) {
    print!(
        "{}",
        chunk::disassemble_chunk(&function.chunk, &function.name)
    );
    for constant in &function.chunk.constants {
        if let Value::Function(inner) = constant {
            dump_functions(inner);
        }
    }
}

fn print_usage() {
    println!(
        "Corros {} — a bytecode-compiled scripting language\n\
         \n\
         Usage:\n\
         \x20 corros [options] [file.cor] [args...]\n\
         \n\
         Options:\n\
         \x20 -h, --help     show this help\n\
         \x20 -v, --version  print the version\n\
         \x20 --dump FILE    compile FILE and print its bytecode\n\
         \x20 --run-bc FILE  run compiled bytecode (self-hosting chain)\n\
         \n\
         With no file, starts the interactive REPL.",
        env!("CARGO_PKG_VERSION")
    );
}
