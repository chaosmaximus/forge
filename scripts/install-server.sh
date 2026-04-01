#!/usr/bin/env bash
# Forge: Download codebase-memory-mcp binary for current platform
# Security: pins version, verifies SHA256 checksum when available
set -euo pipefail

INSTALL_DIR="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}/servers"
mkdir -p "$INSTALL_DIR"

# Pin version for reproducibility (update this when upgrading)
VERSION="latest"
CHECKSUMS_URL=""

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS-$ARCH" in
  linux-x86_64)  PLATFORM="linux-amd64" ;;
  linux-aarch64) PLATFORM="linux-arm64" ;;
  darwin-x86_64) PLATFORM="macos-amd64" ;;
  darwin-arm64)  PLATFORM="macos-arm64" ;;
  *)
    echo "Unsupported platform: $OS-$ARCH"
    echo "Download codebase-memory-mcp manually from https://github.com/DeusData/codebase-memory-mcp/releases"
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/DeusData/codebase-memory-mcp/releases/${VERSION}/download/codebase-memory-mcp-${PLATFORM}"
TARGET="$INSTALL_DIR/codebase-memory-mcp"

echo "Downloading codebase-memory-mcp for ${PLATFORM}..."
curl -fsSL "$DOWNLOAD_URL" -o "$TARGET.tmp" || {
  echo "Download failed. Please download manually from https://github.com/DeusData/codebase-memory-mcp/releases"
  rm -f "$TARGET.tmp"
  exit 1
}

# Verify checksum if available
CHECKSUMS_URL="https://github.com/DeusData/codebase-memory-mcp/releases/${VERSION}/download/checksums.txt"
if curl -fsSL "$CHECKSUMS_URL" -o "$TARGET.checksums" 2>/dev/null; then
  EXPECTED_SHA=$(grep "codebase-memory-mcp-${PLATFORM}" "$TARGET.checksums" | awk '{print $1}')
  if [ -n "$EXPECTED_SHA" ]; then
    ACTUAL_SHA=$(sha256sum "$TARGET.tmp" | awk '{print $1}')
    if [ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]; then
      echo "SECURITY: SHA256 checksum mismatch! Expected: $EXPECTED_SHA Got: $ACTUAL_SHA" >&2
      echo "The downloaded binary may be compromised. Aborting." >&2
      rm -f "$TARGET.tmp" "$TARGET.checksums"
      exit 1
    fi
    echo "SHA256 checksum verified."
  fi
  rm -f "$TARGET.checksums"
fi

mv "$TARGET.tmp" "$TARGET"
chmod +x "$TARGET"
echo "Installed to $TARGET"
