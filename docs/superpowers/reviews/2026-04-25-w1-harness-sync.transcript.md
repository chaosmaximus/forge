# W1 Harness-Sync Adversarial Review — Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose (subagent dispatch)
**Diff reviewed:** `442a9b4..c261e99` ("P3-1 W1: harness-sync auto-amnesty +
drift fixtures + tests")
**Verdict:** lockable-with-fixes
**Closure commit:** `beeb2be` (HIGH-1 + M2 + M3 + L2 fixed)

## Reviewer output (verbatim)

```
Verdict: lockable-with-fixes

BLOCKER: (none)

HIGH:
- scripts/check-harness-sync.sh:128 — trap is installed referencing
  `$cli_commands_file` and `$refs_file` BEFORE those vars are assigned
  (lines 150 and 191). Combined with `set -u`, the parser-regression
  early-exit at line 143 (req_count<MIN) and line 172 (cli_count<MIN)
  triggers the trap, which then errors on unbound variable and exits
  with status 1 instead of the documented 2. Reproduced on
  /tmp/min-drift fixture: stderr shows `cli_commands_file: unbound
  variable` and `exit=1`. Failure mode: a real parser regression in
  production looks identical to "drift in FAIL mode" to any caller
  reading the exit code (CI step, downstream scripts). Fix: either
  initialize the three vars to "" before the trap, or move the trap
  installation to after all three mktemp calls.

MEDIUM:
- scripts/check-harness-sync.sh:72-80 — mode value is not validated.
  FORGE_HARNESS_SYNC_ENFORCE=true / =yes / =garbage silently downgrades
  to WARN because line 248-249 only accepts the literal string `1`.
  Defensible (matches FORCE_FAIL=1 legacy contract) but worth a
  one-line sanity check or doc note.
- scripts/check-harness-sync.sh:42-49 — `--root <flag>` swallows the
  next flag as a path because the guard only checks `-z "${2:-}"`.
  Caught indirectly by the missing-file check, but the error
  ("missing --bad-flag/crates/...") is confusing. Add a
  `case "$2" in --*) error;; esac` guard.
- tests/scripts/test-harness-sync.sh:71 — `assert_contains "no drift"`
  is anchored only to a substring; if a future legitimate output
  prints "no drift in CI banner: 0 OK" or similar, the assertion
  would still pass. Pin to the full line "harness-sync: OK — ... no
  drift" or a stable prefix.
- Locale (probe 8): no `LC_ALL=C` set on either script. `date -u
  +%Y-%m-%d` is locale-stable in practice, and `[ a \> b ]`
  lexicographic compare on YYYY-MM-DD is byte-safe, but the `awk`
  pascal-to-snake at lines 92-106 and the `grep`/`sort` pipelines
  could shift behavior under non-C locale (esp. `[A-Z]` ranges).
  Defer to backlog.
- Fixture authenticity (probe 4): drift fixtures use 6-variant
  clean-format Pascal enums with no doc-comments / cfg attrs / nested
  generics. They exercise the awk-based variant extractor's
  happy path only. A regression that broke handling of `Foo<T>`
  generics or `// doc` comments would slip through. Worth at least
  one fixture variant to bait the awk regex.

LOW / NIT:
- scripts/check-harness-sync.sh:52 — `--help` does
  `sed -n '1,/^set -euo/p' "$0" | sed '$d'`, which prints lines 1
  through the line before `set -euo`. Brittle if `set -e` ever
  moves; harmless.
- CI integration (probe 5): the new fixture-test step runs after
  the harness-drift step on the real repo. If the real-repo step
  fails (real drift), the fixture step is skipped (default
  `if: success()`). Since the fixture step's role is to certify the
  *detector* itself works, ordering it second is backwards — a
  parser regression would mask its own broken self-test. Consider
  `if: always()` or move it before. Defer to backlog.

Summary: One real bug (trap fires with unbound vars on
parser-regression early exit, returning the wrong exit code) and a
handful of edge-case gaps. The BLOCKER-class fix is mechanical (init
vars or move trap) and worth doing before close, since the
parser-regression branch is exactly the one a future contributor will
trip while editing the awk extractor. Fixture set is solid for
happy-path coverage but light on parser-edge variants.
Verdict: lockable-with-fixes — address the trap leak, lock the rest
as backlog.
```

## Resolution log

| Finding | Severity | Status | Closed by |
|---------|----------|--------|-----------|
| H1 (trap leak) | HIGH | resolved | `beeb2be` |
| M2 (`--root <flag>` guard) | MEDIUM | resolved | `beeb2be` |
| M3 (`assert_contains "no drift"` precision) | MEDIUM | resolved | `beeb2be` |
| L2 (CI step ordering) | LOW | resolved | `beeb2be` |
| M1 (mode value not validated) | MEDIUM | deferred | matches `FORCE_FAIL=1` legacy contract |
| M4 (`LC_ALL=C` not set) | MEDIUM | deferred | locale stability holds in practice |
| M5 (fixture authenticity gap) | MEDIUM | deferred | happy-path coverage is sufficient for W1 |
| L1 (`--help` sed brittleness) | LOW | deferred | cosmetic |
