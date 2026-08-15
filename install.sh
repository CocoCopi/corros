#!/usr/bin/env bash
# Corros installer — one line:
#   curl -fsSL https://raw.githubusercontent.com/CocoCopi/corros/main/install.sh | sh
#
# Downloads a prebuilt binary from the latest GitHub release (fast), falling
# back to building from source when none exists for your platform. Installs
# `corros` and the Corros-written interpreter (compiler.cor, vm.cor, cli.cor,
# prelude.cor) side by side, which the binary looks up next to itself.
set -euo pipefail

# --- platform detection ---------------------------------------------------
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$os" in
  linux) os=linux ;;
  darwin) os=macos ;;
  mingw* | msys* | cygwin*) os=windows ;;
  *) echo "corros: unsupported OS '$os'" >&2; exit 1 ;;
esac
case "$arch" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "corros: unsupported architecture '$arch'" >&2; exit 1 ;;
esac

# --- install prefix -------------------------------------------------------
if [[ -n "${PREFIX:-}" ]]; then
  prefix="$PREFIX"
elif [[ "$(id -u)" == "0" ]] && [[ -w /usr/local/bin ]]; then
  prefix=/usr/local
else
  prefix="$HOME/.local"
fi
bindir="$prefix/bin"
mkdir -p "$bindir"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- try a prebuilt binary first ------------------------------------------
asset="corros-${os}-${arch}"
url="https://github.com/CocoCopi/corros/releases/latest/download/${asset}.tar.gz"
if curl -fsSL --max-time 120 "$url" -o "$tmp/pkg.tar.gz" 2>/dev/null; then
  tar -xzf "$tmp/pkg.tar.gz" -C "$tmp"
  if [[ -f "$tmp/corros.exe" ]]; then
    bin="$tmp/corros.exe"
  else
    bin="$tmp/corros"
  fi
  src_dir="$tmp"
  echo "Downloaded prebuilt corros (${os}/${arch})."
else
  echo "No prebuilt binary for ${os}/${arch} — building from source (needs Rust)."
  if ! command -v cargo >/dev/null 2>&1; then
    echo "corros: 'cargo' not found. Install Rust first: https://rustup.rs" >&2
    exit 1
  fi
  repo="$tmp/corros"
  if ! git clone --depth 1 https://github.com/CocoCopi/corros.git "$repo" 2>/dev/null \
     && ! { [[ -f Cargo.toml && -f src/prelude.cor ]] && repo="$(pwd)"; }; then
    echo "corros: could not obtain the sources" >&2
    exit 1
  fi
  (cd "$repo" && cargo build --release)
  bin="$repo/target/release/corros"
  src_dir="$repo/src"
fi

# --- install --------------------------------------------------------------
install -m 0755 "$bin" "$bindir/corros"
for cor in compiler.cor vm.cor cli.cor prelude.cor; do
  if [[ -f "$src_dir/$cor" ]]; then
    install -m 0644 "$src_dir/$cor" "$bindir/$cor"
  else
    echo "corros: warning — $cor not found; some features will fail" >&2
  fi
done

echo
echo "Installed: $bindir/corros"
if [[ "$bindir" != /usr/local/bin && "$bindir" != /usr/bin ]]; then
  echo "Add it to your PATH:  export PATH=\"$bindir:\$PATH\""
fi
echo "Try it:  echo 'speak(\"corros works!\")' | \"$bindir/corros\""
"$bindir/corros" -v
