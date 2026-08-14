#!/usr/bin/env bash
# The self-hosting proof: a COMPLETE Corros interpreter — compiler and virtual
# machine — written in Corros, compiling and running full-language programs.
#
#   selfhost/compiler.cor   the Corros compiler, written in Corros (full language)
#   selfhost/vm.cor         the Corros virtual machine, written in Corros
#
# Usage:  bash selfhost/demo.sh
# Env:    CORROS=/path/to/corros  to point at a specific binary
set -euo pipefail
cd "$(dirname "$0")/.."

# Locate the interpreter: $CORROS, then PATH, then the release build.
if [[ -n "${CORROS:-}" && -x "$CORROS" ]]; then
  corros="$CORROS"
elif command -v corros >/dev/null 2>&1; then
  corros=corros
elif [[ -x ./target/release/corros ]]; then
  corros=./target/release/corros
else
  echo "corros binary not found — build it first (cargo build --release)" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

step "1. The Rust interpreter compiles compiler.cor — a compiler written in Corros — from source"
"$corros" selfhost/compiler.cor selfhost/compiler.cor > "$tmp/self.bc"
echo "   -> compiled the compiler: $(wc -l < "$tmp/self.bc") lines of bytecode"

step "2. ...and compiles vm.cor — a virtual machine written in Corros — from source"
"$corros" selfhost/compiler.cor selfhost/vm.cor > "$tmp/vm.bc"
echo "   -> compiled the VM: $(wc -l < "$tmp/vm.bc") lines of bytecode"

step "3. The compiled compiler, running on the compiled VM, compiles a full-language program"
"$corros" --run-bc "$tmp/vm.bc" "$tmp/self.bc" examples/closures.cor > "$tmp/prog.bc"
"$corros" selfhost/compiler.cor examples/closures.cor > "$tmp/prog-ref.bc"
if diff -q "$tmp/prog-ref.bc" "$tmp/prog.bc" >/dev/null; then
  echo "   -> byte-identical to the Rust compiler's output (closures, upvalues, methods)"
else
  echo "   -> MISMATCH!" >&2
  diff "$tmp/prog-ref.bc" "$tmp/prog.bc" >&2 || true
  exit 1
fi

step "4. The compiled VM runs the compiled program"
"$corros" --run-bc "$tmp/prog.bc"

step "5. The same chain, end to end: compile fib with the bootstrapped compiler, run it"
"$corros" --run-bc "$tmp/vm.bc" "$tmp/self.bc" examples/fib.cor > "$tmp/fib.bc"
"$corros" --run-bc "$tmp/fib.bc"

step "6. The compiler written in Corros is a fixed point: the compiled compiler recompiles itself"
"$corros" --run-bc "$tmp/self.bc" selfhost/compiler.cor > "$tmp/self2.bc"
if diff -q "$tmp/self.bc" "$tmp/self2.bc" >/dev/null; then
  echo "   -> byte-identical: corros compiles corros, and corros runs corros."
else
  echo "   -> MISMATCH!" >&2
  diff "$tmp/self.bc" "$tmp/self2.bc" >&2 || true
  exit 1
fi

echo
echo "Bootstrap complete: the full interpreter (compiler + VM) is written in Corros,"
echo "compiled by itself, and produces byte-identical results to the Rust implementation."
