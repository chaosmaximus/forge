#!/usr/bin/env bash
# check_spans.sh — 2A-4d.1 T7 span-integrity + tokio::spawn guard.
#
# Two checks:
#   (1) every name in PHASE_SPAN_NAMES (instrumentation.rs) appears exactly
#       once as an `info_span!("<name>")` call site in consolidator.rs;
#   (2) the only non-test tokio::spawn calls inside crates/daemon/src/workers/
#       live in workers/mod.rs (the single allowed worker-spawn entry point).
#
# Exits 0 on success, 1 on any violation. Runs locally and in CI.
# Requires: bash, grep, awk, sed, rg (ripgrep optional — falls back to grep).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INSTRUMENTATION="$REPO_ROOT/crates/daemon/src/workers/instrumentation.rs"
CONSOLIDATOR="$REPO_ROOT/crates/daemon/src/workers/consolidator.rs"
WORKERS_DIR="$REPO_ROOT/crates/daemon/src/workers"

if [[ ! -f "$INSTRUMENTATION" ]]; then
  echo "ERR: instrumentation.rs not found at $INSTRUMENTATION" >&2
  exit 1
fi
if [[ ! -f "$CONSOLIDATOR" ]]; then
  echo "ERR: consolidator.rs not found at $CONSOLIDATOR" >&2
  exit 1
fi

fail=0

# ---------- Check 1: phase-span integrity ----------
# Extract every quoted string in the PHASE_SPAN_NAMES slice literal.
# Accepts any formatting: one-per-line, multi-per-line, trailing commas.
names_block=$(awk '
  /pub const PHASE_SPAN_NAMES/ { in_block = 1; next }
  in_block && /^\];/ { exit }
  in_block { print }
' "$INSTRUMENTATION")

if [[ -z "$names_block" ]]; then
  echo "ERR: PHASE_SPAN_NAMES slice not found in instrumentation.rs" >&2
  exit 1
fi

# Pull quoted identifiers; preserve execution order from the slice.
mapfile -t phase_names < <(printf '%s\n' "$names_block" | grep -oE '"[a-z0-9_]+"' | tr -d '"')

if [[ ${#phase_names[@]} -eq 0 ]]; then
  echo "ERR: no span names extracted from PHASE_SPAN_NAMES" >&2
  exit 1
fi

echo "==> Checking ${#phase_names[@]} phase span names..."

missing=()
duplicated=()
for name in "${phase_names[@]}"; do
  # Count exact matches of info_span!("<name>") — anchor to the literal form
  # the spec calls out. Accept both info_span! and info_span ! (formatter safety).
  count=$(grep -cE "info_span!\(\s*\"${name}\"" "$CONSOLIDATOR" || true)
  if [[ "$count" -eq 0 ]]; then
    missing+=("$name")
  elif [[ "$count" -gt 1 ]]; then
    duplicated+=("$name (count=$count)")
  fi
done

if [[ ${#missing[@]} -gt 0 ]]; then
  echo "ERR: phase span names missing from consolidator.rs:" >&2
  printf '  - %s\n' "${missing[@]}" >&2
  fail=1
fi
if [[ ${#duplicated[@]} -gt 0 ]]; then
  echo "ERR: phase span names appear more than once (each must wrap a unique call site):" >&2
  printf '  - %s\n' "${duplicated[@]}" >&2
  fail=1
fi

# ---------- Check 2: tokio::spawn whitelist ----------
echo "==> Checking tokio::spawn usage in workers/..."

# Find every tokio::spawn outside workers/mod.rs, then exclude lines whose
# surrounding module is `#[cfg(test)] mod tests` by filtering on a trailing
# test-only marker column. Simpler heuristic: if the file's block starting
# with `mod tests` contains the line number, it's a test.
violations=()
while IFS= read -r -d '' file; do
  base="$(basename "$file")"
  if [[ "$base" == "mod.rs" ]]; then
    continue
  fi

  # Locate `mod tests {` and compute its start line; everything at or below is test-scope.
  tests_start=$(grep -nE '^[[:space:]]*mod tests[[:space:]]*\{' "$file" | head -n1 | cut -d: -f1 || true)

  while IFS=: read -r lineno _rest; do
    [[ -z "$lineno" ]] && continue
    if [[ -n "$tests_start" && "$lineno" -ge "$tests_start" ]]; then
      continue
    fi
    violations+=("$file:$lineno")
  done < <(grep -nE 'tokio::spawn\s*\(' "$file" || true)
done < <(find "$WORKERS_DIR" -maxdepth 1 -name '*.rs' -print0)

if [[ ${#violations[@]} -gt 0 ]]; then
  echo "ERR: tokio::spawn found outside workers/mod.rs (and not inside tests):" >&2
  printf '  - %s\n' "${violations[@]}" >&2
  echo "    Workers must be spawned from mod.rs::spawn_workers. Move the call or scope it under #[cfg(test)]." >&2
  fail=1
fi

if [[ "$fail" -eq 0 ]]; then
  echo "OK: span integrity + tokio::spawn whitelist"
fi

exit "$fail"
