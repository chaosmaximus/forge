#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — SessionStart hook
# Uses the proactive context compiler to assemble intelligent context
# from all Manas layers + identity + disposition.
#
# KV-cache optimization: caches the static prefix (platform, identity,
# disposition, tools) to a temp file for reuse by post-edit hooks.
# Only the dynamic suffix is regenerated on subsequent invocations.
#
# compile_context() assembles from all 9 layers:
# - Static: platform, identity, disposition, tools
# - Dynamic: decisions, lessons, skills, perceptions, working-set
# - Budget-limits to ~4000 chars (~1000 tokens)
# - Uses lazy loading for skills (summaries, not full steps)
# - Includes only critical/warning perceptions

set -euo pipefail
cat 2>/dev/null || true  # consume stdin

# P3-4 Wave Z (Z9) per CC voice feedback §2.10:
# Default behavior swallows hook errors so the SessionStart JSON channel
# stays clean (otherwise stderr leaks into Claude Code's hook dialog as
# non-JSON noise). Set FORGE_HOOK_VERBOSE=1 to see daemon failures (e.g.
# socket missing, compile-context errors, register-session timeouts).
# This is opt-in so day-to-day users keep the silent contract.
forge_log() {
  if [ "${FORGE_HOOK_VERBOSE:-0}" = "1" ]; then
    echo "[forge-hook] $*" >&2
  fi
}
if [ "${FORGE_HOOK_VERBOSE:-0}" = "1" ]; then
  FORGE_STDERR=/dev/stderr
else
  FORGE_STDERR=/dev/null
fi

# Find forge-next binary — auto-install if missing
FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi

# Auto-install if still not found
if ! command -v "$FORGE_NEXT" &>/dev/null && [ ! -x "$FORGE_NEXT" ]; then
  INSTALL_SCRIPT="${CLAUDE_PLUGIN_ROOT:-$(dirname "$(dirname "$0")")}/scripts/install.sh"
  if [ -f "$INSTALL_SCRIPT" ]; then
    bash "$INSTALL_SCRIPT" &>/dev/null || true
    FORGE_NEXT="$HOME/.local/bin/forge-next"
  fi
fi

# Detect project from CWD
PROJECT=""
if [ -n "${CLAUDE_CWD:-}" ]; then
  PROJECT=$(basename "$CLAUDE_CWD")
elif [ -n "${PWD:-}" ]; then
  PROJECT=$(basename "$PWD")
fi

# Register this session.
# Silence stdout always (the "Session registered: <id>" line otherwise
# leaks into the hook's JSON response channel and renders as non-JSON
# noise in the Claude Code hook dialog every SessionStart). Stderr is
# routed via FORGE_STDERR so FORGE_HOOK_VERBOSE=1 surfaces daemon
# failures during debugging while default-quiet behavior stays clean.
SESSION_ID="${CLAUDE_SESSION_ID:-session-$(date +%s)}"
forge_log "register-session id=$SESSION_ID project=${PROJECT:-} cwd=${CLAUDE_CWD:-}"
"$FORGE_NEXT" register-session \
  --id "$SESSION_ID" \
  --agent claude-code \
  ${PROJECT:+--project "$PROJECT"} \
  ${CLAUDE_CWD:+--cwd "$CLAUDE_CWD"} >/dev/null 2>"$FORGE_STDERR" || true

# Save session ID to a secure state directory (not world-writable /tmp)
# Uses $XDG_RUNTIME_DIR if available, otherwise ~/.forge/sessions/
FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
mkdir -p "$FORGE_SESSION_DIR" 2>/dev/null && chmod 700 "$FORGE_SESSION_DIR" 2>/dev/null
CWD_HASH=$(echo "${CLAUDE_CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
# Refuse to write if path is a symlink (symlink attack defense)
if [ -L "$SESSION_FILE" ]; then
  echo "[forge-hook] WARN: session file is a symlink — refusing to write (possible attack)" >&2
else
  echo "$SESSION_ID" > "$SESSION_FILE" || echo "[forge-hook] warn: could not save session file" >&2
  chmod 600 "$SESSION_FILE" 2>/dev/null || true
fi

# Generate and cache static prefix (stable for this session, reusable by post-edit hooks)
forge_log "compile-context --static-only project=${PROJECT:-} cwd=${CLAUDE_CWD:-}"
STATIC_PREFIX=$("$FORGE_NEXT" compile-context \
  --agent claude-code \
  --static-only \
  ${PROJECT:+--project "$PROJECT"} \
  ${CLAUDE_CWD:+--cwd "$CLAUDE_CWD"} 2>"$FORGE_STDERR" || echo "")

# Save static prefix to temp file for reuse by post-edit hooks
if [ -n "$STATIC_PREFIX" ]; then
  CACHE_FILE="/tmp/forge-static-prefix-${SESSION_ID}"
  echo "$STATIC_PREFIX" > "$CACHE_FILE"
  chmod 600 "$CACHE_FILE"  # restrict to owner only
fi

# Compile full context for initial injection (static + dynamic).
# Pass --cwd so a fresh project (no record yet) gets auto-created on
# first contact — agents see resolution="auto-created" in the rendered
# <code-structure> tag instead of resolution="no-match" and the project
# is bound to its canonical path from turn 1. P3-4 Wave Z (Z7).
forge_log "compile-context (full) project=${PROJECT:-} cwd=${CLAUDE_CWD:-}"
CONTEXT=$("$FORGE_NEXT" compile-context \
  --agent claude-code \
  ${PROJECT:+--project "$PROJECT"} \
  ${CLAUDE_CWD:+--cwd "$CLAUDE_CWD"} 2>"$FORGE_STDERR" || echo "<forge-context version=\"0.7.0\"/>")

# Escape for JSON output
CONTEXT_ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ')

# Output hook response
echo "{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"$CONTEXT_ESCAPED\"}}"
