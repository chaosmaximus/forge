#!/usr/bin/env bash
# Forge v2.0 — Complete install script
# Installs binaries, systemd/launchd service, hooks, default config, and verifies.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
FORGE_DIR="${HOME}/.forge"
VERSION="${FORGE_VERSION:-latest}"
BASE_URL="https://forge.bhairavi.tech/releases"

mkdir -p "$BIN_DIR" "$FORGE_DIR"

echo "╔══════════════════════════════════════╗"
echo "║  Forge — Cognitive Infrastructure    ║"
echo "║  for AI Agents                       ║"
echo "╚══════════════════════════════════════╝"
echo ""

# Input validation
if [[ "$VERSION" =~ [^a-zA-Z0-9._-] ]]; then
  echo "[forge] Error: FORGE_VERSION contains invalid characters: $VERSION" >&2
  exit 1
fi

# ── Step 1: Install binaries ──

download_binary() {
  local OS_TAG ARCH_TAG TARGET ARCHIVE URL TMP_DIR

  case "$(uname -s)" in
    Darwin) OS_TAG="apple-darwin" ;;
    Linux)  OS_TAG="unknown-linux-gnu" ;;
    *)      return 1 ;;
  esac

  case "$(uname -m)" in
    arm64|aarch64) ARCH_TAG="aarch64" ;;
    x86_64|amd64)  ARCH_TAG="x86_64" ;;
    *)             return 1 ;;
  esac

  TARGET="${ARCH_TAG}-${OS_TAG}"
  ARCHIVE="forge-${VERSION}-${TARGET}.tar.gz"
  URL="${BASE_URL}/${ARCHIVE}"

  TMP_DIR=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$TMP_DIR'" RETURN

  echo "[1/6] Downloading binaries (${TARGET})..."
  if command -v curl &>/dev/null; then
    curl -fsSL --retry 3 --retry-delay 2 "$URL" -o "${TMP_DIR}/forge.tar.gz" 2>/dev/null || return 1
  elif command -v wget &>/dev/null; then
    wget -q --tries=3 -O "${TMP_DIR}/forge.tar.gz" "$URL" 2>/dev/null || return 1
  else
    return 1
  fi

  tar -xzf "${TMP_DIR}/forge.tar.gz" -C "$TMP_DIR" 2>/dev/null || return 1

  local found=0
  for bin in forge-daemon forge-next; do
    if [ -f "${TMP_DIR}/${bin}" ]; then
      mv "${TMP_DIR}/${bin}" "${BIN_DIR}/${bin}"
      chmod +x "${BIN_DIR}/${bin}"
      found=1
    fi
  done

  [ "$found" -eq 1 ] && return 0 || return 1
}

if download_binary; then
  echo "  ✓ Installed from pre-built binary"
elif command -v cargo &>/dev/null && [ -f "${REPO_ROOT}/Cargo.toml" ]; then
  echo "[1/6] Building from source (this takes ~60 seconds)..."
  cd "$REPO_ROOT"
  cargo build --release -p forge-daemon -p forge-cli 2>/dev/null
  ln -sf "${REPO_ROOT}/target/release/forge-daemon" "${BIN_DIR}/forge-daemon"
  ln -sf "${REPO_ROOT}/target/release/forge-next" "${BIN_DIR}/forge-next"
  echo "  ✓ Built from source"
else
  echo "[forge] Error: Could not download binary or find cargo." >&2
  exit 1
fi

# ── Step 2: Ensure PATH ──

echo "[2/6] Configuring PATH..."
if ! echo "$PATH" | tr ':' '\n' | grep -Fqx "${BIN_DIR}"; then
  SHELL_RC="${HOME}/.bashrc"
  [ -f "${HOME}/.zshrc" ] && SHELL_RC="${HOME}/.zshrc"
  if ! grep -qF "/.local/bin" "$SHELL_RC" 2>/dev/null; then
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$SHELL_RC"
  fi
  export PATH="${BIN_DIR}:${PATH}"
fi
echo "  ✓ PATH configured"

# ── Step 3: Install system service ──

echo "[3/6] Installing system service..."
case "$(uname -s)" in
  Linux)
    SYSTEMD_DIR="${HOME}/.config/systemd/user"
    mkdir -p "$SYSTEMD_DIR"
    cat > "${SYSTEMD_DIR}/forge-daemon.service" << 'SYSTEMD_EOF'
[Unit]
Description=Forge Daemon — Cognitive Infrastructure for AI Agents
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/forge-daemon
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
Environment=HOME=%h
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=%h/.forge %h/.claude %h/.config %h/.codex

[Install]
WantedBy=default.target
SYSTEMD_EOF
    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user enable forge-daemon 2>/dev/null || true
    systemctl --user start forge-daemon 2>/dev/null || true
    echo "  ✓ systemd service installed and started"
    ;;
  Darwin)
    LAUNCHD_DIR="${HOME}/Library/LaunchAgents"
    mkdir -p "$LAUNCHD_DIR"
    cat > "${LAUNCHD_DIR}/com.forge.daemon.plist" << LAUNCHD_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.forge.daemon</string>
    <key>ProgramArguments</key>
    <array><string>${BIN_DIR}/forge-daemon</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>${FORGE_DIR}/daemon.log</string>
    <key>StandardErrorPath</key><string>${FORGE_DIR}/daemon.log</string>
</dict>
</plist>
LAUNCHD_EOF
    launchctl load "${LAUNCHD_DIR}/com.forge.daemon.plist" 2>/dev/null || true
    echo "  ✓ launchd service installed and started"
    ;;
  *)
    echo "  ⚠ Unknown OS — skipping service install. Start manually: forge-daemon &"
    ;;
esac

# ── Step 4: Create default config ──

echo "[4/6] Creating default configuration..."
CONFIG_FILE="${FORGE_DIR}/config.toml"
if [ ! -f "$CONFIG_FILE" ]; then
  cat > "$CONFIG_FILE" << 'CONFIG_EOF'
# Forge v2.0 Configuration
# See: forge-next config show

[extraction]
backend = "auto"  # auto, ollama, claude, claude_api, openai, gemini

[extraction.ollama]
model = "qwen3:0.6b"
endpoint = "http://localhost:11434"

[workers]
extraction_debounce_secs = 15
consolidation_interval_secs = 1800
embedding_interval_secs = 60
indexer_interval_secs = 300

[context]
budget_chars = 3000
decisions_limit = 10
lessons_limit = 5

[reality]
auto_detect = true
code_embeddings = false
community_detection = true

[a2a]
enabled = true
trust = "open"
CONFIG_EOF
  echo "  ✓ Default config created at ${CONFIG_FILE}"
else
  echo "  ✓ Config already exists"
fi

# ── Step 5: Install Claude Code hooks ──

echo "[5/6] Installing Claude Code hooks..."
CLAUDE_HOOKS_DIR="${HOME}/.claude"
if [ -d "$CLAUDE_HOOKS_DIR" ]; then
  # Check if hooks are already configured
  SETTINGS_FILE="${CLAUDE_HOOKS_DIR}/settings.json"
  if [ -f "$SETTINGS_FILE" ] && grep -q "forge" "$SETTINGS_FILE" 2>/dev/null; then
    echo "  ✓ Hooks already configured"
  else
    echo "  ✓ Claude Code detected — hooks will activate via plugin"
  fi
else
  echo "  ⚠ Claude Code not detected. Install Claude Code, then hooks will activate automatically."
fi

# ── Step 6: Verify installation ──

echo "[6/6] Verifying installation..."
echo ""

# Wait for daemon to start
sleep 2

if "${BIN_DIR}/forge-next" health >/dev/null 2>&1; then
  echo "  ✓ Daemon is healthy"
  "${BIN_DIR}/forge-next" manas-health 2>/dev/null || true
  echo ""
  # Auto-detect reality for current directory
  if [ -d ".git" ] || [ -f "Cargo.toml" ] || [ -f "package.json" ] || [ -f "pyproject.toml" ]; then
    "${BIN_DIR}/forge-next" detect-reality --path "$(pwd)" 2>/dev/null || true
  fi
else
  echo "  ⚠ Daemon not responding yet. It may need a moment to start."
  echo "    Try: forge-next health"
fi

echo ""
echo "╔══════════════════════════════════════╗"
echo "║  Forge is installed!                 ║"
echo "║                                      ║"
echo "║  Start using:                        ║"
echo "║    forge-next health                 ║"
echo "║    forge-next manas-health           ║"
echo "║    forge-next detect-reality --path . ║"
echo "║                                      ║"
echo "║  With Claude Code:                   ║"
echo "║    claude   (Forge hooks auto-inject) ║"
echo "║                                      ║"
echo "║  Docs: https://forge.bhairavi.tech   ║"
echo "╚══════════════════════════════════════╝"
