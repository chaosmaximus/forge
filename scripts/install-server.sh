#!/usr/bin/env bash
# Forge: Download codebase-memory-mcp binary for current platform
# Security: pins version, verifies SHA256 checksum, curl timeouts
set -euo pipefail

INSTALL_DIR="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}/servers"
mkdir -p "$INSTALL_DIR"

# Pin version for reproducibility and supply chain safety
# Update VERSION when upgrading
VERSION="v0.5.0"

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

DOWNLOAD_URL="https://github.com/DeusData/codebase-memory-mcp/releases/download/${VERSION}/codebase-memory-mcp-${PLATFORM}"
TARGET="$INSTALL_DIR/codebase-memory-mcp"

echo "Downloading codebase-memory-mcp ${VERSION} for ${PLATFORM}..."
curl -fsSL --connect-timeout 10 --max-time 60 "$DOWNLOAD_URL" -o "$TARGET.tmp" || {
  echo "Download failed. Please download manually from https://github.com/DeusData/codebase-memory-mcp/releases"
  rm -f "$TARGET.tmp"
  exit 1
}

# Verify checksum — fetch from release checksums file
EXPECTED_SHA=""
CHECKSUMS_URL="https://github.com/DeusData/codebase-memory-mcp/releases/download/${VERSION}/checksums.txt"
if curl -fsSL --connect-timeout 10 --max-time 15 "$CHECKSUMS_URL" -o "$TARGET.checksums" 2>/dev/null; then
  EXPECTED_SHA=$(grep "codebase-memory-mcp-${PLATFORM}" "$TARGET.checksums" | awk '{print $1}')
  rm -f "$TARGET.checksums"
fi

if [ -n "$EXPECTED_SHA" ]; then
  ACTUAL_SHA=$(sha256sum "$TARGET.tmp" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$TARGET.tmp" | awk '{print $1}')
  if [ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]; then
    echo "SECURITY: SHA256 checksum mismatch!" >&2
    echo "  Expected: $EXPECTED_SHA" >&2
    echo "  Got:      $ACTUAL_SHA" >&2
    echo "The downloaded binary may be compromised. Aborting." >&2
    rm -f "$TARGET.tmp"
    exit 1
  fi
  echo "SHA256 checksum verified."
else
  echo "WARNING: No checksum available for verification. Proceeding with unverified binary." >&2
  echo "For production use, verify the binary manually." >&2
fi

mv "$TARGET.tmp" "$TARGET"
chmod +x "$TARGET"
echo "Installed to $TARGET"
