# 🦀 Corros

**A programming language forged from scratch — and named for what Rust does best: corrosion.**

Corros is a bytecode-compiled scripting language with its own lexer, its own
compiler, its own virtual machine, and a syntax that belongs to **no other
language**. Rust is written in Rust. Corros is written in Rust — and it already
compiles **itself**: the `selfhost/` directory holds a Corros compiler and a
Corros virtual machine, both written in Corros, proven by a byte-identical
bootstrap chain (`bash selfhost/demo.sh`).

```corros
// This is Corros. Read it out loud.
forge greet = craft(name) {
  return "hello, " + name
}

each i in 1..=5 {
  when i % 2 == 0 { onward }
  speak(greet("corros"), i)
}
```

## Why Corros?

Rust took the systems world by storm with ideas that were its own. Corros does
the same for scripting: a grammar that copies nothing, vocabulary that belongs
to it alone, and an engine — lexer, compiler, bytecode VM — built from zero.

| other languages say | **Corros says** |
| ------------------- | --------------- |
| `let x = 5`         | `forge x = 5`   |
| `fn foo() {}`       | `craft foo() {}` |
| `if cond {}`        | `when cond {}`  |
| `while cond {}`     | `whilst cond {}` |
| `for x in xs {}`    | `each x in xs {}` |
| `continue`          | `onward`        |
| `include "file"`    | `adopt "file"`  |
| `print(...)`        | `speak(...)`    |
| `len(x)`            | `size(x)`       |
| `range(0, 10)`      | `span(0, 10)`   |
| `xs.push(x)`        | `xs.shove(x)`   |
| `xs.join(",")`     | `xs.weld(",")`  |
| `s.split(",")`     | `s.split(",")`  |
| `s.replace(a, b)`   | `s.reforge(a, b)` |

Everyday utilities stay short and generic (`size`, `num`, `int`, `bool`,
`split`, `clear`); the rest follows the foundry theme — you **forge**
bindings, **craft** functions, **speak** output, **weld** lists together, and
keep going **onward**.

## Features

- **A real compiler pipeline** — lexer → single-pass compiler → stack-machine
  bytecode, the same architecture as Lua and CPython (no tree-walking).
- **Full-featured runtime**: numbers, strings, lists, maps, ranges, `nil`,
  booleans, closures with upvalues, recursion, first-class functions.
- **Control flow**: `when`/`else`, `whilst`, `each … in`, `break`, `onward`,
  compound assignment (`+=`, `**=`, indexed `xs[i] += 1`), ranges
  (`0..10`, `0..=10`), `&&`/`||` short-circuiting.
- **Assignment is an expression**: `forge y = (x = 5)` works, and the REPL
  echoes values the way Python's does.
- **Rich standard library**: `speak`, `hear`, `size`, `nature`, `str`, `num`,
  `int`, `bool`, `abs`, `root`, `least`, `greatest`, `tick`, `span`, `vouch`,
  `flaw`, plus methods on lists, strings, maps, and ranges — `shove`, `yank`,
  `weld`, `reforge`, `order`, `flip`, `clear`, `pluck`, …
- **Corros-native error reporting**: compile errors with file/line/column and
  a caret into your source; runtime errors with full stack tracebacks that
  name every `craft`.
- **REPL**, bytecode disassembly (`--dump`), and `adopt` modules with cycle
  detection.
- **Clean, dependency-free Rust** — one crate, zero external dependencies.

## Install (one line)

```bash
curl -fsSL https://raw.githubusercontent.com/CocoCopi/corros/main/install.sh | sh
```

The installer downloads a prebuilt binary from the latest GitHub release
(Linux, macOS, Windows — x86_64 and ARM64); if none exists for your platform
it falls back to building from source. Both the binary and the Corros-written
standard library (`prelude.cor`) are installed, so `corros` works from
anywhere.

**From source** (requires Rust 1.70+):

```bash
cargo build --release
./target/release/corros            # start the REPL
./target/release/corros file.cor   # run a script
./target/release/corros --dump file.cor  # print compiled bytecode
./target/release/corros --run-bc file.bc # run compiled bytecode (self-hosting)
```

## A taste of Corros

```corros
// Recursion
craft fib(n) {
  when n < 2 { return n }
  return fib(n - 1) + fib(n - 2)
}
speak("fib(15) =", fib(15))            // fib(15) = 610

// Closures capture their surroundings
craft make_counter() {
  forge n = 0
  return craft() { n += 1; return n }
}
forge counter = make_counter()
speak(counter(), counter(), counter()) // 1 2 3

// Lists, maps, ranges
forge xs = [3, 1, 2]
xs.shove(4); xs.order()
speak(xs)                              // [1, 2, 3, 4]

forge ages = { "alice": 30, "bob": 25 }
speak(ages["alice"])                   // 30

each n in 1..=3 { speak(n) }           // 1 2 3
```

## How it works

```
source ──lexer──▶ tokens ──compiler──▶ bytecode ──VM──▶ result
```

## Self-hosting: the full interpreter, written in Corros

Like Rust in Rust, the endgame is the language building itself — and Corros is
there. **The full interpreter — lexer, compiler, and virtual machine — is
written in Corros**, covering every feature: closures with upvalues, maps,
ranges, methods, power, compound and indexed assignment, and `adopt` modules.

```bash
bash selfhost/demo.sh
```

The bootstrap chain, proven end to end:

1. The Rust interpreter compiles `selfhost/compiler.cor` — **a Corros
   compiler written in Corros** — from source.
2. It also compiles `selfhost/vm.cor` — **a Corros virtual machine written
   in Corros** — from source.
3. The **compiled VM runs the compiled compiler**, which compiles a
   full-language program (closures, upvalues, methods, maps).
4. The output is **byte-identical** to the Rust compiler's output, and the
   compiled VM runs the compiled program to completion.
5. The compiled compiler recompiles its own source — **byte-identical**: the
   compiler is a fixed point. Corros compiles Corros, and Corros runs Corros.

To make the deep chain fast, the compiled VM and compiled compiler can run
natively on the host engine (`corros --run-bc file.bc`) — a normal feature,
since they are ordinary Corros programs.

### The standard library is Corros too

`lib/prelude.cor` is the standard library, **written in Corros**. It is
spliced in front of every program, and method calls (`xs.shove(1)`,
`s.split(",")`) route through its `$method` dispatcher — so `shove`, `yank`,
`size`, `holds`, `flip`, `clear`, `weld`, `split`, `opens`, `closes`, and
`reforge` are implemented in the language itself, with a native fallback only
where Corros needs host primitives (case conversion, trimming, map
internals). What's left of Rust is the bootstrap seed — the same role `rustc`
plays for Rust.

| crate module | job |
| ------------ | --- |
| `src/lexer.rs`   | character scanner → tokens (numbers, strings, operators, Corros keywords) |
| `src/compiler.rs`| recursive-descent parser that emits bytecode in one pass: scopes, closures (upvalue capture), control flow, assignment-as-expression |
| `src/chunk.rs`   | bytecode instructions, constant pools, disassembler |
| `src/vm.rs`      | stack-based interpreter: calls, closures, upvalues, builtins, tracebacks |
| `src/value.rs`   | runtime values: numbers, strings, lists, maps, ranges, closures, natives |
| `src/stdlib.rs`  | builtin functions and collection methods (`speak`, `size`, `shove`, `weld`, …) |
| `src/loader.rs`  | file loading + `adopt` token splicing |
| `src/repl.rs`    | the interactive REPL |

## Language reference (the short version)

- **Values**: numbers (`1`, `2.5`, `1e3`), strings (`"hi"`), booleans, `nil`,
  lists `[1, 2]`, maps `{"a": 1}`, ranges `0..5`, functions, closures.
- **Declarations**: `forge x = expr` (top-level names become globals; inside
  functions they are locals), `craft name(params) { … }`, anonymous
  `craft(params) { … }`.
- **Assignment**: `x = v`, `x += v`, `x -= v`, `x *= v`, `x /= v`, `x %= v`,
  `x **= v`, plus indexed `xs[i] = v` and compound indexed `xs[i] += v`.
  Assignment to an undeclared name creates a global.
- **Conditionals**: `when cond { } else { }`, chained `else when`.
- **Loops**: `whilst cond { }`, `each x in iterable { }` with `break`/`onward`.
- **Operators**: `+ - * / % **` arithmetic, `== != < <= > >=` comparison,
  `&& || !` logic, `..` `..=` ranges, indexing `x[i]`, methods `x.method(...)`.
- **Comments**: `// line`, `/* block */`.
- **Builtins**: `speak` (output), `hear` (input), `size` (length),
  `nature` (type name), `str` (to string), `num` (to number), `int`
  (truncate), `bool` (to boolean), `abs`, `root` (sqrt), `least`/`greatest`
  (min/max), `tick` (clock), `span` (range), `vouch` (assert), `flaw` (raise
  an error).
- **Methods**: lists — `shove`, `yank`, `size`, `slot`, `pluck`, `holds`,
  `weld`, `order`, `flip`, `clear`; strings — `size`, `loud`, `quiet`,
  `shave`, `split`, `holds`, `opens`, `closes`, `reforge`; maps — `size`,
  `labels`, `contents`, `holds`, `fetch`, `pluck`, `clear`; ranges — `size`,
  `holds`.
- **Modules**: `adopt "path.cor"` splices another file in (relative paths,
  cycle detection).

## Roadmap

- [x] Lexer, compiler, bytecode VM, REPL
- [x] Closures, collections, ranges, modules, error reporting
- [x] **The full interpreter rewritten in Corros and bootstrapped from source** —
      `selfhost/compiler.cor` + `selfhost/vm.cor` compile and run the entire
      language, byte-identical to the Rust implementation (`bash selfhost/demo.sh`)
- [x] **A standard library written in Corros itself** — `lib/prelude.cor`
      implements the list and string methods in Corros, with native fallbacks
      only where host primitives are required
- [ ] Performance: register-based VM, JIT, or native compilation

## Contributing

Corros is open to contributors — see [CONTRIBUTING.md](CONTRIBUTING.md) for the
workflow, conventions, and the one-time Contributor License Agreement
([CLA.md](CLA.md)) every contributor agrees to. Security issues are handled
privately — see [SECURITY.md](SECURITY.md).

## License

Corros is **dual-licensed**:

- **Community** — MIT, free to use, fork, and build on: [LICENSE](LICENSE)
- **Commercial** — a paid license for private support, indemnification,
  closed-source redistribution, and priority feature work:
  [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)

Contributions are accepted under the [Contributor License Agreement](CLA.md),
which keeps the dual-license model legally sound.
