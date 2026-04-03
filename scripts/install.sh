#!/usr/bin/env bash
# Forge v0.4.0 — Install script
# Builds release binaries and symlinks to ~/.local/bin

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"

echo "Building Forge v0.4.0..."
cd "$REPO_ROOT"
cargo build --release -p forge-daemon -p forge-cli

echo "Installing to ${BIN_DIR}..."
mkdir -p "$BIN_DIR"
ln -sf "${REPO_ROOT}/target/release/forge-daemon" "${BIN_DIR}/forge-daemon"
ln -sf "${REPO_ROOT}/target/release/forge-next" "${BIN_DIR}/forge-next"

# Ensure PATH includes ~/.local/bin
if ! echo "$PATH" | grep -q "${BIN_DIR}"; then
    SHELL_RC="${HOME}/.bashrc"
    [ -f "${HOME}/.zshrc" ] && SHELL_RC="${HOME}/.zshrc"
    if ! grep -q '.local/bin' "$SHELL_RC" 2>/dev/null; then
        echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$SHELL_RC"
        echo "Added ~/.local/bin to PATH in ${SHELL_RC}"
    fi
    export PATH="${BIN_DIR}:${PATH}"
fi

echo ""
echo "Installed:"
echo "  forge-daemon → $(readlink -f "${BIN_DIR}/forge-daemon")"
echo "  forge-next   → $(readlink -f "${BIN_DIR}/forge-next")"
echo ""
echo "Usage:"
echo "  forge-next recall \"search query\"     # search memories (auto-starts daemon)"
echo "  forge-next remember --type decision --title \"...\" --content \"...\""
echo "  forge-next health                    # memory counts"
echo "  forge-next doctor                    # daemon health"
echo "  forge-next daemon stop               # stop daemon"
echo ""
echo "The daemon auto-starts on first command. No manual setup needed."
