#!/usr/bin/env bash
# Forge: Download forge-graph binary for current platform
# Security: pins version, verifies SHA256 checksum, curl timeouts
set -euo pipefail

INSTALL_DIR="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}/servers"
mkdir -p "$INSTALL_DIR"

# Pin version for reproducibility and supply chain safety
VERSION="v0.3.0"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS-$ARCH" in
  linux-x86_64)  PLATFORM="linux-amd64" ;;
  linux-aarch64) PLATFORM="linux-arm64" ;;
  darwin-x86_64) PLATFORM="macos-amd64" ;;
  darwin-arm64)  PLATFORM="macos-arm64" ;;
  *)
    echo "Unsupported platform: $OS-$ARCH"
    echo "Download forge-graph manually from https://github.com/chaosmaximus/forge/releases"
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/chaosmaximus/forge/releases/download/${VERSION}/forge-${PLATFORM}"
TARGET="$INSTALL_DIR/forge"

echo "Downloading forge ${VERSION} for ${PLATFORM}..."
curl -fsSL --connect-timeout 10 --max-time 60 "$DOWNLOAD_URL" -o "$TARGET.tmp" || {
  echo "Download failed. Please download manually from https://github.com/chaosmaximus/forge/releases"
  rm -f "$TARGET.tmp"
  exit 1
}

# Verify checksum — fetch from release checksums file
EXPECTED_SHA=""
CHECKSUMS_URL="https://github.com/chaosmaximus/forge/releases/download/${VERSION}/checksums.txt"
if curl -fsSL --connect-timeout 10 --max-time 15 "$CHECKSUMS_URL" -o "$TARGET.checksums" 2>/dev/null; then
  EXPECTED_SHA=$(grep "forge-${PLATFORM}" "$TARGET.checksums" | awk '{print $1}')
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
