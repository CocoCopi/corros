# Contributing to Corros

Thanks for wanting to help forge Corros! 🦀 Every contribution — bug reports,
docs, tests, and code — makes the language stronger.

## Quick start

```bash
git clone https://github.com/cococopi/corros.git
cd corros
cargo build --release          # builds the `corros` binary
cargo test                     # unit + integration tests
cargo clippy --all-targets     # lint
./target/release/corros        # REPL
./target/release/corros examples/hello.cro
```

No external dependencies — just a Rust toolchain (1.70+).

## CLA — please read before your first pull request

Corros is **dual-licensed** (MIT for the community + a commercial license the
owner sells). For the project to accept and use your work under both licenses,
every contributor must agree to the [Contributor License Agreement](CLA.md).

**How to agree:** in your first pull request, either

- add a comment saying *"I agree to the Corros Contributor License Agreement"*,
  or
- create a file `CLA-signatures/<your-github-username>.md` containing your name,
  GitHub username, and the same statement.

That's it — one time, for all future contributions. You keep ownership of your
code; the CLA just grants the project the rights it needs.

## Good first issues

Look for issues labeled `good first issue`. Great starting points:

- More standard-library tests in `tests/language.rs`
- A new builtin or method (follow the pattern in `src/stdlib.rs`)
- Improving error messages (`src/error.rs`)
- Performance work on the VM (`src/vm.rs`)
- The self-hosting milestone (`selfhost/`): a standard library written in
  Corros itself

## Project layout

| path | what it is |
| --- | --- |
| `src/lexer.rs` | character scanner → tokens |
| `src/compiler.rs` | single-pass bytecode compiler |
| `src/chunk.rs` | bytecode instructions + disassembler |
| `src/vm.rs` | stack-based virtual machine |
| `src/value.rs` | runtime values |
| `src/stdlib.rs` | builtins and methods |
| `src/loader.rs` | file loading + `adopt` |
| `src/repl.rs`, `src/main.rs` | REPL and CLI |
| `src/error.rs` | compile/runtime error rendering |
| `tests/language.rs` | integration tests (`run("...")` → captured output) |
| `examples/*.cro` | runnable Corros programs |
| `selfhost/` | the Corros compiler & VM written in Corros (`demo.sh` proves it) |
| `.claude/skills/corros/` | the Claude Code skill describing the language |

## Style and conventions

- **No new dependencies.** The crate is dependency-free by design.
- **Single-pass compiler.** No AST — the compiler emits bytecode as it parses.
- **Value-based runtime.** `Value` is an enum in `src/value.rs`; new types
  start there.
- **Foundry vocabulary.** Anything user-facing gets a Corros-native name
  (`speak`, `shove`, `weld`, `reforge`, …) — never borrow from other languages.
- Add a test for every new builtin/method/behavior in `tests/language.rs`, and
  run `cargo clippy --all-targets` before pushing.

## Development notes

- Integration tests execute source and compare captured output:
  `assert_eq!(run("speak(1 + 1)"), vec!["2"]);`
- `corros --dump file.cro` prints bytecode — use it when debugging the compiler.
- The REPL wraps input in `speak((...))`, so assignments echo their value.
- On some Android/sdcard mounts, `cargo` incremental compilation locks up; use
  `CARGO_INCREMENTAL=0 cargo build` there.

## Reporting issues

- **Bugs**: include the smallest `.cro` file that reproduces the problem, the
  `corros --dump` output if relevant, and your `corros -v` version.
- **Security**: do **not** open a public issue — see [SECURITY.md](SECURITY.md).

## Code of conduct

Be kind and constructive. Harassment, trolling, and gatekeeping have no place
here. Maintainers may remove off-topic comments and block repeat offenders.

## Getting help

Open a discussion in the GitHub Discussions tab, or reach the owner at
vishalbabuyt04@gmail.com.
