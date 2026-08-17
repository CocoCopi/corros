---
name: corros
description: Write, compile, run, and debug programs in Corros — a from-scratch scripting language with its own lexer, bytecode compiler, and virtual machine. Use this skill whenever the user asks to write, fix, extend, or run Corros code (.cro files), or to work on the Corros interpreter itself (Rust in src/).
---

# Corros — language reference

Corros is a scripting language built entirely from scratch: its own lexer, its own
bytecode compiler, and its own virtual machine, written in Rust. It has a unique
"foundry" vocabulary no other language shares, assignment-as-expression semantics,
and a self-hosting milestone (a compiler and VM for Corros written in Corros under
`selfhost/`).

- File extension: `.cro` (legacy `.cor` files still run, but always write new files as `.cro` — `.cor` collides with another language and is retired)
- Comments: `// line` and `/* block */`
- Semicolons optional (newlines separate statements, but `;` is allowed)

## Build & run

```bash
# one-line install (prebuilt binary from GitHub releases)
curl -fsSL https://raw.githubusercontent.com/CocoCopi/corros/main/install.sh | sh

# or from source
cd /path/to/corros
cargo build --release                      # plain build works
cargo build --release && cp target/release/corros /usr/local/bin/corros   # make it a command

corros hello.cro                    # run a file
corros hello.cro one two            # pass args (visible as the `args` list)
corros                             # interactive REPL (echoes non-nil results)
corros --dump hello.cro            # print compiled bytecode
corros --run-bc file.bc            # run compiled bytecode natively (self-hosting)
corros --compile fib.cro           # AOT-compile to a native binary (needs `cc`)
corros --reference hello.cro       # run through the Corros-written VM
corros -v                          # version
```

**Benchmarks.** The repo ships a real comparison suite in `bench/`:
identical programs (fib, a 2.7M-iteration loop, a primes sieve) written once
in Corros, C, Rust, Go, and Python. `bash bench/run.sh [rounds] [--md]` builds
every language, verifies each prints the same result, and times them
round-robin (so background load hits everyone equally). Compiled Corros ties
or beats hand-written C and beats Rust, Go, and Python on all three — because
`--compile` emits C and builds with `cc -O3`, C is the ceiling.

Note: the installed `corros` needs the `.cro` sources (`prelude.cro`,
`compiler.cro`, `vm.cro`, `cli.cro`, `codegen.cro`) next to the binary — the
installer places them all in the same directory. Without them, startup fails
with "cannot find src/cli.cro".

Exit codes: 65 = compile error, 70 = runtime error. The REPL wraps input in
`speak((...))`, so expressions echo their value (assignment included).

## The foundry vocabulary (Corros' own syntax)

| other languages        | **Corros**            |
|------------------------|-----------------------|
| `let x = 5`            | `forge x = 5`         |
| `fn f(a) {}`           | `craft f(a) {}`       |
| `if c {} else {}`      | `when c {} else {}`   |
| `else if`              | `else when`           |
| `while c {}`           | `whilst c {}`         |
| `for x in xs {}`       | `each x in xs {}`     |
| `continue`             | `onward`              |
| `break`                | `break`               |
| `include "f"`          | `adopt "f"`           |
| `print`                | `speak`               |
| `len`                  | `size`                |
| `type`                 | `nature`              |

## Types & values

- `nil`, `true`, `false` — literals
- `num` — IEEE-754 double (write `42`, `3.14`); `int(x)` truncates toward zero
- `str` — immutable text; indexing `s[0]` returns a **1-character string**
- `list` — heterogeneous, mutable: `[1, "a", true]`
- `map` — hash map literal: `{ "name": "corros", "v": 1 }`
- `craft` (function) — first-class, captures enclosing locals (closures)
- `range` — produced by `a..b` (exclusive) and `a..=b` (inclusive), mainly for `each`

Truthiness: `nil` and `false` are falsy; empty `num`-zero (`0`), `""`, `[]`, and
`{}` are falsy; everything else is truthy. **No implicit type coercion anywhere**:
`"x" + 1` is a runtime error; use `str(1)`.

## Statements

```
forge x = 5                 // declare a local (block-scoped)
x = 6                       // assignment is an EXPRESSION: `y = (x = 5)` yields 5
x += 1                      // += -= *= /= %= **= (compound; also an expression)
craft add(a, b) { return a + b }
when x > 0 { speak("pos") } else when x < 0 { speak("neg") } else { speak("zero") }
whilst x < 10 { x = x + 1 }
each item in items { speak(item) }
each i in 0..=10 { ... }    // inclusive range; `0..10` excludes 10
break                       // exit the innermost whilst/each
onward                      // skip to the next iteration
return expr                  // from a craft; bare `return` returns nil
adopt "lib.cro"             // include another file's top-level code (modules)
vouch(cond, "message")      // assert; aborts with message on failure
flaw("message")             // raise a runtime error immediately
```

`each item in xs` compiles to a hidden `size(xs)` call evaluated once per
iteration — mutating the list while iterating is undefined behavior, avoid it.

## Expressions & operators

Precedence, highest first:

1. `f(x)`, `xs[i]`, `obj.method(args)` — call / index / method
2. `**` (power, right-assoc), unary `-x`, `!x` (logical not)
3. `*  /  %`
4. `+  -`  (on nums; `+` also concatenates two strings, or merges two lists)
5. `..  ..=` (ranges)
6. `==  !=  <  <=  >  >=`
7. `&&`  (short-circuit)
8. `||`  (short-circuit)
9. `=` `+=` etc. — right-associative, returns the assigned value

Note: `==`/`!=` compare by value for nums, strings, bools, nil; for lists/maps they
compare **identity** (same object). Methods are NOT first-class values — you can't
do `xs.shove` and pass it around; call them directly.

## Builtins (global functions)

| builtin     | meaning                                  |
|-------------|------------------------------------------|
| `speak(...)`| print each arg space-separated, newline  |
| `eprint(...)` | print to **stderr** (never captured output — use it for diagnostics) |
| `hear()` / `hear(prompt)` | read a line from stdin         |
| `size(x)`   | length of string/list/map                |
| `nature(x)` | type name: `"num" "bool" "nil" "list" "map" "range"`; strings report `"string"` (not `"str"`), and `craft` values report `"list"` |
| `str(x)`    | string form                              |
| `num(x)`    | parse string → num (errors on garbage)   |
| `int(x)`    | truncate toward zero                     |
| `bool(x)`   | truthiness as bool                       |
| `abs(x)`    | absolute value                           |
| `root(x)`   | square root                              |
| `least(a,b)` / `greatest(a,b)` | min / max                     |
| `tick()`    | seconds since epoch (float)              |
| `span(a, b)`| range from a to b (see `..`/`..=`)       |
| `vouch(c, msg)` | assert                               |
| `flaw(msg)` | raise runtime error                      |
| `read(path)` | file contents as string (self-hosting)  |
| `readlines(path)` | file contents as list of lines (self-hosting) |
| `shove(list, x)` | append (self-hosting; same as `.shove`) |
| `yank(list)` | pop last (self-hosting; same as `.yank`) |

### Host services (networking / files / processes / FFI)

These builtins let Corros programs talk to the OS — they power the Corros
Ollama server (`/sdcard/Projects/Ollama`). Implemented in the seed
(`seed.rs`), shared by the seed and the native executor. Handles are opaque
numbers (u64 ids); pointers travel as u64 bit patterns (canonical user
pointers are < 2^48, so f64 round-trips them exactly).

**Networking** — `net_listen(port)` → listener handle · `net_accept(listener)`
→ connection handle (blocking) · `net_read(conn, max)` → up to max bytes, `""`
on EOF/timeout (30s default, change with `net_timeout(conn, ms)`) ·
`net_write(conn, s)` → bytes written · `net_close(conn)`.

**HTTP client** — `http_get(url)` → `[status, body]` (follows up to 5
redirects) · `http_download(url, path)` → status (streams raw bytes — binary
GGUF downloads never round-trip through a string).

**Files & processes** — `file_write(path, s)` · `file_append(path, s)` ·
`sys_exec(cmd, [args])` → `[status, stdout]` · `getenv(name)` → value or `""`.

**Dynamic FFI (dlopen)** — `load_lib(path)` → handle (0 on failure — some
filesystems, like Android's sdcard fuse mount, refuse dlopen; copy the lib to
a temp dir and retry) · `lib_call(lib, "fn", [i64 args...])` → i64 result
(transmutes the symbol to an 8×i64-arg C function; SysV x86-64 / AAPCS64
ABIs) · `lib_close(lib)`.

**Memory & C strings** — `mem_alloc(bytes)` → ptr (8-aligned, tracked for
safe free) · `mem_free(ptr)` · `mem_i64(ptr, idx)` → the i64 at ptr+idx*8 ·
`cstr_alloc(s)` → leaked C string (free with `cstr_free`) · `cstr_get(ptr)` →
string (reads to NUL) · `cstr_free(ptr)`. C-malloc'd strings from a shim must
be freed by the shim's own free wrapper (e.g. `ll_free`), not `cstr_free`.

**Conventions for FFI programs** (see `/sdcard/Projects/Ollama`):
- Shared app state should be **globals**, not top-level `forge` locals — a
  `craft` sees globals; top-level locals are not visible inside functions.
- `adopt` resolves relative to the **main file's directory**, and adopting the
  same file twice is a hard error — adopt each file once, in dependency order,
  from the entry point.
- Top-level `return` is illegal — dispatch from a `craft main()`.
- `lib_call` args must be numbers; build argument lists as `[a, b, ...]`.
- The `corros` binary resolves its own `src/cli.cro` from the CWD first — a
  project with its own `src/cli.cro` must be launched from a neutral directory
  with an absolute path (see the `corllama` launcher).

## Methods (receiver.method(...))

**lists** — `shove(x)` append · `yank()` pop · `size()` · `slot(i, x)` insert at i
· `pluck(i)` remove at i, returns it · `holds(x)` contains? · `weld(sep)` join to
string · `order()` sort in place · `flip()` reverse in place · `clear()`

**strings** — `size()` · `loud()` uppercase · `quiet()` lowercase · `shave()` trim
· `split(sep)` → list · `reforge(from, to)` replace all · `holds(sub)` contains? ·
`opens(prefix)` starts-with? · `closes(suffix)` ends-with?

**maps** — `size()` · `labels()` keys as list · `contents()` values as list ·
`fetch(key)` get (nil if absent) · `fetch(key, default)` · `pluck(key)` remove &
return · `holds(key)` · `clear()`

## Script arguments & modules

- `args` is a global list of the script's command-line arguments (empty when none).
- `adopt "path.cro"` splices the file's top-level code into the current one, in the
  same global scope. Used to build libraries. Paths resolve relative to the
  current working directory.

## REPL

`corros` with no file starts a REPL. Multi-line input works (keep typing; it
compiles when the input is complete). Every statement is wrapped as
`speak((...))`, so non-`nil` results echo — assignments echo their value.

## Architecture (for extending the language)

**The interpreter is written in Corros; Rust is only the bootstrap.**

Corros side (`src/*.cro`):

- `compiler.cro` — **the Corros compiler**: lexer + single-pass bytecode
  compiler (no AST). Its own implementation stays in a deliberate subset — no
  closures/maps/methods in ITS source — so self-compilation only exercises
  constructs that were already proven correct.
- `vm.cro` — **the reference VM**, written in Corros, using the full
  language. Run any program through it with `corros --reference file.cro`.
  It is authoritative; it is also reasonably fast, because the compiled VM
  runs on the native executor (cached like the compiler) and its globals are
  an O(1) map — only the meta-circular dispatch tax (~3µs/op) remains, ~5×
  faster than the old tree-walking path and ~80× better than the original
  250µs/op interpreter.
- `prelude.cro` — **the Corros standard library**, spliced in front of every
  program; method calls route through its `$method(recv, name, [args])`
  dispatcher, so list/string methods (`shove`, `yank`, `size`, `holds`,
  `flip`, `clear`, `weld`, `split`, `opens`, `closes`, `reforge`) are
  implemented in Corros. The native side falls back to its method table only
  for host primitives (`mcall(name, recv, args)`).
- `codegen.cro` — **the AOT backend for `--compile`**: parses the textual
  bytecode, runs a whole-program type analysis over stack slots, emits C, and
  pipes it through `cc -O3`. It ends with two peephole passes that make the
  generated C as fast as hand-written C: (1) `peephole` inlines every
  single-use temp into its use site (gcc -O3 gives up on functions full of
  `double t8 = (t6 < t7);` copies), and (2) `branch_swap` turns
  `if (!(cond)) goto L; <then-return> L: <rest>` into
  `if (cond) goto Lnew; L: <rest> Lnew: <then-return>` so the hot path falls
  through without a `!`. Debug progress with `CORROS_CODEGEN_DEBUG=1`
  (messages go to stderr via the `eprint` builtin, never into the emitted
  C).

Rust side (`src/*.rs`):

- `seed.rs` — the bootstrap seed: a tree-walking interpreter that runs
  `compiler.cro`. Values, builtins, operators, and the native method table
  live here (`Value`, `binary_op`, `index_get`, `native_builtin`,
  `lookup_method`). The CLI bridge builtins live here too: `run(path, args)`,
  `run_bc`, `run_ref`, `dump`, `run_src_try(src)` (never fails — returns
  `[true, line...]` or `[false, error]`), and `version()`.
- `native.rs` — the native executor: parses the textual bytecode
  `compiler.cro` emits (`FUNCTION`/`ENDFN` blocks, one instruction per line)
  and runs it at native speed — `fib(30)` in ~1s. This is the default
  execution path; the compiled compiler itself is cached (in the OS temp
  dir, keyed on `compiler.cro` + `prelude.cro`) so startup skips the seed.
- `codegen.cro` — **the AOT compiler backend, written in Corros**: a
  whole-program type analysis over the bytecode (numbers, strings, booleans,
  ranges, functions, builtins) plus a stack-accurate C emitter, built with
  `cc -O3`. Like the compiler and the VM, the compiler-backend is Corros —
  the seed only runs its cached compiled bytecode. It rejects dynamic
  programs (lists, maps, methods, closures with upvalues) with clear
  messages. `fib(30)` compiles to a binary that runs in ~0.025s — faster
  than Go and Rust-f64, at hand-written-C parity.
- `lexer.rs` — the seed's tokenizer (reads the Corros sources).
- `cli.cro` — **the CLI, written in Corros**: flags, `--dump`, `--run-bc`,
  `--reference`, and the REPL (which uses `run_src_try` so an error in one
  line doesn't kill the session). Keep it method-free-ish (it uses
  `line.shave()`, which the spliced prelude provides).
- `main.rs` — a thin launcher: compiles `cli.cro` with the cached compiled
  compiler and runs it on the native executor with the user's arguments.
- `tests/language.rs` — integration tests: `run("...")` executes source and
  returns captured output lines.

**Self-hosting** (`demo.sh`): the seed boots `compiler.cro`, which compiles
itself and `vm.cro`; the compiled VM then runs the compiled compiler, and the
output is byte-identical to the source compiler's — Corros compiles Corros,
and Corros runs Corros.

**AOT compilation** (`corros --compile`): the pipeline is `compiler.cro`
(bytecode) → `codegen.cro` (types + C) → `cc -O3` (native binary). It compiles
*without* the prelude so the static analysis stays tractable; programs that
need methods/lists/maps run interpreted instead. The default output name is
the source minus `.cro` (`fib.cro` → `./fib`), or pass an explicit path.

## Gotchas (important — these bite)

1. **No type coercion.** `"a" + 1` fails; write `"a" + str(1)`. Comparisons only
   work on two nums or two strings.
2. **Assignment is an expression** — `forge y = (x = 5)` works; `Set*` opcodes keep
   the value on the stack. This also means `xs[0] = 99` evaluates to `99`.
3. **`break`/`onward` inside a loop pop the loop's block locals** (so values don't
   leak onto the stack). Don't rely on loop variables after the loop.
4. **Methods aren't values** — no `map(xs, xs.shove)`.
5. **String indexing gives 1-char strings**, so `s[0] == "a"` (not `'a'`; there are
   no char literals).
6. **Maps/lists compare by identity** with `==`, not by contents.
7. **`each` uses `size(xs)` per iteration** — don't mutate the collection mid-loop.
8. Numbers are doubles: `10 / 4` is `2.5`, and `str(2.0)` prints as `2`.
9. Function parameters and `forge` locals are separate from globals: assigning to
   an undeclared name creates/updates a **global**, while `forge` declares a local.
   Inside a craft, use `forge` for every temporary — plain assignment to a new
   name silently becomes a global (and can clobber same-named globals used by
   callees in the self-hosting code).

## Example programs

```corros
// hello.cro
speak("Hello, world!")

// fib.cro — recursion + closures
craft fib(n) {
  when n <= 1 { return n }
  return fib(n - 1) + fib(n - 2)
}
speak(fib(10))          // 55

// fizzbuzz.cro — each + ranges + when
each i in 1..=15 {
  when i % 15 == 0 { speak("fizzbuzz") }
  else when i % 3 == 0 { speak("fizz") }
  else when i % 5 == 0 { speak("buzz") }
  else { speak(i) }
}

// closures.cro — the foundry theme
craft make_counter() {
  forge count = 0
  return craft() { count = count + 1; return count }
}
forge next = make_counter()
speak(next(), next(), next())   // 1 2 3

// collections.cro
forge metals = ["iron", "copper", "gold"]
metals.shove("steel")
metals.order()
speak(metals.weld(" ~ "))               // copper ~ gold ~ iron ~ steel
speak("corrosion".reforge("corr", "c")) // cosion
speak("a,b,c".split(","))               // [a, b, c]
speak(size("hello"), size([1,2,3]))     // 5 3
vouch(1 + 1 == 2, "math is broken")
```

## Contributing to the interpreter

- Run tests: `cargo test` (integration tests live in `tests/language.rs`).
- Run clippy: `cargo clippy --all-targets`.
- On Android/sdcard mounts, target dirs may need `CARGO_TARGET_DIR` or
  `CARGO_INCREMENTAL=0`; on a normal machine plain `cargo build` works.
- Follow the existing conventions: single-pass compiler, no new AST, value-based
  `Value` enum, foundry-themed names for anything user-facing.
