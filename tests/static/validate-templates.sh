#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Template Validation ==="

# CONSTITUTION.md — must have Article headers
file="$PLUGIN_ROOT/templates/CONSTITUTION.md"
if grep -q '## Article' "$file"; then
  count=$(grep -c '## Article' "$file")
  echo "[PASS] CONSTITUTION.md: $count articles"
else
  echo "[FAIL] CONSTITUTION.md: no Article headers"; errors=$((errors + 1))
fi

# STATE.md — must have key tracking sections
file="$PLUGIN_ROOT/templates/STATE.md"
for section in "Current phase" "Decisions Made" "In Progress" "Completed" "Blockers" "Next Steps"; do
  if grep -qi "$section" "$file"; then
    echo "[PASS] STATE.md: has '$section'"
  else
    echo "[FAIL] STATE.md: missing '$section'"; errors=$((errors + 1))
  fi
done

# HANDOFF.md — must have resume sections
file="$PLUGIN_ROOT/templates/HANDOFF.md"
for section in "What Was Completed" "What Remains" "Resume Instructions"; do
  if grep -qi "$section" "$file"; then
    echo "[PASS] HANDOFF.md: has '$section'"
  else
    echo "[FAIL] HANDOFF.md: missing '$section'"; errors=$((errors + 1))
  fi
done

# PRD.md — must have key PRD sections
file="$PLUGIN_ROOT/templates/PRD.md"
for section in "Executive Summary" "Success Criteria" "User Journeys" "Functional Requirements" "Non-Functional" "Scope"; do
  if grep -qi "$section" "$file"; then
    echo "[PASS] PRD.md: has '$section'"
  else
    echo "[FAIL] PRD.md: missing '$section'"; errors=$((errors + 1))
  fi
done

# PRD.md — must have capability contract format guidance
if grep -q 'FR#' "$file" || grep -q 'Actor.*can' "$file"; then
  echo "[PASS] PRD.md: has FR capability contract format"
else
  echo "[WARN] PRD.md: capability contract format guidance not found"
fi

# PRD.md — must have NEEDS CLARIFICATION section
if grep -qi 'NEEDS CLARIFICATION' "$file"; then
  echo "[PASS] PRD.md: has NEEDS CLARIFICATION section"
else
  echo "[FAIL] PRD.md: missing NEEDS CLARIFICATION section"; errors=$((errors + 1))
fi

[ $errors -eq 0 ] && echo "=== All template checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
