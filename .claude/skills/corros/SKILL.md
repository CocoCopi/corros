---
name: corros
description: Write, compile, run, and debug programs in Corros — a from-scratch scripting language with its own lexer, bytecode compiler, and virtual machine. Use this skill whenever the user asks to write, fix, extend, or run Corros code (.cor files), or to work on the Corros interpreter itself (Rust in src/).
---

# Corros — language reference

Corros is a scripting language built entirely from scratch: its own lexer, its own
bytecode compiler, and its own virtual machine, written in Rust. It has a unique
"foundry" vocabulary no other language shares, assignment-as-expression semantics,
and a self-hosting milestone (a compiler and VM for Corros written in Corros under
`selfhost/`).

- File extension: `.cor`
- Comments: `// line` and `/* block */`
- Semicolons optional (newlines separate statements, but `;` is allowed)

## Build & run

```bash
cd /path/to/corros
cargo build --release                      # plain build works
cargo build --release && cp target/release/corros /usr/local/bin/corros   # make it a command

corros hello.cor                    # run a file
corros hello.cor one two            # pass args (visible as the `args` list)
corros                             # interactive REPL (echoes non-nil results)
corros --dump hello.cor            # print compiled bytecode
corros -v                          # version
```

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
adopt "lib.cor"             // include another file's top-level code (modules)
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
| `hear()` / `hear(prompt)` | read a line from stdin         |
| `size(x)`   | length of string/list/map                |
| `nature(x)` | type name: `"num" "str" "bool" "nil" "list" "map" "craft" "range"` |
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
- `adopt "path.cor"` splices the file's top-level code into the current one, in the
  same global scope. Used to build libraries. Paths resolve relative to the
  current working directory.

## REPL

`corros` with no file starts a REPL. Multi-line input works (keep typing; it
compiles when the input is complete). Every statement is wrapped as
`speak((...))`, so non-`nil` results echo — assignments echo their value.

## Architecture (for extending the language)

Rust crate in `src/`:

- `lexer.rs` — tokens; keywords live in `identifier()` (line ~424)
- `compiler.rs` — single-pass recursive-descent compiler → bytecode (no AST);
  `mute` mode scans lvalues without emitting; `Rotate3` opcode handles
  `xs[i] += 1`; assignment keeps its value on the stack (opcodes `SetLocal`,
  `SetGlobal`, `SetIndex` all leave the value)
- `chunk.rs` — `OpCode` enum, disassembler
- `vm.rs` — stack VM; frames, closures (upvalue capture), traceback rendering
- `stdlib.rs` — builtins (`BUILTINS` table) and the method table (`method_defs`:
  `"shove" => list_shove` etc.)
- `value.rs` — `Value` enum; `type_name()` feeds `nature()`
- `repl.rs`, `main.rs` — REPL wrapper & CLI (`--dump`, `-v`)
- `error.rs` — source-aware compile errors + runtime tracebacks
- `tests/language.rs` — integration tests: `run("...")` executes source and
  returns captured output lines

**Self-hosting** (`selfhost/`): `compiler.cor` is a Corros compiler written in
Corros and `vm.cor` is a Corros VM written in Corros — together they cover the
FULL language (closures with upvalues, maps, ranges, methods, `adopt`, power,
compound assignment). `selfhost/demo.sh` runs the bootstrap proof: the compiled
VM runs the compiled compiler, output is byte-identical to the Rust
implementation, and the compiler is a fixed point under its own compilation.
`corros --run-bc file.bc` runs compiled bytecode natively on the host engine.

(Note: `compiler.cor`'s own implementation stays in a deliberate subset — no
closures/maps/methods in ITS source — so self-compilation only exercises
constructs that were already proven correct. `vm.cor` uses the full language.)

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
// hello.cor
speak("Hello, world!")

// fib.cor — recursion + closures
craft fib(n) {
  when n <= 1 { return n }
  return fib(n - 1) + fib(n - 2)
}
speak(fib(10))          // 55

// fizzbuzz.cor — each + ranges + when
each i in 1..=15 {
  when i % 15 == 0 { speak("fizzbuzz") }
  else when i % 3 == 0 { speak("fizz") }
  else when i % 5 == 0 { speak("buzz") }
  else { speak(i) }
}

// closures.cor — the foundry theme
craft make_counter() {
  forge count = 0
  return craft() { count = count + 1; return count }
}
forge next = make_counter()
speak(next(), next(), next())   // 1 2 3

// collections.cor
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
