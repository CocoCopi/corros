# I Wrote a Programming Language From Scratch — and Then Wrote the Compiler in It

**Corros** is a programming language I built from nothing: its own lexer, its
own bytecode compiler, its own virtual machine. No LLVM, no parser generator,
no borrowed syntax.

The twist: Rust is written in Rust. **Corros is written in Rust — and it
already compiles itself.** The compiler, the VM, and the standard library live
in the repository as `.cor` files, and a byte-identical bootstrap chain proves
they are the real thing: `corros` compiles `compiler.cor`, the compiled
compiler compiles itself, and the output is byte-for-byte identical.

## Syntax that belongs to no other language

No `let`, no `fn`, no `print`. Corros speaks the language of the forge:

```corros
forge greet = craft(name) {
  return "hello, " + name
}

each i in 1..=5 {
  when i % 2 == 0 { onward }
  speak(greet("corros"), i)
}
```

`forge` declares, `craft` defines a function, `when`/`whilst`/`each` control
flow, `speak` prints, `flaw` raises an error, `adopt` imports a module.

## Faster than Go — by compiling

An interpreter is never fast enough, so Corros grew a real ahead-of-time
compiler: `corros --compile fib.cor` runs a whole-program type analysis over
the bytecode, emits C, and builds a native binary with `cc -O3`. On the repo's
benchmark suite — identical programs in each language — compiled Corros ties
hand-written C and beats Rust, Go, and Python:

| fib(30) | time |
|---|---|
| Corros `--compile` | ~0.03s |
| hand-written C | parity |
| Rust (`-O`) | ~0.04s |
| Go | ~0.06s |

## It can even talk to the OS

Corros ships host services — sockets, an HTTP client, file and process
access, and a dynamic FFI (`load_lib` / `lib_call`). That was enough to write
**corllama**, a local LLM server with a streaming REST API, entirely in
Corros.

**Try it:**
```bash
curl -fsSL https://raw.githubusercontent.com/CocoCopi/corros/main/install.sh | sh
corros hello.cor
```

⭐ The repository: https://github.com/CocoCopi/corros
