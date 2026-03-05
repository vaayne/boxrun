#!/bin/bash
set -e

# Build boxrun from source and install to ~/.boxrun with a wrapper at /usr/local/bin/boxrun.
#
# Usage:
#   ./scripts/install-local.sh
#   BOXRUN_HOME=/opt/boxrun ./scripts/install-local.sh

BOXRUN_HOME="${BOXRUN_HOME:-$HOME/.boxrun}"
BIN_DIR="${BOXRUN_BIN_DIR:-/usr/local/bin}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BOXLITE_ROOT="$PROJECT_ROOT/../boxlite"

# 0) Auto-clone BoxLite if not present, pinned to Cargo.lock revision
BOXLITE_REV=$(grep -A2 'name = "boxlite"' "$PROJECT_ROOT/Cargo.lock" | sed -n 's/.*#\([0-9a-f]*\)".*/\1/p')
if [ ! -d "$BOXLITE_ROOT" ]; then
  echo "==> BoxLite not found at $BOXLITE_ROOT, cloning..."
  git clone --recurse-submodules \
    https://github.com/boxlite-ai/boxlite.git "$BOXLITE_ROOT"
fi
BOXLITE_ROOT="$(cd "$BOXLITE_ROOT" && pwd)"
if [ -n "$BOXLITE_REV" ]; then
  echo "==> Pinning BoxLite to Cargo.lock revision: $BOXLITE_REV"
  cd "$BOXLITE_ROOT" && git checkout "$BOXLITE_REV"
  cd "$PROJECT_ROOT"
fi

# 1) Build boxrun binary
echo "==> Building boxrun (release)..."
cd "$PROJECT_ROOT"
cargo build --release

# 2) Build BoxLite runtime (guest + shim + libs)
echo "==> Building BoxLite runtime..."
RUNTIME_DIR="$PROJECT_ROOT/target/boxlite-runtime"
bash "$BOXLITE_ROOT/scripts/build/build-runtime.sh" \
  --dest-dir "$RUNTIME_DIR" \
  --libs-dir "$(find "$PROJECT_ROOT/target/release/build" -path "*/boxlite-*/out/runtime" -type d | head -1)"

# 3) Install
echo "==> Installing to ${BOXRUN_HOME}..."
mkdir -p "${BOXRUN_HOME}/runtime"

cp "$PROJECT_ROOT/target/release/boxrun" "${BOXRUN_HOME}/boxrun"
chmod +x "${BOXRUN_HOME}/boxrun"

# Add rpath so the binary can find runtime dylibs (libkrun, libgvproxy, etc.)
if [ "$(uname)" = "Darwin" ]; then
  install_name_tool -add_rpath @executable_path/runtime "${BOXRUN_HOME}/boxrun" 2>/dev/null || true
fi

cp -f "$RUNTIME_DIR"/* "${BOXRUN_HOME}/runtime/" 2>/dev/null || true
chmod +x "${BOXRUN_HOME}/runtime/boxlite-guest" "${BOXRUN_HOME}/runtime/boxlite-shim" 2>/dev/null || true

# 4) Create symlink (the binary auto-detects runtime dir, no wrapper needed)
LINK="${BIN_DIR}/boxrun"
echo "==> Creating symlink at ${LINK}..."
if [ -w "$BIN_DIR" ]; then
  ln -sf "${BOXRUN_HOME}/boxrun" "$LINK"
else
  sudo ln -sf "${BOXRUN_HOME}/boxrun" "$LINK"
fi

echo ""
echo "Done! Installed:"
echo "  Binary:  ${BOXRUN_HOME}/boxrun"
echo "  Runtime: ${BOXRUN_HOME}/runtime/"
echo "  Symlink: ${LINK}"
echo ""
echo "Run: boxrun shell ubuntu"
