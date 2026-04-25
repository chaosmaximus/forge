#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# check-harness-sync.sh — 2P-1b §1 harness-drift detector.
#
# Validates that harness-facing files (plugin.json, marketplace.json, hooks/,
# scripts/hooks/, skills/, agents/, docs/) reference only JSON protocol
# methods + CLI subcommands that actually exist in the public daemon.
#
# Run modes:
#   default                          auto-derived from $AMNESTY_END_DATE:
#                                    WARN before, FAIL on/after.
#   FORGE_HARNESS_SYNC_ENFORCE=1     FAIL — exits non-zero on any drift.
#   FORGE_HARNESS_SYNC_ENFORCE=0     WARN — explicit override (escape hatch).
#   FORCE_FAIL=1                     legacy alias for FORGE_HARNESS_SYNC_ENFORCE=1.
#
# Exit codes:
#   0  — no drift, OR drift in WARN mode
#   1  — drift detected and mode == FAIL
#   2  — usage error / missing authoritative source / parser regression
#
# Arguments:
#   --root <dir>      override the inferred repo root (used by integration tests
#                     against fixture directories).
#
# Threshold overrides (used by fixture tests with synthetic small enums):
#   FORGE_HARNESS_SYNC_MIN_REQUEST   minimum Request enum variants the parser
#                                    must extract (default 50).
#   FORGE_HARNESS_SYNC_MIN_CLI       minimum CLI subcommands (default 20).
#
# Amnesty: this script lands 2026-04-25 in WARN mode. Auto-flips to FAIL on
# AMNESTY_END_DATE below. CI workflow doesn't need to set FORGE_HARNESS_SYNC_ENFORCE
# — the date check inside the script handles the flip without a workflow edit.

set -euo pipefail

# 14-day amnesty from 2026-04-25 (W1 land date) — fail-closed kicks in
# automatically once `date -u` reaches or exceeds this value.
AMNESTY_END_DATE="2026-05-09"

REPO_ROOT_OVERRIDE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --root)
            if [ -z "${2:-}" ]; then
                echo "ERROR: --root requires a path" >&2
                exit 2
            fi
            REPO_ROOT_OVERRIDE="$2"
            shift 2
            ;;
        --help|-h)
            sed -n '1,/^set -euo/p' "$0" | sed '$d'
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

if [ -n "$REPO_ROOT_OVERRIDE" ]; then
    REPO_ROOT="$REPO_ROOT_OVERRIDE"
else
    REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fi
REQ_RS="$REPO_ROOT/crates/core/src/protocol/request.rs"
CLI_MAIN="$REPO_ROOT/crates/cli/src/main.rs"

# Mode resolution. Explicit env vars override; otherwise auto-flip on date.
TODAY=$(date -u +%Y-%m-%d)
if [ -n "${FORGE_HARNESS_SYNC_ENFORCE:-}" ]; then
    MODE="$FORGE_HARNESS_SYNC_ENFORCE"
elif [ -n "${FORCE_FAIL:-}" ]; then
    MODE="$FORCE_FAIL"
elif [ "$TODAY" \> "$AMNESTY_END_DATE" ] || [ "$TODAY" = "$AMNESTY_END_DATE" ]; then
    MODE=1
else
    MODE=0
fi

MIN_REQUEST_VARIANTS="${FORGE_HARNESS_SYNC_MIN_REQUEST:-50}"
MIN_CLI_SUBCOMMANDS="${FORGE_HARNESS_SYNC_MIN_CLI:-20}"

[ -f "$REQ_RS" ] || { echo "missing $REQ_RS" >&2; exit 2; }
[ -f "$CLI_MAIN" ] || { echo "missing $CLI_MAIN" >&2; exit 2; }

# ─── 1. Authoritative Request variant → snake_case JSON method ─────────────
# Request enum is `#[serde(rename_all = "snake_case")]`, so PascalCase variant
# names map deterministically.
pascal_to_snake() {
    echo "$1" | awk '{
        s = ""; prev_lower = 0
        for (i=1; i<=length($0); i++) {
            c = substr($0, i, 1)
            if (c >= "A" && c <= "Z") {
                if (prev_lower) s = s "_"
                s = s tolower(c)
                prev_lower = 0
            } else {
                s = s c
                prev_lower = 1
            }
        }
        print s
    }'
}

pascal_to_kebab() {
    echo "$1" | awk '{
        s = ""; prev_lower = 0
        for (i=1; i<=length($0); i++) {
            c = substr($0, i, 1)
            if (c >= "A" && c <= "Z") {
                if (prev_lower) s = s "-"
                s = s tolower(c)
                prev_lower = 0
            } else {
                s = s c
                prev_lower = 1
            }
        }
        print s
    }'
}

request_methods_file="$(mktemp)"
trap 'rm -f "$request_methods_file" "$cli_commands_file" "$refs_file" 2>/dev/null || true' EXIT

grep -E '^\s+[A-Z][a-zA-Z0-9]+(\s*\{|,|\s*$)' "$REQ_RS" \
  | grep -vE '^\s*//|^\s*#|pub enum' \
  | sed -E 's/^\s+//; s/[{,].*$//; s/\s+$//' \
  | while IFS= read -r variant; do
      [ -z "$variant" ] && continue
      pascal_to_snake "$variant"
    done \
  | sort -u > "$request_methods_file"

# Sanity: at least MIN_REQUEST_VARIANTS variants expected.
req_count=$(wc -l < "$request_methods_file")
if [ "$req_count" -lt "$MIN_REQUEST_VARIANTS" ]; then
    echo "error: extracted only $req_count Request variants from $REQ_RS (min $MIN_REQUEST_VARIANTS) — parser regression?" >&2
    exit 2
fi

# ─── 2. Authoritative CLI subcommand names ─────────────────────────────────
# clap's rename_all for enum-variant subcommands defaults to kebab-case in
# clap v4, so collect explicit #[command(name = "...")] annotations AND
# kebab-case all variant names in the Commands enum.
cli_commands_file="$(mktemp)"

{
  grep -oE '#\[command\(name = "[^"]+"\)' "$CLI_MAIN" \
    | sed -E 's/.*name = "([^"]+)".*/\1/'

  awk '
    /^(pub )?enum Commands/ { in_enum=1; next }
    in_enum && /^\s*\}$/   { in_enum=0 }
    in_enum && match($0, /^\s+([A-Z][a-zA-Z0-9]+)(\s*\{|,|\s*$)/, m) {
        print m[1]
    }
  ' "$CLI_MAIN" \
    | while IFS= read -r variant; do
        [ -z "$variant" ] && continue
        pascal_to_kebab "$variant"
      done
} | sort -u > "$cli_commands_file"

cli_count=$(wc -l < "$cli_commands_file")
if [ "$cli_count" -lt "$MIN_CLI_SUBCOMMANDS" ]; then
    echo "error: extracted only $cli_count CLI subcommands from $CLI_MAIN (min $MIN_CLI_SUBCOMMANDS) — parser regression?" >&2
    exit 2
fi

# ─── 3. Scan harness files for references ──────────────────────────────────
# Files that ship as the agent-facing surface. We intentionally exclude
# crates/, target/, docs/superpowers/ (internal design docs), docs/benchmarks/
# (historical results) — those either ARE the source of truth or don't
# influence runtime behavior.
HARNESS_PATHS=(
    ".claude-plugin/plugin.json"
    ".claude-plugin/marketplace.json"
    "hooks"
    "scripts/hooks"
    "skills"
    "agents"
    "CLAUDE.md"
    "README.md"
)

refs_file="$(mktemp)"
# 3a. JSON method literals: "method":"foo"
for p in "${HARNESS_PATHS[@]}"; do
    path="$REPO_ROOT/$p"
    [ -e "$path" ] || continue
    grep -rohE '"method"\s*:\s*"[a-z_]+"' "$path" 2>/dev/null \
      | sed -E 's/.*:\s*"([a-z_]+)".*/METHOD \1/' || true
done > "$refs_file"

# 3b. CLI subcommand usage: `forge-next <subcmd>` or `forge-cli <subcmd>`
#     We restrict to tokens that look like real subcommands (lowercase,
#     1+ hyphens or letters) and skip obvious noise ("binary", "CLI", etc).
for p in "${HARNESS_PATHS[@]}"; do
    path="$REPO_ROOT/$p"
    [ -e "$path" ] || continue
    grep -rohE '\bforge-(next|cli)\s+([a-z][a-z-]+)' "$path" 2>/dev/null \
      | awk '{ print "CLI " $NF }' || true
done >> "$refs_file"

# ─── 4. Diff refs against authoritative sets ───────────────────────────────
SKIP_CLI_TOKENS=("binary" "cli")   # common English words that look like subcommands

unknowns=0
while IFS= read -r line; do
    kind="${line%% *}"
    sym="${line##* }"
    [ -z "$sym" ] && continue
    case "$kind" in
        METHOD)
            if ! grep -qxF "$sym" "$request_methods_file"; then
                echo "drift: unknown JSON method \"$sym\" referenced in harness but absent from Request enum" >&2
                unknowns=$((unknowns+1))
            fi
            ;;
        CLI)
            skip=0
            for tok in "${SKIP_CLI_TOKENS[@]}"; do
                [ "$sym" = "$tok" ] && skip=1 && break
            done
            [ "$skip" -eq 1 ] && continue
            if ! grep -qxF "$sym" "$cli_commands_file"; then
                echo "drift: unknown CLI subcommand \"forge-next $sym\" referenced in harness but absent from Commands enum" >&2
                unknowns=$((unknowns+1))
            fi
            ;;
    esac
done < <(sort -u "$refs_file")

if [ "$unknowns" -eq 0 ]; then
    echo "harness-sync: OK — ${req_count} JSON methods + ${cli_count} CLI subcommands authoritative, no drift"
    exit 0
fi

echo "" >&2
echo "harness-sync: $unknowns drift entries detected" >&2
echo "  — authoritative Request enum: $REQ_RS" >&2
echo "  — authoritative CLI Commands enum: $CLI_MAIN" >&2
echo "  — mode: $([ "$MODE" = 1 ] && echo FAIL || echo WARN)" >&2
if [ "$MODE" = 1 ]; then
    exit 1
fi
exit 0
