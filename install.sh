#!/bin/sh
set -e

# BoxRun installer — downloads the latest (or pinned) release binary.
#
# Usage:
#   curl -fsSL https://boxlite.ai/boxrun/install | sh
#
# Pin a specific version:
#   VERSION=v0.2.0 curl -fsSL https://boxlite.ai/boxrun/install | sh
#
# Environment variables:
#   VERSION           Version tag to install (default: latest)
#   BOXRUN_HOME       Installation directory (default: ~/.boxrun)
#   BOXRUN_BIN_DIR    Where to place the symlink (default: /usr/local/bin)
#   BOXRUN_ARCH       Override detected arch (amd64|arm64|x86_64|aarch64)

REPO="boxlite-ai/boxrun"
BOXRUN_HOME="${BOXRUN_HOME:-$HOME/.boxrun}"
BIN_DIR="${BOXRUN_BIN_DIR:-/usr/local/bin}"

# Temp directory with guaranteed cleanup
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

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

# Extract a JSON field value — try jq first, fall back to grep/sed.
json_field() {
  _field="$1"
  if command -v jq >/dev/null 2>&1; then
    jq -r ".$_field"
  else
    grep "\"$_field\":" | head -1 | sed -E 's/.*"([^"]+)".*/\1/'
  fi
}

OS=$(detect_os)
ARCH=${BOXRUN_ARCH:-$(detect_arch)}

case "$ARCH" in
  amd64) ARCH="x86_64" ;;
  arm64) ARCH="aarch64" ;;
  x86_64|aarch64) ;;
  *) ARCH="unsupported" ;;
esac

if [ "$OS" = "unsupported" ] || [ "$ARCH" = "unsupported" ]; then
  echo "Error: Unsupported platform $(uname -s)/$(uname -m)"
  exit 1
fi

ARCHIVE="boxrun-${ARCH}-${OS}.tar.gz"

# Determine version
if [ -n "$VERSION" ]; then
  TAG="$VERSION"
else
  TAG=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" | json_field tag_name)
  if [ -z "$TAG" ]; then
    echo "Error: Could not determine latest release"
    exit 1
  fi
fi

URL="https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE}"
CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${TAG}/checksums.txt"

echo "Downloading boxrun ${TAG} for ${OS}/${ARCH}..."
curl -fSL "$URL" -o "${TMP}/${ARCHIVE}"

# Portable SHA-256: prefer sha256sum (Linux), fall back to shasum (macOS).
compute_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo ""
  fi
}

# Verify checksum if checksums.txt is available
if curl -fsSL "$CHECKSUMS_URL" -o "${TMP}/checksums.txt" 2>/dev/null; then
  echo "Verifying checksum..."
  # checksums.txt has lines like: <hash>  <filename>
  EXPECTED=$(grep "$ARCHIVE" "${TMP}/checksums.txt" | awk '{print $1}')
  if [ -n "$EXPECTED" ]; then
    ACTUAL=$(compute_sha256 "${TMP}/${ARCHIVE}")
    if [ -z "$ACTUAL" ]; then
      echo "Warning: no sha256sum or shasum found, skipping verification"
    elif [ "$ACTUAL" != "$EXPECTED" ]; then
      echo "Error: Checksum verification failed!"
      echo "  Expected: $EXPECTED"
      echo "  Got:      $ACTUAL"
      exit 1
    else
      echo "Checksum verified."
    fi
  else
    echo "Warning: no checksum entry for ${ARCHIVE}, skipping verification"
  fi
else
  echo "Warning: checksums.txt not available, skipping verification"
fi

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

# Create symlink in BIN_DIR (replaces wrapper script — the binary auto-detects runtime dir)
LINK="${BIN_DIR}/boxrun"
echo "Creating symlink at ${LINK}..."
if [ -w "$BIN_DIR" ]; then
  ln -sf "${BOXRUN_HOME}/boxrun" "$LINK"
else
  sudo ln -sf "${BOXRUN_HOME}/boxrun" "$LINK"
fi

echo ""
echo "boxrun ${TAG} installed successfully!"
echo "  Binary:  ${BOXRUN_HOME}/boxrun"
echo "  Runtime: ${BOXRUN_HOME}/runtime/"
echo "  Symlink: ${LINK}"
echo ""
echo "Get started:"
echo "  boxrun shell ubuntu     # Launch an interactive VM"
echo "  boxrun --help           # See all commands"
echo ""
echo "Manage installation:"
echo "  boxrun upgrade          # Self-update to latest version"
echo "  boxrun uninstall        # Remove boxrun"
