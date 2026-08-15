#!/usr/bin/env bash
# demo.sh — the Corros self-hosting proof.
#
# The interpreter is written in Corros itself (src/compiler.cor, src/vm.cor,
# src/prelude.cor). The `corros` binary is only the bootstrap seed: a small
# tree-walking interpreter that boots the Corros-written compiler, plus a
# native executor (src/native.rs) that runs the compiler's bytecode at native
# speed. The Corros-written VM (src/vm.cor) remains the reference interpreter
# and is exercised here with --reference.
#
#   cargo build --release   (first)
set -euo pipefail
cd "$(dirname "$0")"

CORROS="${CORROS:-target/release/corros}"
if [[ ! -x "$CORROS" ]]; then
  if command -v corros >/dev/null 2>&1; then
    CORROS="$(command -v corros)"
  else
    echo "building corros (release)..."
    cargo build --release
  fi
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "== 1. The Corros compiler (src/compiler.cor) compiles a program"
"$CORROS" --dump examples/closures.cor > "$TMP/closures.bc"
grep -q "CLOSURE" "$TMP/closures.bc" && echo "   -> compiled closures.cor to bytecode (CLOSURE instructions present)"

echo
echo "== 2. The Corros VM (src/vm.cor) executes that bytecode (--reference)"
"$CORROS" --reference examples/closures.cor

echo
echo "== 3. The compiler written in Corros compiles itself"
time "$CORROS" --dump src/compiler.cor > "$TMP/self1.bc"
echo "   -> $(wc -l < "$TMP/self1.bc") lines of bytecode for its own 1000-line source"

echo
echo "== 4. The full bootstrap chain: the compiled VM runs the compiled compiler"
"$CORROS" --dump src/vm.cor > "$TMP/vm.bc"
"$CORROS" --dump examples/hello.cor > "$TMP/hello1.bc"
time "$CORROS" --run-bc "$TMP/vm.bc" "$TMP/self1.bc" examples/hello.cor > "$TMP/hello2.bc"
if cmp -s "$TMP/hello1.bc" "$TMP/hello2.bc"; then
  echo "   -> byte-identical: the compiled VM running the compiled compiler"
  echo "      behaves exactly like the source compiler."
else
  echo "   -> MISMATCH — the compiled chain differs from the source compiler!"
  diff "$TMP/hello1.bc" "$TMP/hello2.bc" | head -20
  exit 1
fi

echo
echo "== 5. And it is fast"
cat > "$TMP/bench.cor" <<'EOF'
forge n = 0
whilst n < 2000000 { n = n + 1 }
speak("2,000,000 iterations ->", n)
EOF
time "$CORROS" "$TMP/bench.cor"

echo
echo "Bootstrap complete: the full interpreter (compiler + VM) is written in Corros,
compiled by itself, and the seed only boots it."
