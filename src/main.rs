//! main.rs — a thin launcher.
//!
//! The entire command-line interface — flags, `--dump`, `--run-bc`,
//! `--reference`, and the REPL — lives in `src/cli.cro`, written in Corros.
//! This file only boots it: compile `cli.cro` with the cached compiled
//! compiler and run it on the native executor with the user's arguments.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match corros::seed::run_cli(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(70);
        }
    }
}
