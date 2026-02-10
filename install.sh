#!/bin/sh
set -e

# BoxRun installer — downloads the latest release binary for your platform.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/boxlite-ai/boxrun/main/install.sh | sh
#
# Environment variables:
#   BOXRUN_HOME       Installation directory (default: ~/.boxrun)
#   BOXRUN_BIN_DIR    Where to place the wrapper script (default: /usr/local/bin)

REPO="boxlite-ai/boxrun"
BOXRUN_HOME="${BOXRUN_HOME:-$HOME/.boxrun}"
BIN_DIR="${BOXRUN_BIN_DIR:-/usr/local/bin}"

detect_os() {
  case "$(uname -s)" in
    Linux*)  echo "linux" ;;
    Darwin*) echo "darwin" ;;
    *)       echo "unsupported" ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)  echo "x86_64" ;;
    arm64|aarch64) echo "aarch64" ;;
    *)             echo "unsupported" ;;
  esac
}

OS=$(detect_os)
ARCH=$(detect_arch)

if [ "$OS" = "unsupported" ] || [ "$ARCH" = "unsupported" ]; then
  echo "Error: Unsupported platform $(uname -s)/$(uname -m)"
  exit 1
fi

ARCHIVE="boxrun-${ARCH}-${OS}.tar.gz"

# Get latest release tag
LATEST=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$LATEST" ]; then
  echo "Error: Could not determine latest release"
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${LATEST}/${ARCHIVE}"

echo "Downloading boxrun ${LATEST} for ${OS}/${ARCH}..."
TMP=$(mktemp -d)
curl -fSL "$URL" -o "${TMP}/${ARCHIVE}"

echo "Installing to ${BOXRUN_HOME}..."

# Extract archive (contains boxrun/ directory with binary + runtime/)
tar xzf "${TMP}/${ARCHIVE}" -C "${TMP}"

# Create installation directory
mkdir -p "${BOXRUN_HOME}"

# Copy binary and runtime
cp "${TMP}/boxrun/boxrun" "${BOXRUN_HOME}/boxrun"
chmod +x "${BOXRUN_HOME}/boxrun"

if [ -d "${TMP}/boxrun/runtime" ]; then
  rm -rf "${BOXRUN_HOME}/runtime"
  cp -R "${TMP}/boxrun/runtime" "${BOXRUN_HOME}/runtime"
fi

rm -rf "$TMP"

# Create wrapper script in BIN_DIR
WRAPPER="${BIN_DIR}/boxrun"
WRAPPER_CONTENT="#!/bin/sh
BOXRUN_HOME=\"${BOXRUN_HOME}\"
export BOXLITE_RUNTIME_DIR=\"\${BOXRUN_HOME}/runtime\"
export DYLD_LIBRARY_PATH=\"\${BOXRUN_HOME}/runtime\${DYLD_LIBRARY_PATH:+:\$DYLD_LIBRARY_PATH}\"
exec \"\${BOXRUN_HOME}/boxrun\" \"\$@\"
"

echo "Creating wrapper at ${WRAPPER}..."
if [ -w "$BIN_DIR" ]; then
  printf '%s' "$WRAPPER_CONTENT" > "$WRAPPER"
  chmod +x "$WRAPPER"
else
  printf '%s' "$WRAPPER_CONTENT" | sudo tee "$WRAPPER" > /dev/null
  sudo chmod +x "$WRAPPER"
fi

echo ""
echo "boxrun ${LATEST} installed successfully!"
echo "  Binary:  ${BOXRUN_HOME}/boxrun"
echo "  Runtime: ${BOXRUN_HOME}/runtime/"
echo "  Wrapper: ${WRAPPER}"
echo ""
echo "Get started:"
echo "  boxrun serve     # Start the server"
echo "  boxrun create    # Create a box"
echo "  boxrun exec      # Run a command"
