# P3-2 W4 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `50e64d3` ("feat(P3-2 W4): record() span-scope refactor across 22 consolidator phases")
**Base:** `c37f59d` (P3-2 W3 fix-wave)
**Verdict:** `lockable`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 0 | n/a |
| MEDIUM   | 0 | n/a |
| LOW      | 2 | both deferred (cosmetic, pre-existing semantics preserved) |

**Aggregate:** zero correctness/safety findings. First `lockable` (no fixes needed) verdict in P3-2; refactor is clean. Two LOW items captured "for the record" — neither is actionable.

## LOW-1 — stats.healed_faded / stats.healed_superseded writes moved inside Phase 21/20 spans

**File:** `crates/daemon/src/workers/consolidator.rs:1039` (Phase 21), parallel pattern at line 1002 (Phase 20).

**Reviewer rationale:**

> Pre-W4 wrote stats.healed_faded outside the span; post-W4 writes it inside. Since the read (line 1095, healing notification) happens at function scope after the let-binding drops the span, observability is identical and no behavior change occurs.

**Status:** deferred. Cosmetic — pre-existing semantics preserved.

## LOW-2 — Phase 17 / 21 tracing!() field syntax shifted from shorthand to explicit alias

**File:** `crates/daemon/src/workers/consolidator.rs:693, 1040`

**Reviewer rationale:**

> `tracing::info!(protocols, ...)` → `tracing::info!(protocols = p, ...)` and similarly `tracing::info!(healed_faded, ...)` → `tracing::info!(healed_faded = faded, ...)`. Emitted span field name preserved. No event-name or schema change. Required because the local binding renamed (protocols→p, healed_faded→faded) when the external `let protocols;` / `let healed_faded;` declarations were folded into the let-binding tuple destructure.

**Status:** deferred. Semantically equivalent.

## Notable non-findings (reviewer's own validation work)

The reviewer went deep on this refactor — 22 sites with subtle shadowing concerns:

1. **Behavior preservation confirmed** across spot-checks of phases 1, 4, 9, 17, 19, 20, 21. Span enter/drop boundaries at exactly the same lexical positions. Only the record() call moves outside the block. `t0.elapsed()` still captured inside the span scope (excludes record() cost in both pre and post).

2. **Phase 9 dual-strategy** correctly threads all 5 values + duration: `(output, err, valence_summary, content_contradictions, content_errors, phase_9_duration_ms)`. Aggregate `output_count = output + content_contradictions`, `error_count = err + content_errors`, both unchanged. `stats.contradictions += ...` accumulation preserved inside span scope.

3. **Phase 17 protocols escape:** pre-W4 used outer `let protocols;` reused by Phase 19 notification (line 790). Post-W4 destructures protocols out of the let-binding tuple at function scope — identical scoping.

4. **Phase 21 healed_faded escape:** same pattern — folded from outer `let healed_faded;` into tuple destructure, still at function scope, still consumed at line 1097 (`healed_faded > 0` healing notification predicate).

5. **Phase 4 checked_count:** extended tuple `(output, err, checked_count, phase_4_duration_ms)` correctly carries the value into the `extra: serde_json::json!({ "checked_count": checked_count })` payload.

6. **Phase 19 itself untouched.** Only the `let (notifs_generated, ...) = {` destructure line is in the diff context as a hunk anchor — body lines unmodified.

7. **Phase 23 ordering quirk** (physically between 17 and 18 in source order) preserved unchanged.

8. **Variable shadowing benign:** each phase's `(output, err)` is consumed by record() before the next phase rebinds, so each binding has exactly one consumer. No dangling/unused names; no aliasing.

9. **Span-integrity counts:**
   - 23 `let _span = tracing::info_span!("phase_` declarations
   - 23 `duration_ms: phase_X_duration_ms` PhaseOutcome assignments
   - 0 leftover `duration_ms: t0.elapsed()` direct usages
   Matches PHASE_SPAN_NAMES.len()=23 in instrumentation.rs.

10. **Healing notification** correctly reads both `healing_stats_topic_superseded` (saved local at line 1030 after Phase 20 record()) and `healed_faded` (from Phase 21 let-binding destructure). Two-source threading intact.

11. **Diff strictly mechanical** — no logic drift, no missed tracing log, no error_count semantics change, no extra-field shape change. Only logic-equivalent rewrites: `stats.protocols_extracted = protocols→p` (same value), `stats.healed_faded = healed_faded→faded` (same value).

12. **Test failure** (`test_daemon_state_new_is_fast`) is the documented timing flake per HANDOFF "Known quirks" since 2P-1a; passes in isolation. Not introduced by W4. Single failure, otherwise green (1484/1485).

## Process check

* Wave timing: W4 commit (`50e64d3`) → review (this transcript) → first `lockable` verdict in P3-2.
* No fix-wave commit needed — both LOWs are deferred cosmetic notes.
* Pattern matches P3-1 + P3-2 W1-W3 cadence; this wave is the cleanest yet.
