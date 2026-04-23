#!/usr/bin/env bash
# migrate-scrub.sh — proprietary-leak scanner per spec v3 §2.2.
# Usage: scripts/migrate-scrub.sh <TARGET_DIR>
# Exit 0 iff zero leak matches across text / binary / filename scans.
# Exit non-zero on any match; prints matches to stderr.
set -euo pipefail

TARGET="${1:?usage: migrate-scrub.sh <target-dir>}"
WORKSPACE="$(cd "$(dirname "$0")/.." && pwd)"
LEXICON="$WORKSPACE/scripts/migrate-lexicon.txt"
ALLOWLIST="$WORKSPACE/scripts/migrate-scrub-allowlist.txt"

if [ ! -d "$TARGET" ]; then
    echo "ERROR: target is not a directory: $TARGET" >&2
    exit 2
fi

hits=0

# Returns 0 (allowed) if the file path matches any allowlist glob.
# Usage: is_allowed "<abs-path-or-relative>"
is_allowed() {
    local f="$1" glob
    [ -f "$ALLOWLIST" ] || return 1
    while IFS= read -r glob; do
        case "$glob" in ''|'#'*) continue ;; esac
        # Match glob against any suffix of the path so callers can use
        # relative-to-target patterns (e.g. skills/forge-security/**).
        case "$f" in
            */$glob|$glob|*/${glob%/**}/*|${glob%/**}/*) return 0 ;;
        esac
    done < "$ALLOWLIST"
    return 1
}

# --- Text scan ---
if [ -f "$LEXICON" ]; then
    active_lexicon=$(mktemp)
    grep -Ev '^\s*(#|$)' "$LEXICON" > "$active_lexicon"
    if [ -s "$active_lexicon" ]; then
        mapfile -t text_hits < <(grep -rIlFf "$active_lexicon" "$TARGET" 2>/dev/null || true)
        filtered_hits=()
        for f in "${text_hits[@]}"; do
            if is_allowed "$f"; then
                echo "  [allowlisted] $f" >&2
                continue
            fi
            filtered_hits+=("$f")
        done
        if [ "${#filtered_hits[@]}" -gt 0 ]; then
            echo "LEAK [text]: files matching lexicon:" >&2
            for f in "${filtered_hits[@]}"; do echo "  $f" >&2; done
            hits=$((hits + ${#filtered_hits[@]}))
        fi
    fi
    rm -f "$active_lexicon"
else
    echo "ERROR: lexicon missing: $LEXICON" >&2
    exit 2
fi

# --- Filename scan ---
while IFS= read -r -d '' f; do
    echo "LEAK [filename]: $f" >&2
    hits=$((hits + 1))
done < <(find "$TARGET" \
    \( -iname '*SESSION-GAPS*' \
    -o -iname '*STRATEGY*' \
    -o -iname '*PRICING*' \
    -o -iname '*-private.*' \
    -o -name '.env' \
    -o -name '.env.local' \
    -o -name '.env.prod*' \) \
    -print0 2>/dev/null)

# --- Binary / archive / sqlite scan ---
# Images + fonts: run strings + grep lexicon; images also exiftool author strip check.
# Archives: refuse entirely (surface for manual review).
# Sqlite/db: refuse entirely.
# wasm: strings + grep.
#
# We refuse (not silently strip) so the human sees the leak candidate.

while IFS= read -r -d '' f; do
    case "$f" in
        *.zip|*.tar|*.tgz|*.tar.gz|*.tar.bz2|*.7z)
            echo "LEAK [archive — refused, migrate manually if safe]: $f" >&2
            hits=$((hits + 1))
            ;;
        *.sqlite|*.sqlite3|*.db)
            echo "LEAK [database — refused]: $f" >&2
            hits=$((hits + 1))
            ;;
    esac
done < <(find "$TARGET" -type f \
    \( -iname '*.zip' -o -iname '*.tar' -o -iname '*.tgz' \
    -o -iname '*.tar.gz' -o -iname '*.tar.bz2' -o -iname '*.7z' \
    -o -iname '*.sqlite' -o -iname '*.sqlite3' -o -iname '*.db' \) \
    -print0 2>/dev/null)

# Images + fonts + wasm — use `strings` to dump readable text, grep against lexicon.
if command -v strings >/dev/null 2>&1; then
    active_lexicon=$(mktemp)
    grep -Ev '^\s*(#|$)' "$LEXICON" > "$active_lexicon"
    while IFS= read -r -d '' f; do
        if [ -s "$active_lexicon" ] && strings "$f" 2>/dev/null | grep -Ff "$active_lexicon" >/dev/null 2>&1; then
            echo "LEAK [binary strings]: $f" >&2
            hits=$((hits + 1))
        fi
    done < <(find "$TARGET" -type f \
        \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
        -o -iname '*.svg' -o -iname '*.webp' -o -iname '*.gif' -o -iname '*.ico' \
        -o -iname '*.woff' -o -iname '*.woff2' -o -iname '*.ttf' \
        -o -iname '*.otf' -o -iname '*.eot' \
        -o -iname '*.wasm' \) \
        -print0 2>/dev/null)
    rm -f "$active_lexicon"
else
    echo "WARN: 'strings' not available — skipping binary strings scan" >&2
fi

# Image EXIF author / creator check.
if command -v exiftool >/dev/null 2>&1; then
    while IFS= read -r -d '' f; do
        out=$(exiftool -s -s -s -Author -Creator -Artist -Copyright "$f" 2>/dev/null || true)
        if [ -n "$out" ] && echo "$out" | grep -iE '(bhairavi|proprietary|konurud|dsskonuru)' >/dev/null 2>&1; then
            echo "LEAK [image EXIF]: $f → $out" >&2
            hits=$((hits + 1))
        fi
    done < <(find "$TARGET" -type f \
        \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
        -o -iname '*.webp' -o -iname '*.gif' \) \
        -print0 2>/dev/null)
else
    echo "WARN: 'exiftool' not installed — skipping image EXIF scan. Install with: sudo apt-get install -y libimage-exiftool-perl" >&2
fi

if [ "$hits" -gt 0 ]; then
    echo "" >&2
    echo "SCRUB FAILED: $hits leak match(es). Abort migration." >&2
    exit 1
fi

echo "SCRUB PASSED: zero leak matches across text, filename, binary, EXIF scans."
exit 0
