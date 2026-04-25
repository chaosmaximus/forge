# P3-2 W2 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `5b24f8d` ("feat(P3-2 W2): batch resolve_scoped_config")
**Base:** `899bac2` (P3-2 W1 fix-wave)
**Verdict:** `lockable-with-fixes`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 0 | n/a |
| MEDIUM   | 2 | both resolved (this fix-wave) |
| LOW      | 2 | both deferred (cosmetic / future opt) |

**Aggregate:** zero correctness/safety findings. Two MEDIUM doc/test-coverage gaps closed in the fix-wave; two LOW items captured in plan-doc P3-2 deferred backlog.

## MED-1 — DB-error semantics diverge from old path

**File:** `crates/daemon/src/config.rs:644-655`

**Reviewer rationale:**

> Old path called resolve_scoped_config 6× independently; each Err was caught by `_ => current`, so a transient DB error on key K only suppressed K's override — keys 1..K-1 had already been honored. New path: if resolve_effective_config returns Err (which fires if ANY scope's list_scoped_config errors, OR any internal resolve_scoped_config errors during the second loop), the fn returns global before any take_bool runs, suppressing overrides for ALL 6 keys. The doc comment claims "byte-for-byte" preservation; this is technically untrue. In practice DB errors here are nearly always systemic (schema corruption), so observable impact is near-zero — but the claim should be softened, or the code should iterate per-key on Err to preserve the old semantics exactly.

**Fix:** softened the doc comment. Now reads: "Success-path behavior is preserved exactly … Error-path behavior change (W2 review MED-1): when resolve_effective_config returns Err, the new path returns global for all 6 toggles in one shot. The old path caught Err per-key and could return global for some keys while honoring others if the error were truly per-key (in practice, rusqlite::Error is systemic — a corrupt config_scope schema fails every key alike). The observable regression surface is therefore near-zero: any DB error here already means the daemon has bigger problems than which context_injection flag survives. We intentionally accept this trade for the perf win on the success path."

The slow-path option (per-key fallback on Err) was considered and rejected — it would re-introduce the redundancy the W2 refactor exists to eliminate, for no production benefit (DB errors here are systemic, never per-key). Design intent now visible in code.

## MED-2 — Test suite lacks non-session-scope precedence cases

**File:** `crates/daemon/src/config.rs:1856-2050`

**Reviewer rationale:**

> The 6 new tests exercise: no-sid, missing-row, no-overrides, session-override, unparseable, session-vs-org. Missing: team-only override (proves user_id/team_id/org_id columns are correctly threaded into resolve_effective_config), and reality-only override. Since resolve_effective_config builds its scope chain from the same args, a typo like passing user_id where team_id was expected would NOT be caught by the current test set — only the session-scope path is positively exercised. Old fn had the same risk but the W2 refactor is the perfect inflection point to add coverage.

**Fix:** added 3 new tests (~95 lines) bringing the resolve_context_injection coverage to 9 cases:

1. `test_resolve_context_injection_team_scope_override_propagates` — sets `context_injection.skills=false` at team scope `t1`, sets `session.team_id='t1'`, asserts `resolved.skills == false`. Catches a regression that drops team_id from the chain assembly.
2. `test_resolve_context_injection_user_scope_override_propagates` — sets `context_injection.anti_patterns=false` at user scope `u1`, asserts propagation. Catches a regression that drops user_id.
3. `test_resolve_context_injection_session_beats_team` — sets opposite values at team and session scopes, asserts session wins. Catches a regression that flips precedence (e.g., walks the chain in the wrong order).

All 9 tests pass. Total resolver-layer coverage:
- 4 fall-through paths (no-sid, missing-row, no-overrides, unparseable)
- 5 override-active paths (session-only, session-vs-org, team-scope, user-scope, session-vs-team)

## LOW-1 — Commit-message arithmetic ambiguity (deferred)

**Reviewer:** "36 unconditional resolutions" → directionally correct, units loose. Future commit-message hygiene to distinguish "resolve calls" from "underlying SELECTs". No code change; commit history immutable. Captured in P3-2 deferred backlog.

## LOW-2 — resolve_effective_config inner-loop optimization (deferred)

**Reviewer:** when K overrides are present, the new path issues 6 (list) + K × scopes-with-entry (resolve) SELECTs, possibly worse than the old 36 in the K=6 worst case. Production hot path is K=0 so this rarely fires. Out of scope for W2; recorded in plan-doc as a P3-3+ optimization candidate.

## Notable non-findings (reviewer's own validation work)

1. **Behavior preservation on success path:** confirmed identical. `take_bool` maps `effective.get(key)` to the same true / false / log+fallback semantics as the old `resolve_bool`. Empty-string and case-folding paths preserved.

2. **Scope-chain shape:** confirmed identical. `resolve_effective_config` drops `agent=None` / `user_id=None` / etc. exactly as `resolve_scoped_config` does (both use `if let Some(id) = ...`).

3. **Internal arg order:** `resolve_effective_config(conn, session_id, agent, reality_id, user_id, team_id, org_id)` forwards positionally to `resolve_scoped_config`. Same order, same values, as the old direct call site.

4. **Read-only invariant:** `list_scoped_config` and `resolve_scoped_config` are SELECT-only; W1 H1 writer-channel path unaffected.

5. **Concurrency:** both paths use `&Connection`. Total SELECT count drops in the no-override case (production hot path); holds steady at ~12 with one override; climbs only with many overrides. No new locking surface.

6. **Empty-string value:** `""` → `eq_ignore_ascii_case("true")` false, `eq_ignore_ascii_case("false")` false → debug log + `current`. Old and new paths identical.

## Process check

* Wave timing: W2 commit (`5b24f8d`) → review (this transcript) → fix-wave (next commit) all in same session.
* Pattern matches P3-1 W1-W8 cadence.
* Test growth in W2 (6 new) + W2-fix (3 new) = 9 new behavioral tests for resolve_context_injection_for_session — closing not just W2's coverage gap but partially the W1 review's deferred behavioral-test note (resolver layer covered; trace handler still pending W6).
