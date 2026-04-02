#!/usr/bin/env bash
# Forge v0.3.0 — Universal Installer
# Usage: curl -sSf https://raw.githubusercontent.com/chaosmaximus/forge/master/install.sh | bash
# Or run locally: bash install.sh
set -euo pipefail

VERSION="v0.3.0"
REPO="chaosmaximus/forge"

echo "╔═══════════════════════════════════╗"
echo "║    Forge $VERSION Installer       ║"
echo "╚═══════════════════════════════════╝"
echo ""

# 1. Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$OS-$ARCH" in
  linux-x86_64)  PLATFORM="linux-amd64" ;;
  linux-aarch64) PLATFORM="linux-arm64" ;;
  darwin-arm64)  PLATFORM="darwin-arm64" ;;
  darwin-x86_64) PLATFORM="darwin-amd64" ;;
  *) echo "Unsupported platform: $OS-$ARCH"; exit 1 ;;
esac

# 2. Determine install location
INSTALL_DIR="${CLAUDE_PLUGIN_ROOT:-$(pwd)}/servers"
mkdir -p "$INSTALL_DIR"

# 3. Download forge binary
echo "Downloading forge for $PLATFORM..."
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$VERSION/forge-$PLATFORM"
if command -v curl &>/dev/null; then
  curl -sSfL "$DOWNLOAD_URL" -o "$INSTALL_DIR/forge" || {
    echo "Download failed. Build from source: cargo install forge-agentic-os"
    exit 1
  }
elif command -v wget &>/dev/null; then
  wget -q "$DOWNLOAD_URL" -O "$INSTALL_DIR/forge" || {
    echo "Download failed. Build from source: cargo install forge-agentic-os"
    exit 1
  }
fi
chmod +x "$INSTALL_DIR/forge"
echo "✓ forge binary installed to $INSTALL_DIR/forge"

# 4. Setup Python venv for graph operations
FORGE_GRAPH="${CLAUDE_PLUGIN_ROOT:-$(pwd)}/forge-graph"
if [ -d "$FORGE_GRAPH/pyproject.toml" ] || [ -d "$FORGE_GRAPH/src" ]; then
  echo "Setting up Python venv..."
  cd "$FORGE_GRAPH"
  if command -v uv &>/dev/null; then
    uv venv --python 3.11 2>/dev/null || python3 -m venv .venv 2>/dev/null || true
    uv pip install -e . 2>/dev/null || .venv/bin/pip install -e . 2>/dev/null || true
  elif command -v python3 &>/dev/null; then
    python3 -m venv .venv 2>/dev/null || true
    .venv/bin/pip install -e . 2>/dev/null || true
  fi
  cd - > /dev/null
  echo "✓ Python venv ready"
else
  echo "⚠ forge-graph not found — graph operations will be unavailable"
fi

# 5. Run doctor
echo ""
"$INSTALL_DIR/forge" doctor --format text --state-dir "${CLAUDE_PLUGIN_DATA:-.forge}" 2>/dev/null || echo "⚠ Doctor check skipped"

echo ""
echo "═══════════════════════════════════"
echo "  Forge $VERSION installed"
echo ""
echo "  Install as Claude Code plugin:"
echo "    claude plugin install forge@forge-marketplace"
echo ""
echo "  Or install via cargo:"
echo "    cargo install forge-agentic-os"
echo ""
echo "  Or via Homebrew (coming soon):"
echo "    brew install chaosmaximus/tap/forge"
echo "═══════════════════════════════════"
