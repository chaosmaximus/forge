#!/usr/bin/env bash
# Forge: Download codebase-memory-mcp binary for current platform
set -euo pipefail

INSTALL_DIR="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}/servers"
mkdir -p "$INSTALL_DIR"

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

LATEST_URL="https://github.com/DeusData/codebase-memory-mcp/releases/latest/download/codebase-memory-mcp-${PLATFORM}"

echo "Downloading codebase-memory-mcp for ${PLATFORM}..."
curl -fsSL "$LATEST_URL" -o "$INSTALL_DIR/codebase-memory-mcp" || {
  echo "Download failed. Please download manually from https://github.com/DeusData/codebase-memory-mcp/releases"
  exit 1
}
chmod +x "$INSTALL_DIR/codebase-memory-mcp"
echo "Installed to $INSTALL_DIR/codebase-memory-mcp"
