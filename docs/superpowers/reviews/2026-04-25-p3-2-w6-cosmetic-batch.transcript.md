# P3-2 W6 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `d226ba3` ("feat(P3-2 W6): cosmetic batch")
**Base:** `2783861` (P3-2 W5 fix-wave)
**Verdict:** `lockable-as-is`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 0 | n/a |
| MEDIUM   | 0 | n/a |
| LOW      | 6 | 5 deferred (cosmetic / informational), 1 resolved |

**Aggregate:** zero correctness/safety findings. All 6 findings are LOW; the reviewer raised them in response to the prompt's checklist questions and concluded each "no fix needed" or accepted-as-designed. The single resolved item (LOW-6) was a one-liner test for the M3 negative-epoch clamp.

## LOW-6 — Test for M3 clamp (resolved)

**Reviewer rationale:**

> M3 clamp is unreachable from the only caller current_epoch_secs(); a 1-line test "epoch_to_iso(-1.0) == 1970-01-01T00:00:00Z" would lock behavior cheaply but isn't load-bearing.

**Fix:** added `test_epoch_to_iso_clamps_negative_epoch_to_unix_origin` covering:
- `epoch_to_iso(-1.0) == "1970-01-01T00:00:00Z"`
- `epoch_to_iso(-1e9) == "1970-01-01T00:00:00Z"`
- `epoch_to_iso(0.0) == "1970-01-01T00:00:00Z"` (boundary)
- `epoch_to_iso(1_704_067_200.0) == "2024-01-01T00:00:00Z"` (positive sanity)

Pins the M3 clamp behavior. 4-line addition; runs in 0ms.

## LOW-1 — Windows newline concern (deferred)

**Reviewer:** "git's %n format placeholder always emits a literal LF regardless of host OS (it's a format substitution, not a print). str::lines() splits on both \n and \r\n. Robust."

**Status:** no fix needed — concern raised in prompt does not materialize.

## LOW-2 — Fork count claim precision (deferred)

**Reviewer:** "Pre-W6 was 2 forks (status + show), post-W6 is 2 forks (log + status) — neutral on CI with GITHUB_SHA set. Saves up to one fork in the local-dev path (dominant case)."

**Status:** commit message phrasing is "saves one fork on every bench run" — technically "up to one fork", but accurate for the dominant local-dev path. No code change.

## LOW-3 — Negative-epoch silent clamp (deferred)

**Reviewer:** "Saturates to UNIX origin silently; no error path. Matches doc 'clamps to epoch itself'; bench-harness 'never panic' lane. An erroneous timestamp at the epoch boundary is visually obvious in dashboards (1970-01-01)."

**Status:** matches doc; bench-harness convention. No fix needed.

## LOW-4 — L2 unwrap_or unreachable (deferred — soundness verified)

**Reviewer math walk:**

> doy ≤ 365 (day-of-year)
> mp = (5*doy+2)/153 ≤ (5*365+2)/153 ≈ 11
> month: if mp<10 → mp+3 ∈ [3,12]; mp=10 → 1; mp=11 → 2 — all in [1,12]
> day: doy - (153*mp+2)/5 + 1; max at mp=11,doy=365 → 365 - 337 + 1 = 29 ≤ 31

Both u64 values bounded ≤ 31, fit u32 trivially. unwrap_or(1) is genuinely unreachable for non-negative z. Soundness argument confirmed.

## LOW-5 — #[serial] chain length (deferred)

**Reviewer:** "telemetry.rs already has 5 #[serial] tests (lines 268, 285, 338, 368, 386). Adding a 6th adds one more lock acquisition; sub-ms marginal cost. Defensive rationale sound."

**Status:** acceptable.

## Notable non-findings (reviewer's own validation work)

1. **Out-of-scope claims VERIFIED:**
   - L3 (kpi_reaper trace! downgrade) at `kpi_reaper.rs:85` bears `Phase 2A-4d.3.1 #6 L3 (W8)` marker.
   - M4 (compile-time tautology) at `forge_identity.rs:1270` bears `Phase 2A-4d.3.1 #6 M4 (W8)` marker.
   Both were closed in the prior phase as the commit-body claimed.

2. **Diff hygiene CLEAN:** every change maps to one of the five listed items; no drive-by edits, no whitespace churn, no unrelated rewrites. Doc-comment additions tightly scoped to the changed fns.

3. **GITHUB_SHA precedence preserved.** Pre-W6: env-var first, then `git rev-parse`. Post-W6: env-var first, then log_combined's SHA. Same precedence; only the lazy-vs-eager evaluation of log_combined changed.

4. **commit_ts under GITHUB_SHA: HEAD-vs-SHA mismatch** is a pre-existing footgun preserved across W6. If GITHUB_SHA points to a different commit than HEAD (rare CI configs), commit_ts is for HEAD. Pre-W6 had the same bug; not introduced by W6.

5. **L1 cast style.** `payload.pass as i64` reads consistently with the surrounding `as i64` cluster. `i64::from(bool)` is more pedantic but visually splits the cluster. Cosmetic-only.

## Process check

* Wave timing: W6 commit (`d226ba3`) → review (this transcript) → minimal fix-wave (LOW-6 only).
* All 5 cosmetic items in the 2A-4d.3.1 #6 backlog now closed.
* M4 + L3 from the same backlog were closed in the prior phase per the verified file references.
