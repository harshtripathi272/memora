#!/bin/sh
# memora installer — downloads the latest release binary for your platform.
# Usage: curl -fsSL https://raw.githubusercontent.com/harshtripathi272/memora/main/install.sh | sh
set -e

REPO="harshtripathi272/memora"
INSTALL_DIR="${MEMORA_INSTALL_DIR:-/usr/local/bin}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  TARGET="x86_64-unknown-linux-gnu" ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      *)             TARGET="x86_64-apple-darwin" ;;
    esac
    ;;
  *) echo "Unsupported OS: $OS (use Windows .zip from GitHub Releases)"; exit 1 ;;
esac

LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)
if [ -z "$LATEST" ]; then
  echo "Error: could not determine latest release."
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${LATEST}/memora-${LATEST}-${TARGET}.tar.gz"
echo "Downloading memora ${LATEST} for ${TARGET}..."
curl -fsSL "$URL" | tar -xz

mv "memora-${LATEST}-${TARGET}/memora" "${INSTALL_DIR}/memora"
chmod +x "${INSTALL_DIR}/memora"
rm -rf "memora-${LATEST}-${TARGET}"

echo "Installed memora ${LATEST} to ${INSTALL_DIR}/memora"
echo "Run 'memora --help' to get started."
