#!/bin/sh
set -e

# BoxRun installer — downloads the latest release binary for your platform.

REPO="boxlite-ai/boxrun"
INSTALL_DIR="${BOXRUN_INSTALL_DIR:-/usr/local/bin}"

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
curl -sL "$URL" -o "${TMP}/${ARCHIVE}"

echo "Installing to ${INSTALL_DIR}/boxrun..."
tar xzf "${TMP}/${ARCHIVE}" -C "${TMP}"

if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/boxrun" "${INSTALL_DIR}/boxrun"
else
  sudo mv "${TMP}/boxrun" "${INSTALL_DIR}/boxrun"
fi

chmod +x "${INSTALL_DIR}/boxrun"
rm -rf "$TMP"

echo "boxrun ${LATEST} installed to ${INSTALL_DIR}/boxrun"
echo ""
echo "Get started:"
echo "  boxrun serve     # Start the server"
echo "  boxrun create    # Create a box"
echo "  boxrun exec      # Run a command"
