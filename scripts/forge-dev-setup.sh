#!/usr/bin/env bash
# Forge Dev Setup — Configure Claude Code to use ONLY the forge plugin + its dependencies
# Usage: bash scripts/forge-dev-setup.sh
#
# What this script does:
# 1. Builds Rust binaries (forge, forge-hud, forge-daemon, forge-next)
# 2. Sets up Python venv for forge-graph
# 3. Installs forge as a local Claude Code plugin
# 4. Creates project-level settings that disable all other plugins
# 5. Enables only: forge, superpowers (orchestrated by forge), serena, episodic-memory, skill-creator
# 6. Checks codex CLI availability
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLAUDE_DIR="$HOME/.claude"
PLUGINS_FILE="$CLAUDE_DIR/plugins/installed_plugins.json"

# Compute the Claude Code project settings path (dashes for slashes)
PROJECT_KEY=$(echo "$REPO_ROOT" | sed 's|^/||; s|/|-|g')
PROJECT_SETTINGS_DIR="$CLAUDE_DIR/projects/-${PROJECT_KEY}"
PROJECT_SETTINGS_FILE="$PROJECT_SETTINGS_DIR/settings.json"

echo "╔═══════════════════════════════════════╗"
echo "║    Forge Dev Setup                    ║"
echo "╚═══════════════════════════════════════╝"
echo ""
echo "Repo:     $REPO_ROOT"
echo "Settings: $PROJECT_SETTINGS_FILE"
echo ""

# ── Step 1: Build Rust binaries ──────────────────────────────────
echo "═══ Step 1: Building Rust binaries ═══"
if ! command -v cargo &>/dev/null; then
  echo "[FAIL] cargo not found. Install Rust: https://rustup.rs"
  exit 1
fi

# Check Xcode license (macOS only)
if [[ "$(uname -s)" == "Darwin" ]]; then
  if ! xcodebuild -version &>/dev/null 2>&1; then
    echo "[WARN] Xcode license not accepted. Run: sudo xcodebuild -license"
    echo "       Skipping Rust build — run this script again after accepting."
    RUST_OK=false
  else
    RUST_OK=true
  fi
else
  RUST_OK=true
fi

if [ "$RUST_OK" = true ]; then
  cargo build --release 2>&1
  echo "[PASS] Built binaries:"
  ls -lh "$REPO_ROOT/target/release/forge" "$REPO_ROOT/target/release/forge-hud" \
         "$REPO_ROOT/target/release/forge-daemon" "$REPO_ROOT/target/release/forge-next" 2>/dev/null || true

  # Symlink to ~/.local/bin for PATH access
  BIN_DIR="${HOME}/.local/bin"
  mkdir -p "$BIN_DIR"
  for bin in forge forge-hud forge-daemon forge-next; do
    if [ -f "$REPO_ROOT/target/release/$bin" ]; then
      ln -sf "$REPO_ROOT/target/release/$bin" "$BIN_DIR/$bin"
      echo "  $bin → $BIN_DIR/$bin"
    fi
  done
fi
echo ""

# ── Step 2: Python venv for forge-graph ──────────────────────────
echo "═══ Step 2: Setting up Python venv ═══"
FORGE_GRAPH="$REPO_ROOT/forge-graph"
if [ -d "$FORGE_GRAPH/src" ]; then
  cd "$FORGE_GRAPH"
  if command -v uv &>/dev/null; then
    uv venv --python 3.11 2>/dev/null || python3 -m venv .venv 2>/dev/null || true
    uv pip install -e ".[dev]" 2>/dev/null || .venv/bin/pip install -e ".[dev]" 2>/dev/null || true
  elif command -v python3 &>/dev/null; then
    python3 -m venv .venv 2>/dev/null || true
    .venv/bin/pip install -e ".[dev]" 2>/dev/null || true
  fi
  cd "$REPO_ROOT"
  echo "[PASS] Python venv ready"
else
  echo "[SKIP] forge-graph/src not found"
fi
echo ""

# ── Step 3: Register forge as local plugin ───────────────────────
echo "═══ Step 3: Registering forge as local plugin ═══"
mkdir -p "$CLAUDE_DIR/plugins"

# Add forge-marketplace to known marketplaces
KNOWN_FILE="$CLAUDE_DIR/plugins/known_marketplaces.json"
if [ -f "$KNOWN_FILE" ]; then
  if ! python3 -c "import json; d=json.load(open('$KNOWN_FILE')); exit(0 if 'forge-marketplace' in d else 1)" 2>/dev/null; then
    python3 -c "
import json
with open('$KNOWN_FILE') as f:
    d = json.load(f)
d['forge-marketplace'] = {'source': {'source': 'github', 'repo': 'chaosmaximus/forge'}, 'installLocation': '$HOME/.claude/plugins/marketplaces/forge-marketplace'}
with open('$KNOWN_FILE', 'w') as f:
    json.dump(d, f, indent=2)
print('[PASS] Added forge-marketplace to known_marketplaces.json')
"
  else
    echo "[PASS] forge-marketplace already in known_marketplaces.json"
  fi
else
  python3 -c "
import json
d = {'forge-marketplace': {'source': {'source': 'local', 'path': '$REPO_ROOT'}}}
with open('$KNOWN_FILE', 'w') as f:
    json.dump(d, f, indent=2)
print('[PASS] Created known_marketplaces.json with forge-marketplace')
"
fi

# Register plugin in installed_plugins.json
if [ -f "$PLUGINS_FILE" ]; then
  python3 -c "
import json, datetime
with open('$PLUGINS_FILE') as f:
    d = json.load(f)

key = 'forge@forge-marketplace'
now = datetime.datetime.utcnow().isoformat() + 'Z'
entry = {
    'scope': 'local',
    'projectPath': '$REPO_ROOT',
    'installPath': '$REPO_ROOT',
    'version': '0.3.0',
    'installedAt': now,
    'lastUpdated': now
}

# Check if already registered as local for this project
existing = d.get('plugins', {}).get(key, [])
already = any(e.get('scope') == 'local' and e.get('projectPath') == '$REPO_ROOT' for e in existing)

if not already:
    existing.append(entry)
    d.setdefault('plugins', {})[key] = existing
    with open('$PLUGINS_FILE', 'w') as f:
        json.dump(d, f, indent=2)
    print('[PASS] Registered forge as local plugin for $REPO_ROOT')
else:
    print('[PASS] forge already registered as local plugin')
"
else
  echo "[WARN] installed_plugins.json not found — install a plugin first via claude CLI"
fi
echo ""

# ── Step 4: Project-level settings (disable all, enable forge deps) ──
echo "═══ Step 4: Creating project-level settings ═══"
mkdir -p "$PROJECT_SETTINGS_DIR"

# Read all user-level plugins and disable them, then selectively enable forge deps
python3 -c "
import json, os

user_settings_file = os.path.expanduser('~/.claude/settings.json')
user_plugins = {}
if os.path.exists(user_settings_file):
    with open(user_settings_file) as f:
        user_plugins = json.load(f).get('enabledPlugins', {})

# Start with everything disabled
enabled = {k: False for k in user_plugins}

# Enable ONLY what forge needs
# superpowers: forge orchestrates these internally — Claude must not call them directly
# serena: symbol navigation for forge agents
# episodic-memory: cross-session memory
# skill-creator: active for development
forge_deps = [
    'superpowers@claude-plugins-official',
    'serena@claude-plugins-official',
    'episodic-memory@superpowers-marketplace',
    'superpowers@superpowers-marketplace',
    'skill-creator@claude-plugins-official',
]
for dep in forge_deps:
    enabled[dep] = True

# Also disable common plugins that might interfere
for extra in [
    'apify-ultimate-scraper@apify-agent-skills',
    'apify-content-analytics@apify-agent-skills',
    'apify-audience-analysis@apify-agent-skills',
    'apify-influencer-discovery@apify-agent-skills',
    'apify-trend-analysis@apify-agent-skills',
    'apify-competitor-intelligence@apify-agent-skills',
    'figma@claude-plugins-official',
]:
    enabled[extra] = False

forge_hud_bin = '$REPO_ROOT/target/release/forge-hud'
forge_data = os.path.expanduser('~/.claude/plugins/data/forge-forge-marketplace')

settings = {
    '\$schema': 'https://json.schemastore.org/claude-code-settings.json',
    'enabledPlugins': enabled,
    'statusLine': {
        'type': 'command',
        'command': f'{forge_hud_bin} --state-dir {forge_data}'
    }
}

with open('$PROJECT_SETTINGS_FILE', 'w') as f:
    json.dump(settings, f, indent=2)

enabled_list = [k for k, v in enabled.items() if v]
disabled_list = [k for k, v in enabled.items() if not v]
print(f'[PASS] Project settings created')
print(f'  Enabled:  {len(enabled_list)} plugins')
for p in sorted(enabled_list):
    print(f'    + {p}')
print(f'  Disabled: {len(disabled_list)} plugins')
print(f'  StatusLine: forge-hud → {forge_hud_bin}')
"

# Ensure forge-hud data dir exists
mkdir -p "$HOME/.claude/plugins/data/forge-forge-marketplace/hud"
echo ""

# ── Step 5: Check codex ─────────────────────────────────────────
echo "═══ Step 5: Checking codex ═══"
if command -v codex &>/dev/null; then
  echo "[PASS] codex CLI: $(codex --version 2>&1)"
else
  echo "[WARN] codex not found. Install: npm install -g @openai/codex"
  echo "       Adversarial review will be unavailable."
fi

# Check codex Claude plugin
if python3 -c "import json; d=json.load(open('$PLUGINS_FILE')); exit(0 if any('codex' in k.lower() for k in d.get('plugins',{})) else 1)" 2>/dev/null; then
  echo "[PASS] codex Claude Code plugin installed"
else
  echo "[INFO] codex Claude Code plugin not installed."
  echo "       To install: claude plugin marketplace add openai/codex-plugin-cc"
  echo "       Then: claude plugin install codex@openai-codex-plugin-cc"
fi
echo ""

# ── Step 6: Validation ──────────────────────────────────────────
echo "═══ Step 6: Running validation ═══"
ERRORS=0
for validator in validate-plugin.sh validate-hooks.sh validate-skills.sh validate-agents.sh validate-csv.sh validate-rubrics.sh validate-templates.sh; do
  if [ -f "$REPO_ROOT/tests/static/$validator" ]; then
    if bash "$REPO_ROOT/tests/static/$validator" 2>&1 | tail -1 | grep -q "passed"; then
      echo "[PASS] $validator"
    else
      echo "[FAIL] $validator"
      ERRORS=$((ERRORS + 1))
    fi
  fi
done
echo ""

# ── Summary ─────────────────────────────────────────────────────
echo "═══════════════════════════════════════"
echo "  Forge Dev Setup Complete"
echo ""
echo "  Active plugins for this project:"
echo "    forge (local dev)           — orchestrates everything"
echo "    superpowers                 — invoked BY forge, not directly"
echo "    serena                      — symbol navigation for agents"
echo "    episodic-memory             — cross-session memory"
echo "    skill-creator               — skill development tooling"
echo ""
echo "  All other plugins DISABLED for this project."
echo ""
if [ "${RUST_OK:-true}" = false ]; then
  echo "  [!] Rust build skipped — accept Xcode license first:"
  echo "      sudo xcodebuild -license"
  echo "      Then re-run: bash scripts/forge-dev-setup.sh"
fi
echo "═══════════════════════════════════════"
