#!/usr/bin/env bash
# check_spans.sh — 2A-4d.1 T7 span-integrity + tokio::spawn guard.
#
# Two checks:
#   (1) every name in PHASE_SPAN_NAMES (instrumentation.rs) appears exactly
#       once as a `tracing::info_span!` / `tracing::span!` / `#[instrument]`
#       reference to `"<name>"` in consolidator.rs;
#   (2) the only non-test `tokio::spawn(` calls inside
#       `crates/daemon/src/workers/*.rs` live in `mod.rs` (the allowed
#       worker-spawn entry point) or inside a `#[cfg(test)]`-gated module.
#
# Exits 0 on success, 1 on any violation. Runs locally and in CI.
# Requires: bash, grep, awk, sed.

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

# Strip line/block comments and string literals (regular and raw) before
# regex matching. Keeps line numbers by replacing comment/string bodies
# with spaces of the same length so downstream greps still report the
# right `file:line`.
#
# Phase 2A-4d.1.1 #3: extended to handle Rust raw strings of the form
# `r"..."`, `r#"..."#`, `r##"..."##`, ..., up to 16 hashes (Rust's
# practical max — anything higher is reserved syntax / vanishingly
# rare). In practice 1–2 covers everything in this codebase, 3+ is
# exotic. Without this, a raw string containing `{` / `}` would
# corrupt the brace-balance scope detection that Check 2 uses to find
# #[cfg(test)] modules.
#
# Not a full Rust lexer (the long-term fix is a `syn`-based rewrite,
# tracked under #4 in the same backlog). Sufficient to neutralise the
# documented false-positive classes: `// tokio::spawn(...)` in a line
# comment, "mod tests" inside a string literal, and `r#"...{...}..."#`
# in a raw string.
strip_comments_and_strings() {
  local file="$1"
  # Remove block comments via awk state machine (/* … */ across lines).
  awk '
    BEGIN { in_block = 0 }
    {
      line = $0
      out = ""
      i = 1
      while (i <= length(line)) {
        ch = substr(line, i, 1)
        nch = substr(line, i + 1, 1)
        if (in_block) {
          if (ch == "*" && nch == "/") {
            out = out "  "; i += 2; in_block = 0
          } else {
            out = out " "; i++
          }
          continue
        }
        if (ch == "/" && nch == "*") {
          out = out "  "; i += 2; in_block = 1
          continue
        }
        if (ch == "/" && nch == "/") {
          # Rest of line is a line comment.
          rest_len = length(line) - i + 1
          for (k = 0; k < rest_len; k++) out = out " "
          i = length(line) + 1
          continue
        }
        # Raw string: r"..." | r#"..."# | r##"..."## | r###"..."###
        if (ch == "r") {
          # Count opening hashes (0..3 supported).
          hash_count = 0
          j = i + 1
          while (j <= length(line) && substr(line, j, 1) == "#" && hash_count < 16) {
            hash_count++; j++
          }
          if (j <= length(line) && substr(line, j, 1) == "\"") {
            # Confirmed raw string opening: blank `r`, hashes, opening quote.
            for (k = 0; k <= hash_count + 1; k++) out = out " "
            i = j + 1
            # Scan for matching `"` followed by exactly hash_count #.
            while (i <= length(line)) {
              if (substr(line, i, 1) == "\"") {
                ok = 1
                for (k = 1; k <= hash_count; k++) {
                  if (substr(line, i + k, 1) != "#") { ok = 0; break }
                }
                if (ok) {
                  for (k = 0; k <= hash_count; k++) out = out " "
                  i = i + hash_count + 1
                  break
                }
              }
              out = out " "; i++
            }
            continue
          }
          # Not a raw string opener — fall through and treat r as literal.
        }
        if (ch == "\"") {
          # Consume string until unescaped closing quote. Blank out the body.
          out = out " "; i++
          while (i <= length(line)) {
            c = substr(line, i, 1)
            p = substr(line, i - 1, 1)
            if (c == "\"" && p != "\\") {
              out = out " "; i++; break
            }
            out = out " "; i++
          }
          continue
        }
        out = out ch
        i++
      }
      print out
    }
  ' "$file"
}

# ---------- Check 1: phase-span integrity ----------
# Extract every quoted string in the PHASE_SPAN_NAMES slice literal.
# (We read instrumentation.rs raw here — the slice *is* the contract.)
names_block=$(awk '
  /pub const PHASE_SPAN_NAMES/ { in_block = 1; next }
  in_block && /^\];/ { exit }
  in_block { print }
' "$INSTRUMENTATION")

if [[ -z "$names_block" ]]; then
  echo "ERR: PHASE_SPAN_NAMES slice not found in instrumentation.rs" >&2
  exit 1
fi

mapfile -t phase_names < <(printf '%s\n' "$names_block" | grep -oE '"[a-z0-9_]+"' | tr -d '"')

if [[ ${#phase_names[@]} -eq 0 ]]; then
  echo "ERR: no span names extracted from PHASE_SPAN_NAMES" >&2
  exit 1
fi

echo "==> Checking ${#phase_names[@]} phase span names..."

missing=()
duplicated=()
for name in "${phase_names[@]}"; do
  # Accept any of the canonical tracing invocation forms. We do NOT strip
  # strings from consolidator.rs here — the phase-name string IS a string
  # literal, so it has to survive.
  #   - tracing::info_span!("<name>") / info_span!("<name>")
  #   - tracing::info_span!(target: "...", "<name>")
  #   - tracing::span!(tracing::Level::INFO, "<name>")
  #   - #[tracing::instrument(name = "<name>")] / #[instrument(name = "<name>")]
  count=$(grep -cE \
    -e "(tracing::)?info_span[[:space:]]*!\([^)]*\"${name}\"" \
    -e "(tracing::)?span[[:space:]]*!\([^)]*Level::INFO[^)]*\"${name}\"" \
    -e "#\[(tracing::)?instrument[^]]*name[[:space:]]*=[[:space:]]*\"${name}\"" \
    "$CONSOLIDATOR" || true)
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

violations=()
while IFS= read -r -d '' file; do
  base="$(basename "$file")"
  if [[ "$base" == "mod.rs" ]]; then
    continue
  fi

  # Neutralise comments + string literals so `// tokio::spawn(` and
  # "mod tests" in a string can't trigger or exempt the guard.
  scrubbed=$(strip_comments_and_strings "$file")

  # Locate every test-scope `#[cfg(...)]` annotation on a module — the
  # module itself is declared on the very next `mod X {` line (in
  # practice, inner_attributes-in-test-modules convention).
  # A `tokio::spawn(` inside any such module (by line range to the matching
  # closing brace of that mod) is exempt.
  #
  # Phase 2A-4d.1.1 #3: the prior regex anchored on the literal
  # `#[cfg(test)]` form only, missing `#[cfg(all(test, feature="foo"))]`,
  # `#[cfg(any(test, ...))]`, and other compound forms. Broaden to any
  # `#[cfg(...)]` whose argument list contains the bare word `test`
  # delimited by non-identifier characters. Skip `#[cfg(not(test))]`
  # since that gates the inverse — code there is non-test and should
  # still be subject to the spawn whitelist.
  cfg_test_modules=$(printf '%s\n' "$scrubbed" | awk '
    /^[[:space:]]*#\[cfg\(/ && /[^a-zA-Z0-9_]test([^a-zA-Z0-9_]|$)/ && !/not\([^)]*test/ {
      cfg_line = NR
      in_expect = 1
      next
    }
    in_expect && /^[[:space:]]*mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*\{/ {
      mod_start = NR
      depth = 1
      for (n = NR + 1; n <= NR_limit && depth > 0; n++) { } # placeholder
      print cfg_line " " mod_start
      in_expect = 0
      next
    }
    in_expect && !/^[[:space:]]*$/ { in_expect = 0 }
  ')

  # For each #[cfg(test)] mod X { … }, find the matching close brace line by
  # counting { and } balance starting at mod_start. Build a list of exempt
  # line-ranges.
  declare -a ranges=()
  while read -r _cfg_line mod_start; do
    [[ -z "$mod_start" ]] && continue
    # Walk from mod_start forward counting braces in the scrubbed source.
    range_end=$(printf '%s\n' "$scrubbed" | awk -v start="$mod_start" '
      NR < start { next }
      {
        n = gsub(/\{/, "{", $0)
        m = gsub(/\}/, "}", $0)
        depth += n - m
        if (NR == start) depth = n - m
        if (NR > start && depth <= 0) { print NR; exit }
      }
    ')
    if [[ -n "$range_end" ]]; then
      ranges+=("$mod_start:$range_end")
    fi
  done <<< "$cfg_test_modules"

  # Now grep for tokio::spawn in the scrubbed source and filter by ranges.
  while IFS=: read -r lineno _rest; do
    [[ -z "$lineno" ]] && continue
    exempt=0
    for r in "${ranges[@]}"; do
      lo="${r%:*}"
      hi="${r#*:}"
      if [[ "$lineno" -ge "$lo" && "$lineno" -le "$hi" ]]; then
        exempt=1
        break
      fi
    done
    if [[ "$exempt" -eq 0 ]]; then
      violations+=("$file:$lineno")
    fi
  done < <(printf '%s\n' "$scrubbed" | grep -nE 'tokio::spawn[[:space:]]*\(' || true)
  unset ranges
done < <(find "$WORKERS_DIR" -maxdepth 1 -name '*.rs' -print0)

if [[ ${#violations[@]} -gt 0 ]]; then
  echo "ERR: tokio::spawn found outside workers/mod.rs (and not inside #[cfg(test)] modules):" >&2
  printf '  - %s\n' "${violations[@]}" >&2
  echo "    Workers must be spawned from mod.rs::spawn_workers. Move the call or scope it under #[cfg(test)]." >&2
  fail=1
fi

if [[ "$fail" -eq 0 ]]; then
  echo "OK: span integrity + tokio::spawn whitelist"
fi

exit "$fail"
