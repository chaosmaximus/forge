# Bench composite baselines — protocol README

This directory pins the **locked composite floors** for the 6 functional
Forge benches. The single source of truth is
[`composites.json`](composites.json); this README documents the
recalibration protocol and how the file is consumed by CI.

## What composites.json encodes

Each bench has three numbers worth distinguishing:

| Field | Meaning | Source |
|-------|---------|--------|
| `composite_min` | The **spec-intended** minimum acceptable composite. A drop below this triggers a hard regression issue. | This file (locked at release boundaries). |
| `calibrated_value` | The **actual measured** composite at lock time. | Most recent results doc per bench. |
| `code_threshold` | The **runtime gate** in the bench code (`pass = composite >= X`). May be looser than `composite_min` for first-calibration tolerance. | `crates/daemon/src/bench/forge_*.rs`. |

The CI guard (`scripts/ci/check_bench_regression.py`) raises a
GitHub Issue when **either** of the following is true:

* `current_composite < composite_min` (hard floor breached), OR
* `current_composite < prior_composite - regression_policy.alarm_at`
  (5% drop vs the previous master run).

## When baselines are locked

Baselines are locked at **release boundaries** only:

| Event | Action |
|-------|--------|
| Release candidate cut (e.g. `v0.6.0-rc.3`) | `composite_min` stays, `calibrated_value` updated from re-run results. |
| GA release (e.g. `v0.6.0`) | Both fields updated; `locked_at` + `locked_at_sha` bumped. |
| Spec change (D1 weight changes, dim added/removed) | Hard re-lock: bump `schema_version` if shape changed, otherwise refresh both fields. |
| Daemon bug fix that improves a dim | `calibrated_value` only; `composite_min` stays. Don't ratchet the floor up reactively. |

## How to recalibrate

When a planned change to the bench or daemon is expected to shift the
composite (up OR down), follow this protocol:

1. **Land the change** with a commit message that includes `regression-expected: <bench-name>` or `improvement-expected: <bench-name>`.
2. **Run the bench** with `--seed 42` and capture the new composite.
3. **Adversarial review** of the diff (one general-purpose agent). The reviewer must classify the change as planned or unplanned regression.
4. **User sign-off** is required before lowering `composite_min`. Raising `calibrated_value` does not require sign-off.
5. **Update composites.json** with new values + bump `locked_at` + `locked_at_sha`.
6. **Annotate the results doc** with a "Calibration history" section noting the prior value, the new value, and the cause.

## How CI consumes the file

`scripts/ci/check_bench_regression.py` (added in P3-3 Stage 5 as
`2C-2 auto-PR-on-regression`):

1. Loads `composites.json` from the current checkout.
2. Loads the prior-master `summary.json` artifacts via
   `gh run list --workflow ci.yml --branch master --status success --limit 5`.
3. For each bench: compares current composite against
   `max(composite_min, prior_composite - 0.05)`.
4. If any bench fails: `gh issue create --label "bench-regression,automated"`
   with the diff and dim breakdown.

The `.github/workflows/bench-regression.yml` workflow runs this
post-CI on every master push.

## Related

* Plan: [`../../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.5 W3)
* CI guard: `scripts/ci/check_bench_regression.py`
* CI workflow: `.github/workflows/bench-regression.yml`
* Per-bench results docs: `../results/2026-04-26-forge-*-pre-release.md`
* Spec floors: each `docs/benchmarks/forge-*-design.md` §"Pass thresholds"
