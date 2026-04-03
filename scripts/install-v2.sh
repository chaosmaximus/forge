#!/usr/bin/env bash
# Forge v0.4.0 — Install script
# Builds release binaries and symlinks to ~/.local/bin

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"

echo "Building Forge v0.4.0..."
cd "$REPO_ROOT"
cargo build --release -p forge-daemon -p forge-v2-cli

echo "Installing to ${BIN_DIR}..."
mkdir -p "$BIN_DIR"
ln -sf "${REPO_ROOT}/target/release/forge-daemon" "${BIN_DIR}/forge-daemon"
ln -sf "${REPO_ROOT}/target/release/forge-v2" "${BIN_DIR}/forge-v2"

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
echo "  forge-v2     → $(readlink -f "${BIN_DIR}/forge-v2")"
echo ""
echo "Usage:"
echo "  forge-v2 recall \"search query\"     # search memories (auto-starts daemon)"
echo "  forge-v2 remember --type decision --title \"...\" --content \"...\""
echo "  forge-v2 health                    # memory counts"
echo "  forge-v2 doctor                    # daemon health"
echo "  forge-v2 daemon stop               # stop daemon"
echo ""
echo "The daemon auto-starts on first command. No manual setup needed."
