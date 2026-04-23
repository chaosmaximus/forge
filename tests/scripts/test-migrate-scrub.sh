#!/usr/bin/env bash
# test-migrate-scrub.sh — fixture-based validator for scripts/migrate-scrub.sh.
#
# Per-fixture assertions:
#   * leak fixtures  → scrub must exit != 0 AND stderr must contain expected
#                      category-marker string and/or expected filename.
#   * clean fixture  → scrub must exit == 0.
#
# Exits 0 iff every case passes. Prints PASS/FAIL per test + final summary.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SCRUB="$REPO/scripts/migrate-scrub.sh"
FIX_ROOT="$REPO/tests/fixtures/scrub"

if [ ! -x "$SCRUB" ] && [ ! -f "$SCRUB" ]; then
    echo "FATAL: scrub script not found at $SCRUB" >&2
    exit 2
fi
if [ ! -d "$FIX_ROOT" ]; then
    echo "FATAL: fixtures root missing: $FIX_ROOT" >&2
    exit 2
fi

PASS=0
FAIL=0
FAILED_TESTS=()

# run_leak_case <fixture-subdir> <expected-stderr-substring> [<extra-expected-substring>]
run_leak_case() {
    local sub="$1"
    local expect1="$2"
    local expect2="${3:-}"
    local dir="$FIX_ROOT/$sub"
    local out rc

    if [ ! -d "$dir" ]; then
        echo "FAIL  $sub (fixture dir missing: $dir)"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: fixture dir missing")
        return
    fi

    # Capture stderr only; stdout is harmless success-line noise on pass paths.
    set +e
    out=$(bash "$SCRUB" "$dir" 2>&1 1>/dev/null)
    rc=$?
    set -e

    if [ "$rc" -eq 0 ]; then
        echo "FAIL  $sub (expected non-zero exit, got 0)"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: scrub exit 0 but leak expected")
        return
    fi

    if ! printf '%s' "$out" | grep -qF -- "$expect1"; then
        echo "FAIL  $sub (stderr missing expected marker: '$expect1')"
        echo "---- stderr ----"
        printf '%s\n' "$out" | sed 's/^/  /'
        echo "----------------"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: missing '$expect1'")
        return
    fi

    if [ -n "$expect2" ] && ! printf '%s' "$out" | grep -qF -- "$expect2"; then
        echo "FAIL  $sub (stderr missing second expected marker: '$expect2')"
        echo "---- stderr ----"
        printf '%s\n' "$out" | sed 's/^/  /'
        echo "----------------"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: missing '$expect2'")
        return
    fi

    echo "PASS  $sub"
    PASS=$((PASS + 1))
}

# run_clean_case <fixture-subdir>
run_clean_case() {
    local sub="$1"
    local dir="$FIX_ROOT/$sub"
    local out_stdout out_stderr rc

    if [ ! -d "$dir" ]; then
        echo "FAIL  $sub (fixture dir missing: $dir)"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: fixture dir missing")
        return
    fi

    set +e
    out_stderr=$(bash "$SCRUB" "$dir" 2>&1 1>/dev/null)
    rc=$?
    out_stdout=$(bash "$SCRUB" "$dir" 2>/dev/null)
    set -e

    if [ "$rc" -ne 0 ]; then
        echo "FAIL  $sub (expected exit 0 on clean fixture, got $rc)"
        echo "---- stderr ----"
        printf '%s\n' "$out_stderr" | sed 's/^/  /'
        echo "----------------"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: clean fixture failed scrub")
        return
    fi

    if ! printf '%s' "$out_stdout" | grep -qF 'SCRUB PASSED'; then
        echo "FAIL  $sub (exit 0 but no SCRUB PASSED line on stdout)"
        FAIL=$((FAIL + 1))
        FAILED_TESTS+=("$sub: no SCRUB PASSED marker")
        return
    fi

    echo "PASS  $sub"
    PASS=$((PASS + 1))
}

echo "Running migrate-scrub fixture tests against: $SCRUB"
echo

# --- Text-scan leaks: assert [text] category + filename. ---
run_leak_case brand-leak         "LEAK [text]"    "brand-leak/README.md"
run_leak_case domain-leak        "LEAK [text]"    "domain-leak/plugin.json"
run_leak_case license-leak       "LEAK [text]"    "license-leak/LICENSE"
run_leak_case commercial-leak    "LEAK [text]"    "commercial-leak/pricing.md"
run_leak_case internal-url-leak  "LEAK [text]"    "internal-url-leak/notes.md"
run_leak_case aws-leak           "LEAK [text]"    "aws-leak/config.yaml"
run_leak_case private-path-leak  "LEAK [text]"    "private-path-leak/doc.md"

# --- Filename-scan leaks. ---
run_leak_case filename-leak-1    "LEAK [filename]" "SESSION-GAPS.md"
run_leak_case filename-leak-2    "LEAK [filename]" "PRICING-strategy.md"
run_leak_case env-file-leak      "LEAK [filename]" ".env.local"

# --- Binary strings (SVG title harbours brand). ---
# SVG is detected as text AND via strings; we assert the strings-category marker
# to prove the binary path wasn't silently bypassed.
run_leak_case binary-strings-leak "LEAK [binary strings]" "logo.svg"

# --- Archive / sqlite refuse. ---
run_leak_case archive-refuse     "LEAK [archive"   "bundle.tar.gz"
run_leak_case sqlite-refuse      "LEAK [database"  "data.sqlite"

# --- Clean control. ---
run_clean_case clean-sample

echo
echo "================================================"
echo "migrate-scrub fixture tests: $PASS passed, $FAIL failed"
if [ "$FAIL" -gt 0 ]; then
    echo "Failures:"
    for t in "${FAILED_TESTS[@]}"; do
        echo "  - $t"
    done
    exit 1
fi
echo "All checks passed."
exit 0
