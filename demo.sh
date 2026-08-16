#!/usr/bin/env bash
# demo.sh — the Corros self-hosting proof.
#
# The interpreter is written in Corros itself (src/compiler.cro, src/vm.cro,
# src/prelude.cro). The `corros` binary is only the bootstrap seed: a small
# tree-walking interpreter that boots the Corros-written compiler, plus a
# native executor (src/native.rs) that runs the compiler's bytecode at native
# speed. The Corros-written VM (src/vm.cro) remains the reference interpreter
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

echo "== 1. The Corros compiler (src/compiler.cro) compiles a program"
"$CORROS" --dump examples/closures.cro > "$TMP/closures.bc"
grep -q "CLOSURE" "$TMP/closures.bc" && echo "   -> compiled closures.cro to bytecode (CLOSURE instructions present)"

echo
echo "== 2. The Corros VM (src/vm.cro) executes that bytecode (--reference)"
"$CORROS" --reference examples/closures.cro

echo
echo "== 3. The compiler written in Corros compiles itself"
time "$CORROS" --dump src/compiler.cro > "$TMP/self1.bc"
echo "   -> $(wc -l < "$TMP/self1.bc") lines of bytecode for its own 1000-line source"

echo
echo "== 4. The full bootstrap chain: the compiled VM runs the compiled compiler"
"$CORROS" --dump src/vm.cro > "$TMP/vm.bc"
"$CORROS" --dump examples/hello.cro > "$TMP/hello1.bc"
time "$CORROS" --run-bc "$TMP/vm.bc" "$TMP/self1.bc" examples/hello.cro > "$TMP/hello2.bc"
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
cat > "$TMP/bench.cro" <<'EOF'
forge n = 0
whilst n < 2000000 { n = n + 1 }
speak("2,000,000 iterations ->", n)
EOF
time "$CORROS" "$TMP/bench.cro"

echo
echo "Bootstrap complete: the full interpreter (compiler + VM) is written in Corros,
compiled by itself, and the seed only boots it."
