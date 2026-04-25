# P3-2 W1 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `a52cbc9` ("feat(P3-2 W1): Request::CompileContextTrace gains session_id")
**Base:** `03a6a8b` (P3-1 close)
**Verdict:** `lockable-with-fixes`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 0 | n/a |
| MEDIUM   | 0 | n/a |
| LOW      | 1 | resolved (this fix-wave) |

**Aggregate:** zero correctness/safety findings. Single cosmetic doc-orphan (LOW-1) — closed in the W1 fix-wave commit. One non-blocking observation about behavioral test coverage, deferred to P3-2 W6 cosmetic batch.

## LOW-1 — P3-1 deferred backlog SIGTERM bullet not crossed off after W7 lift

**Files:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md:74` + `HANDOFF.md:236-237`

**Reviewer rationale:**

> The bullet at line 74 still reads "Track for next P3-1 wave or P3-2" with no marker that the strategic fix has been re-homed to P3-2 W7 (line 91). Two readers of this file (a future you, an outside reviewer) will see two live entries for the same work item and not know whether one is stale. HANDOFF.md:236-237 has the same orphan reference. Cosmetic — does not affect runtime, build, or correctness.

**Fix:** appended `→ Lifted to P3-2 W7 per user sign-off 2026-04-25` to both bullets so the cross-reference is explicit and the entry is no longer mistakable for a still-deferred item.

## Notable non-findings (reviewer's own validation work)

The review went deeper than the verdict implies:

1. **Session-ownership SQL parity** — confirmed handler.rs CompileContextTrace arm is byte-identical to CompileContext at line 3149 (same query, same params, same `unwrap_or(false)`, same status enum `'active'/'idle'`). The W1 dormant rename concern was investigated and dismissed — `grep` for `dormant` and `SessionStatus::Dormant` returned zero matches across `crates/daemon`.

2. **Resolver call-shape parity** — `resolve_context_injection_for_session(state.conn, sid, Some(agent_name), &trace_config.context_injection)` matches the CompileContext callsite at handler.rs:3172-3177 modulo local-var name (`trace_config` vs `config`). Both pass `&<config>.context_injection` as the global baseline.

3. **JSON wire-compat is real, not just claimed** — `test_decode_from_raw_json` at contract_tests.rs:777 contains the probe `{"method":"compile_context_trace","params":{"agent":"claude-code"}}` (no `session_id` key). This decode test is the only thing exercising `#[serde(default)]`; if the annotation were dropped, the test would fail with "missing field session_id". Coverage is real.

4. **recall fn body verified clean** — `inj` is consumed via `&` only at lines 2097 (decisions) and 2172 (lessons). No residual `load_config()` inside the function. The `_agent` underscore was confirmed intentional (agent is consumed at the handler for inj resolution, not the recall layer).

5. **CLI plumbing complete** — clap subcommand at main.rs:368 declares `--session-id`, destructure at line 1471 forwards it, `commands::system::context_trace` gains the third `Option<String>` param. Doc-comment is accurate.

## Behavioral test gap (deferred, non-blocking)

The reviewer noted: the W1 diff has no new unit/integration test that actually flips a session-scoped `context_injection` flag and asserts the trace mirrors it. Coverage is structural (compiler proves the param threads end-to-end) plus contract test (proves wire decodes). A behavioral test would be additive — not blocking, since the structural+contract guard catches the original bug class (handler dropping `inj` on the floor).

Captured in P3-2 deferred backlog → P3-2 W6 cosmetic batch.

## Process check

* W4 protocol-hash interlock fired as designed on first protocol change since P3-1 W4 landed (drift `9a38d781…` → `3bac3136…` caught + sync helper rewrote).
* Wave timing: review dispatched ≈ commit-time; fix-wave commit follows in same session. Pattern matches P3-1 W1-W8 cadence.
