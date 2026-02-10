#!/bin/bash
set -e

# Build boxrun from source and install to ~/.boxrun with a wrapper at /usr/local/bin/boxrun.
#
# Prerequisites: boxlite repo checked out at ../boxlite (sibling directory)
#
# Usage:
#   ./scripts/install-local.sh
#   BOXRUN_HOME=/opt/boxrun ./scripts/install-local.sh

BOXRUN_HOME="${BOXRUN_HOME:-$HOME/.boxrun}"
BIN_DIR="${BOXRUN_BIN_DIR:-/usr/local/bin}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BOXLITE_ROOT="$(cd "$PROJECT_ROOT/../boxlite" && pwd)"

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

cp -f "$RUNTIME_DIR"/* "${BOXRUN_HOME}/runtime/" 2>/dev/null || true
chmod +x "${BOXRUN_HOME}/runtime/boxlite-guest" "${BOXRUN_HOME}/runtime/boxlite-shim" 2>/dev/null || true

# 4) Create wrapper script
WRAPPER="${BIN_DIR}/boxrun"
WRAPPER_CONTENT="#!/bin/sh
BOXRUN_HOME=\"${BOXRUN_HOME}\"
export BOXLITE_RUNTIME_DIR=\"\${BOXRUN_HOME}/runtime\"
export DYLD_LIBRARY_PATH=\"\${BOXRUN_HOME}/runtime\${DYLD_LIBRARY_PATH:+:\$DYLD_LIBRARY_PATH}\"
exec \"\${BOXRUN_HOME}/boxrun\" \"\$@\"
"

echo "==> Creating wrapper at ${WRAPPER}..."
if [ -w "$BIN_DIR" ]; then
  printf '%s' "$WRAPPER_CONTENT" > "$WRAPPER"
  chmod +x "$WRAPPER"
else
  printf '%s' "$WRAPPER_CONTENT" | sudo tee "$WRAPPER" > /dev/null
  sudo chmod +x "$WRAPPER"
fi

echo ""
echo "Done! Installed:"
echo "  Binary:  ${BOXRUN_HOME}/boxrun"
echo "  Runtime: ${BOXRUN_HOME}/runtime/"
echo "  Wrapper: ${WRAPPER}"
echo ""
echo "Run: boxrun serve"
