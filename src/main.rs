//! Corros's command-line interface.
//!
//! Every program runs through the Corros-written interpreter: the seed runs
//! `src/compiler.cor` (the Corros compiler, which compiles your program to
//! bytecode) and then `src/vm.cor` (the Corros VM, which executes it).
//!
//! Usage:
//!   corros [options] [file.cor] [args...]
//!   corros          start the REPL
//!   corros --dump f compile f and print its bytecode
//!   corros --run-bc f.bc [args...]  run compiled bytecode (native executor)
//!   corros --reference f  run f through the Corros-written VM (src/vm.cor)
//!   corros -v       print the version

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut dump = false;
    let mut run_bc = false;
    let mut reference = false;
    let mut path: Option<String> = None;
    let mut script_args: Vec<String> = Vec::new();

    let mut i = 0;
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
            "--reference" => reference = true,
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
        None => repl(),
        Some(file) => {
            if run_bc {
                // Execute pre-compiled bytecode text (native executor, or the
                // Corros-written VM under --reference).
                let result = if reference {
                    corros::seed::run_vm_on_reference(&file, &script_args, true)
                } else {
                    corros::seed::run_vm_on(&file, &script_args, true)
                };
                if let Err(e) = result {
                    eprintln!("error: {}", e);
                    std::process::exit(70);
                }
                return;
            }
            if dump {
                // Compile the file and print the bytecode the Corros compiler emits.
                let result = corros::seed::dump_bytecode(&file);
                match result {
                    Ok(lines) => {
                        for line in lines {
                            println!("{}", line);
                        }
                    }
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(65);
                    }
                }
                return;
            }
            // The native executor by default; the Corros-written VM under
            // --reference.
            let result = if reference {
                corros::seed::run_file_reference(&file, &script_args, true)
            } else {
                corros::run_file(&file, &script_args, true)
            };
            match result {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(70);
                }
            }
        }
    }
}

/// A simple REPL. Each line is a complete program run through the Corros
/// compiler and VM (state does not persist between lines).
fn repl() {
    println!(
        "corros {} — the language written in Corros. Type 'halt' to exit.",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    loop {
        print!("corros> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "halt" || line == "exit" {
            break;
        }
        match corros::run_source(&line, &[]) {
            Ok(lines) => {
                for l in lines {
                    println!("{}", l);
                }
            }
            Err(e) => eprintln!("error: {}", e),
        }
    }
}

fn print_usage() {
    println!(
        "Corros {} — a scripting language whose interpreter is written in Corros\n\
         \n\
         Usage:\n\
         \x20 corros [options] [file.cor] [args...]\n\
         \n\
         Options:\n\
         \x20 -h, --help     show this help\n\
         \x20 -v, --version  print the version\n\
         \x20 --dump FILE    compile FILE and print its bytecode\n\
         \x20 --run-bc FILE  run compiled bytecode (native executor)\n\
         \x20 --reference    run through the Corros-written VM (src/vm.cor)\n\
         \n\
         With no file, starts the interactive REPL.",
        env!("CARGO_PKG_VERSION")
    );
}
