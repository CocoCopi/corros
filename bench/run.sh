#!/usr/bin/env bash
# The Corros benchmark suite.
#
# Compiles and runs IDENTICAL programs (fib, counter-loop, primes) written in
# Corros, C, Rust, Go, and Python, verifies every language produces the same
# result, then times them round-robin (best of N).
#
#   ./run.sh [iterations]      times N rounds per benchmark (default: 5)
#   ./run.sh [iterations] --md prints a markdown table instead
#
# The `corros --compile` column is the fast lane (ahead-of-time compilation to
# native code through cc -O3). The interpreted column is `corros file.cor`.
#
# Timing is round-robin (one run of every language per round) so that any
# background-load drift on the machine hits all languages equally.
#
# Binaries are built in a temp dir: some filesystems (e.g. Android sdcard) are
# mounted noexec and cannot run compiled programs.

set -u
cd "$(dirname "$0")"
BENCH_DIR="$(pwd)"

N="${1:-5}"
MD=0
[ "${2:-}" = "--md" ] && MD=1

# --- locate tools -----------------------------------------------------------
CORROS="${CORROS:-$(command -v corros 2>/dev/null || echo /usr/local/bin/corros)}"
CC="${CC:-$(command -v cc 2>/dev/null || command -v gcc 2>/dev/null || echo '')}"
RUSTC="$(command -v rustc 2>/dev/null || echo '')"
GO="$(command -v go 2>/dev/null || echo '')"
PYTHON="$(command -v python3 2>/dev/null || echo '')"

have() { [ -n "$1" ]; }

if ! have "$CORROS" && [ ! -x "$CORROS" ]; then
    echo "corros binary not found (set CORROS=... or install it)" >&2
    exit 1
fi
if ! have "$CC"; then echo "no C compiler (cc/gcc) found" >&2; exit 1; fi
if ! have "$RUSTC"; then echo "warning: rustc not found" >&2; fi
if ! have "$GO"; then echo "warning: go not found" >&2; fi
if ! have "$PYTHON"; then echo "warning: python3 not found" >&2; fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/corros-bench.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 1

BENCHES=(fib loop primes)
EXPECTED=(832040 3644998650000 "9592454396537")
LANGS=(corros c rust go python)

# --- build all ---------------------------------------------------------------
build() { # bench lang -> 0 on success
    local bench="$1" lang="$2" out="out_${bench}_${lang}"
    case "$lang" in
        corros) "$CORROS" --compile "$BENCH_DIR/$bench.cor" "$WORK/$out" >/dev/null 2>&1 ;;
        c)      "$CC" -O3 -o "$WORK/$out" "$BENCH_DIR/$bench.c" -lm >/dev/null 2>&1 ;;
        rust)   "$RUSTC" -O -o "$WORK/$out" "$BENCH_DIR/$bench.rs" >/dev/null 2>&1 ;;
        go)     "$GO" build -o "$WORK/$out" "$BENCH_DIR/$bench.go" >/dev/null 2>&1 ;;
    esac
}

runcmd() { # bench lang -> eval-able run command
    local bench="$1" lang="$2" out="out_${bench}_${lang}"
    case "$lang" in
        corros) echo "./$out" ;;
        c)      echo "./$out" ;;
        rust)   echo "./$out" ;;
        go)     echo "./$out" ;;
        python) echo "$PYTHON $BENCH_DIR/$bench.py" ;;
    esac
}

ok_lang() { # lang -> 0 if its toolchain exists
    case "$1" in
        rust)   have "$RUSTC" ;;
        go)     have "$GO" ;;
        python) have "$PYTHON" ;;
        *)      return 0 ;;
    esac
}

echo "building..."
for b in "${!BENCHES[@]}"; do
    bench="${BENCHES[$b]}"
    for lang in "${LANGS[@]}"; do
        ok_lang "$lang" || continue
        if build "$bench" "$lang"; then
            echo "  ok: $lang/$bench"
        else
            echo "  BUILD FAILED: $lang/$bench" >&2
        fi
    done
done

# --- verify all --------------------------------------------------------------
echo "verifying..."
for b in "${!BENCHES[@]}"; do
    bench="${BENCHES[$b]}"
    expected="${EXPECTED[$b]}"
    for lang in "${LANGS[@]}"; do
        ok_lang "$lang" || continue
        got=$(eval "$(runcmd "$bench" "$lang")" 2>/dev/null | tr -d '[:space:]')
        if [ "$got" != "$expected" ]; then
            echo "  WRONG OUTPUT ($lang/$bench): got '$got', expected '$expected'" >&2
            exit 1
        fi
    done
done
echo "  all outputs match ($expected for each benchmark)"

# --- machine info -----------------------------------------------------------
echo
echo "machine: $(uname -s -m)  |  date: $(date -u +%Y-%m-%d)  |  rounds: $N (best of $N)"
echo "corros:  $("$CORROS" -v 2>/dev/null || echo '?')"
echo "cc:      $("$CC" --version 2>/dev/null | head -1 || echo '?')"
[ -n "$RUSTC" ] && echo "rustc:   $("$RUSTC" --version 2>/dev/null)"
[ -n "$GO" ] && echo "go:      $("$GO" version 2>/dev/null)"
[ -n "$PYTHON" ] && echo "python:  $("$PYTHON" --version 2>&1)"
echo

# --- time round-robin ---------------------------------------------------------
# interp needs its own command (no binary)
interp_cmd() { echo "$CORROS $BENCH_DIR/$1.cor"; }

declare -A BEST
for b in "${!BENCHES[@]}"; do
    bench="${BENCHES[$b]}"
    for lang in "${LANGS[@]}"; do
        ok_lang "$lang" || continue
        BEST["$bench/$lang"]=""
    done
    BEST["$bench/interp"]=""
done

for ((r = 0; r < N; r++)); do
    for b in "${!BENCHES[@]}"; do
        bench="${BENCHES[$b]}"
        for lang in "${LANGS[@]}"; do
            ok_lang "$lang" || continue
            s=$(date +%s.%N)
            eval "$(runcmd "$bench" "$lang")" >/dev/null 2>&1
            e=$(date +%s.%N)
            d=$(awk -v a="$s" -v b="$e" 'BEGIN { printf "%.4f", b - a }')
            if [ -z "${BEST["$bench/$lang"]}" ] || awk -v a="$d" -v b="${BEST["$bench/$lang"]}" 'BEGIN { exit !(a < b) }'; then
                BEST["$bench/$lang"]="$d"
            fi
        done
        s=$(date +%s.%N)
        eval "$(interp_cmd "$bench")" >/dev/null 2>&1
        e=$(date +%s.%N)
        d=$(awk -v a="$s" -v b="$e" 'BEGIN { printf "%.4f", b - a }')
        if [ -z "${BEST["$bench/interp"]}" ] || awk -v a="$d" -v b="${BEST["$bench/interp"]}" 'BEGIN { exit !(a < b) }'; then
            BEST["$bench/interp"]="$d"
        fi
    done
done

# --- print table ------------------------------------------------------------
if [ "$MD" = "1" ]; then
    echo "| benchmark | Corros \`--compile\` | Corros (interp) | C | Rust | Go | Python |"
    echo "|---|---|---|---|---|---|---|"
fi

printf '%-16s %-17s %-15s %-8s %-8s %-8s %-8s\n' \
    "benchmark" "corros--compile" "corros-interp" "c" "rust" "go" "python"
for bench in "${BENCHES[@]}"; do
    printf '%-16s %-17s %-15s %-8s %-8s %-8s %-8s\n' \
        "$bench" \
        "${BEST["$bench/corros"]:-   -}" \
        "${BEST["$bench/interp"]:-   -}" \
        "${BEST["$bench/c"]:-   -}" \
        "${BEST["$bench/rust"]:-   -}" \
        "${BEST["$bench/go"]:-   -}" \
        "${BEST["$bench/python"]:-   -}"
done

exit 0
