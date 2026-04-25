# Adversarial Review Artifacts (Phase 2P-1b §2)

Every adversarial review of a wave or phase lands here as a YAML
artifact. CI validates the artifacts via
`scripts/check-review-artifacts.sh`; a malformed YAML, an open
BLOCKER/CRITICAL/HIGH finding, or a missing `artifacts` list all fail
the build.

## Schema (`schema_version: 1`)

```yaml
schema_version: 1
slug: 2026-04-25-w1-harness-sync           # filename slug, no extension
target_paths:                               # paths reviewed; must exist at HEAD
  - scripts/check-harness-sync.sh
  - tests/scripts/test-harness-sync.sh
reviewer:
  agent: claude-general-purpose             # claude-general-purpose | codex-rescue
                                            # | skill-creator | <other>
  date: 2026-04-25                          # ISO YYYY-MM-DD
commit_range:
  base: 442a9b4                             # SHA the diff was reviewed against
  head: c261e99                             # SHA at review time
verdict: lockable-with-fixes                # lockable-as-is | lockable-with-fixes
                                            # | not-lockable
artifacts:                                  # must be non-empty
  - kind: review-transcript
    path: docs/superpowers/reviews/2026-04-25-w1-harness-sync.transcript.md
findings:                                   # optional; empty = clean review
  - id: H1                                  # short id (B1/H1/M1/L1/...)
    severity: HIGH                          # BLOCKER|CRITICAL|HIGH|MEDIUM|LOW|NIT
    summary: trap leak on parser-regression early exit
    file: scripts/check-harness-sync.sh
    line: 71
    status: resolved                        # resolved | deferred | open
    fixed_by: beeb2be                       # commit SHA that closed it (resolved)
                                            # OR rationale (deferred)
  - id: M1
    severity: MEDIUM
    summary: mode value not validated
    status: deferred
    rationale: matches FORCE_FAIL=1 legacy contract
```

## CI gate (`scripts/check-review-artifacts.sh`)

The validator asserts:

1. **Schema:** every YAML matches v1. Unknown top-level / artifact / finding
   keys WARN to stderr but don't fail (allows additive evolution).
2. **Existence + containment:** every `target_paths` and `artifacts[].path`
   entry must (a) be relative, (b) not escape the repo root via `..`, and
   (c) exist on disk.
3. **No open blockers:** any finding with `severity ∈ {BLOCKER, CRITICAL, HIGH}`
   AND `status == open` fails the gate. Resolved + deferred always pass.
4. **Status-coupled fields:** `status: resolved` requires `fixed_by`
   (commit SHA); `status: deferred` requires `rationale` (one-line why).
   `status: open` has no extra requirement (but blocks at item 3 if severity
   is BLOCKING).
5. **Artifacts non-empty:** at least one entry in `artifacts` (transcript,
   test output, dogfood log, etc.).
6. **Verdict in allowed set:** lockable-as-is | lockable-with-fixes | not-lockable.
7. **Robust YAML decoding:** scalars decoded by PyYAML (e.g. `date: 2026-13-45`
   → `datetime.date(...)` raises `ValueError`) are caught and reported as a
   single FAIL line per file, never a stack trace.

## Workflow per wave

1. Reviewer (Claude general-purpose, Codex codex-rescue, or skill-creator)
   produces a textual review. Author saves it to
   `docs/superpowers/reviews/<slug>.transcript.md`.
2. Author writes `<slug>.yaml` summarizing findings + verdict, points
   `artifacts` at the transcript.
3. Author addresses BLOCKER + HIGH; on each fix, updates the YAML's
   `findings[].status` to `resolved` and adds `fixed_by: <sha>`.
4. MEDIUM/LOW go to `status: deferred` with `rationale: <one-line>`.
5. Next push to master triggers `check-review-artifacts.sh`; clean run
   means the wave is reviewable-on-master.

## Backfill policy

YAML artifacts are required for every wave landing 2026-04-25 onwards
(P3-1 W1 is the first). Pre-2P-1b reviews live in commit history only
and do not need backfill.
