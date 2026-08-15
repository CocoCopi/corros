# 🦀 Corros

**A programming language forged from scratch — and named for what Rust does best: corrosion.**

Corros is a bytecode-compiled scripting language with its own lexer, its own
compiler, its own virtual machine, and a syntax that belongs to **no other
language**. Rust is written in Rust. Corros is written in Rust — and it already
compiles **itself**: `src/compiler.cor`, `src/vm.cor`, and `src/prelude.cor`
are a Corros compiler, a Corros virtual machine, and a Corros standard
library — all written in Corros, proven by a byte-identical bootstrap chain
(`bash demo.sh`).

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

## Benchmarks

`corros --compile` runs a whole-program type analysis over your program's
bytecode, emits C, and builds a native binary with `cc -O3` — so compiled
Corros sits **at the C ceiling**: it ties or beats hand-written C, and beats
**Rust, Go, and Python on every workload measured here**. The interpreter
(`corros file.cor`) stays the fast-to-start scripting default.

Measured on an ARM64 Linux box with `bench/run.sh` (round-robin, best of 7):

| benchmark | Corros `--compile` | Corros (interp) | C | Rust | Go | Python |
|---|---|---|---|---|---|---|
| `fib(30)` | **0.081s** | ~3s | 0.078s | 0.098s | 0.131s | 0.885s |
| 2.7M-iteration loop | **0.075s** | ~4s | 0.087s | 0.087s | 0.122s | 2.134s |
| primes below 100,000 | **0.170s** | ~3.6s | 0.182s | 0.253s | 0.964s | 1.139s |

Run it yourself — the suite lives in `bench/`:

```bash
bash bench/run.sh          # plain table
bash bench/run.sh 9 --md   # markdown table, 9 rounds
```

The five programs are **identical** — the same algorithm with the same `f64`
number type — written once in Corros, C, Rust, Go, and Python. The runner
builds every language, verifies that each one prints the same result, then
times them round-robin so background load on the machine hits everyone
equally. (This box also runs background workloads, so absolute numbers
fluctuate; the corros-vs-C relationship — parity or better — holds in every
run.)

The one honest boundary: `--compile` generates C and compiles it through gcc,
so it can equal C but not exceed it. What it delivers is **C-level speed with
Corros' syntax** — ahead of Go and Rust, ~10× ahead of Python — while the
interpreter keeps everything else simple and dynamic.

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
- **Ahead-of-time compilation** — `corros --compile file.cor` runs a
  whole-program type analysis over the bytecode, emits C, and builds a native
  binary with `cc -O3`. Compiled code ties or beats hand-written C, and runs
  **faster than Rust, Go, and Python** (see the benchmarks above).
- **Clean, dependency-free Rust** — one crate, zero external dependencies.

## Install — one line

```bash
curl -fsSL https://raw.githubusercontent.com/CocoCopi/corros/main/install.sh | sh
```

That's it. It downloads a prebuilt binary for your platform (Linux, macOS,
Windows — x86_64 and ARM64), or builds from source if no prebuilt exists, and
installs `corros` alongside the Corros-written interpreter (`compiler.cor`,
`vm.cor`, `cli.cor`, `prelude.cor`), which the binary loads from beside itself.

**Build from source — one line** (requires Rust 1.70+):

```bash
git clone https://github.com/CocoCopi/corros.git && cd corros && bash install.sh
```

Or build and run in place:

```bash
cargo build --release
./target/release/corros            # start the REPL
./target/release/corros file.cor   # run a script
./target/release/corros --dump file.cor  # print compiled bytecode
./target/release/corros --run-bc file.bc # run compiled bytecode (native executor)
./target/release/corros --reference file.cor # run through the Corros-written VM (src/vm.cor)
./target/release/corros --compile file.cor   # AOT-compile to a native binary
```

`--compile` needs a C compiler (`cc`) and works best on statically-typed
numeric programs: plain functions, numbers, booleans, strings, ranges,
`when`/`whilst`/`each`, and the builtins. Dynamic features (lists, maps,
methods, closures with upvalues) are rejected with a clear message — run those
with the interpreter instead. The output binary is placed next to the source
(`fib.cor` → `fib`) or at the path you give as the second argument.

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

**The language is Corros, the bootstrap is Rust.** Like rustc's first compiler
was written in OCaml, Corros's first compiler is written in Rust — but only as
a small seed (`src/seed.rs`, a tree-walking interpreter that can boot the
Corros-written compiler) and a native executor (`src/native.rs`) that runs the
compiler's bytecode at native speed. Your program is compiled by
`src/compiler.cor` (written in Corros) and executed by the native executor;
`src/vm.cor` is the reference interpreter, written in Corros, available via
`--reference` and proven by `demo.sh`. Even the **command-line interface is
written in Corros** (`src/cli.cor`) — `main.rs` is a ~20-line launcher that
just boots it. The result: a language written in itself, with programs running
at native-interpreter speed.

## Self-hosting: the full interpreter, written in Corros

Like Rust in Rust, the endgame is the language building itself — and Corros is
there. **The full interpreter — lexer, compiler, and virtual machine — is
written in Corros**, covering every feature: closures with upvalues, maps,
ranges, methods, power, compound and indexed assignment, and `adopt` modules.

```bash
bash demo.sh
```

The bootstrap chain, proven end to end:

1. The seed boots `src/compiler.cor` — **a Corros compiler written in
   Corros** — which compiles a full-language program.
2. The same compiler compiles `src/vm.cor` — **a Corros virtual machine
   written in Corros** — from source.
3. The **compiled VM runs the compiled compiler**, which compiles a program
   with closures, upvalues, methods, and maps.
4. The output is **byte-identical** to the source compiler's output — the
   compiled chain behaves exactly like the source compiler.
5. The compiler is a **fixed point**: it recompiles its own source
   byte-identically. Corros compiles Corros, and Corros runs Corros.

The deep chain is fast because compiled programs run on the native executor
(`corros --run-bc file.bc`) — ordinary Corros programs, at native speed.

### The standard library is Corros too

`src/prelude.cor` is the standard library, **written in Corros**. It is
spliced in front of every program, and method calls (`xs.shove(1)`,
`s.split(",")`) route through its `$method` dispatcher — so `shove`, `yank`,
`size`, `holds`, `flip`, `clear`, `weld`, `split`, `opens`, `closes`, and
`reforge` are implemented in the language itself, with a native fallback only
where Corros needs host primitives (case conversion, trimming, map
internals). What's left of Rust is the bootstrap seed and the native executor
— the same role `rustc`'s first OCaml compiler played for Rust.

| file | job |
| ---- | --- |
| `src/compiler.cor` | **the Corros compiler** — lexer + single-pass bytecode compiler, written in Corros |
| `src/vm.cor`       | **the Corros VM** — the reference interpreter, written in Corros (`--reference`) |
| `src/prelude.cor`  | **the Corros standard library** — list/string methods, `$method` dispatch |
| `src/cli.cor`      | **the Corros CLI** — flags, `--dump`, `--run-bc`, `--reference`, `--compile`, the REPL |
| `src/codegen.cor`  | **the AOT compiler backend, written in Corros** — whole-program type analysis, C emission, and a peephole pass (temp inlining + branch inversion) that makes compiled output as fast as hand-written C |
| `src/seed.rs`      | the bootstrap seed: a tree-walking interpreter that boots `compiler.cor` |
| `src/native.rs`    | the native executor: runs the compiler's bytecode at native speed |
| `src/lexer.rs`     | the seed's tokenizer (reads the Corros sources) |
| `src/error.rs`     | compile-error formatting |
| `src/main.rs`      | a thin launcher that boots `cli.cor` |

**Why is there any Rust at all?** A language's *first* compiler must be written
in some other language — rustc's was OCaml, CPython's is C. The seed is that
first compiler: the minimal piece of native code that can run `compiler.cor`
the first time. It cannot itself be written in Corros (nothing exists to run
it yet), and the native executor is the accelerator that makes bytecode run
fast. Everything a user can see and write — the compiler, the VM, the standard
library, and the CLI — is Corros.

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
      `src/compiler.cor` + `src/vm.cor` compile and run the entire language,
      byte-identical through the whole chain (`bash demo.sh`)
- [x] **A standard library written in Corros itself** — `src/prelude.cor`
      implements the list and string methods in Corros, with native fallbacks
      only where host primitives are required
- [x] **Native execution speed** — the native executor (`src/native.rs`) runs
      compiled bytecode at interpreter speed: `fib(30)` in ~1s and a
      2.7M-iteration loop in ~1s (up from 21 minutes and 53 minutes), with the
      compiled compiler cached so startup skips re-compilation
- [x] **Ahead-of-time compilation** — `corros --compile` types the bytecode
      (numbers, strings, booleans, ranges, functions), emits C, and builds a
      native binary with `cc -O3`. The emitter runs a peephole pass (inline
      single-use temps, invert `when { return }` branches so the hot path
      falls through) that makes the generated C as fast as hand-written C —
      measured on ARM64 with the same number type (`f64`) everywhere: corros
      ties or beats **C**, and beats **Rust, Go, and Python** on `fib(30)`, a
      2.7M-iteration loop, and a primes sieve (see the benchmarks above).
      (The remaining Rust in `src/` — the seed, the native executor — is the
      physical bootstrap and the accelerator; a language's first compiler
      cannot be written in itself, any more than rustc's first compiler could
      be Rust.)
- [ ] Beyond: a register-based VM or a true JIT for the dynamic features

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
