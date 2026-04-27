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

# P3-4 Wave Y (Y1) per CC voice Round 2 §C HIGH:
# Claude Code sends the SessionStart event as JSON on stdin:
#   {"session_id":"<UUID>","cwd":"<abs-path>","hook_event_name":"SessionStart",
#    "transcript_path":"...","source":"startup","model":"..."}
# Pre-Y1 the hook discarded stdin and fell back to CLAUDE_CWD /
# CLAUDE_SESSION_ID env vars that Claude Code does NOT export — so
# `--cwd` was never passed to compile-context (defeating Z7's
# auto-create end-to-end) and SESSION_ID became a Unix timestamp
# rather than CC's actual UUID (defeating update-session UX).
PAYLOAD="$(cat 2>/dev/null || true)"

# Parse top-level string fields. jq is the clean path; grep+sed is the
# portable fallback for hosts without jq (Claude Code's payload is a
# flat JSON object with simple string values, so the grep is robust
# enough for the well-formed input).
HOOK_CWD=""
HOOK_SESSION=""
if [ -n "$PAYLOAD" ]; then
  if command -v jq >/dev/null 2>&1; then
    HOOK_CWD="$(echo "$PAYLOAD" | jq -r '.cwd // empty' 2>/dev/null || true)"
    HOOK_SESSION="$(echo "$PAYLOAD" | jq -r '.session_id // empty' 2>/dev/null || true)"
  else
    HOOK_CWD="$(echo "$PAYLOAD" | grep -oE '"cwd"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*:[[:space:]]*"([^"]*)".*/\1/')"
    HOOK_SESSION="$(echo "$PAYLOAD" | grep -oE '"session_id"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*:[[:space:]]*"([^"]*)".*/\1/')"
  fi
fi

# Stdin-first → env-second → PWD-third precedence. The env-second tier
# is intentional: it lets non-CC agents (Codex, Cline, etc.) inject
# their own CWD via `CLAUDE_CWD` until they grow their own JSON
# payload contract. Timestamp fallback for SESSION_ID covers the
# legacy / unknown-agent case the same way.
CWD="${HOOK_CWD:-${CLAUDE_CWD:-${PWD:-}}}"
SESSION_ID="${HOOK_SESSION:-${CLAUDE_SESSION_ID:-session-$(date +%s)}}"

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

# Detect project from CWD (which now follows the stdin/env/PWD chain
# established above; pre-Y1 this only consulted CLAUDE_CWD which CC
# doesn't export, so projects always defaulted to basename($PWD)).
PROJECT=""
if [ -n "$CWD" ]; then
  PROJECT=$(basename "$CWD")
fi

# Register this session.
# Silence stdout always (the "Session registered: <id>" line otherwise
# leaks into the hook's JSON response channel and renders as non-JSON
# noise in the Claude Code hook dialog every SessionStart). Stderr is
# routed via FORGE_STDERR so FORGE_HOOK_VERBOSE=1 surfaces daemon
# failures during debugging while default-quiet behavior stays clean.
forge_log "register-session id=$SESSION_ID project=${PROJECT:-} cwd=${CWD:-}"
"$FORGE_NEXT" register-session \
  --id "$SESSION_ID" \
  --agent claude-code \
  ${PROJECT:+--project "$PROJECT"} \
  ${CWD:+--cwd "$CWD"} >/dev/null 2>"$FORGE_STDERR" || true

# Save session ID to a secure state directory (not world-writable /tmp)
# Uses $XDG_RUNTIME_DIR if available, otherwise ~/.forge/sessions/
FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
mkdir -p "$FORGE_SESSION_DIR" 2>/dev/null && chmod 700 "$FORGE_SESSION_DIR" 2>/dev/null
CWD_HASH=$(echo "${CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
# Refuse to write if path is a symlink (symlink attack defense)
if [ -L "$SESSION_FILE" ]; then
  echo "[forge-hook] WARN: session file is a symlink — refusing to write (possible attack)" >&2
else
  echo "$SESSION_ID" > "$SESSION_FILE" || echo "[forge-hook] warn: could not save session file" >&2
  chmod 600 "$SESSION_FILE" 2>/dev/null || true
fi

# Generate and cache static prefix (stable for this session, reusable by post-edit hooks)
forge_log "compile-context --static-only project=${PROJECT:-} cwd=${CWD:-}"
STATIC_PREFIX=$("$FORGE_NEXT" compile-context \
  --agent claude-code \
  --static-only \
  ${PROJECT:+--project "$PROJECT"} \
  ${CWD:+--cwd "$CWD"} 2>"$FORGE_STDERR" || echo "")

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
forge_log "compile-context (full) project=${PROJECT:-} cwd=${CWD:-}"
CONTEXT=$("$FORGE_NEXT" compile-context \
  --agent claude-code \
  ${PROJECT:+--project "$PROJECT"} \
  ${CWD:+--cwd "$CWD"} 2>"$FORGE_STDERR" || echo "<forge-context version=\"0.7.0\"/>")

# Escape for JSON output
CONTEXT_ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ')

# Output hook response
echo "{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"$CONTEXT_ESCAPED\"}}"
