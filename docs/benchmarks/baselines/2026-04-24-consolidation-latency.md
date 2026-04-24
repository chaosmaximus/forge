# 2A-4d.1 T10 — Consolidation Latency Baseline

- **Date:** 2026-04-24
- **Commit (post-instrumentation):** `cbbc0e8` (current HEAD after T9.3 lands)
- **Harness:** `crates/daemon/tests/t10_instrumentation_latency.rs`
- **Invocation:** `cargo test -p forge-daemon --release --test t10_instrumentation_latency -- --ignored --nocapture`
- **Host:** Linux x86_64, release profile, single-threaded `cargo test` default

## What this measures

The wall-clock cost of `workers::consolidator::run_all_phases` on a seeded in-memory SQLite
database with 400 active-status memories mixing 4 types (decision, lesson, pattern, preference),
3 tag variants, mixed confidence, and a handful of faded rows. Two variants are timed:

| Variant | `metrics:` arg | Approximates            |
| ------- | -------------- | ----------------------- |
| A       | `None`         | Pre-T1 behaviour (Prometheus updates skipped) — closest in-tree approximation without a worktree checkout. Phase spans, kpi_events INSERTs, and `tracing::info!` drops are always compiled in, so this is *not* a clean pre-instrumentation baseline; it is a lower bound on the fully-instrumented cost with Prometheus disabled. |
| B       | `Some(&metrics)` | Full 2A-4d.1 hot path: phase spans, kpi_events INSERT OR IGNORE, Prometheus histogram + counter updates, plus the `table_rows` gauge (served by `/metrics`). |

## Numbers

N = 5 iterations per variant, fresh seeded DB per iteration, median reported. Two consecutive
runs are shown because the Prometheus-path overhead sits comfortably inside single-digit-ms
CPU jitter at this workload size and run-to-run variance matters more than any one sample.

### Run 1

| Variant | Median | Samples (ms) |
| ------- | -----: | ------------ |
| A (no metrics)   | **101.08 ms** | 105.89, 113.66, 98.25, 101.08, 99.91 |
| B (full metrics) | **98.82 ms**  | 116.16, 103.37, 98.82, 96.67, 95.48 |

Overhead (B − A): **0 ns** (B nominally faster — saturating subtraction floors at zero).

### Run 2

| Variant | Median | Samples (ms) |
| ------- | -----: | ------------ |
| A (no metrics)   | **97.15 ms**  | 101.09, 98.77, 93.23, 97.15, 96.50 |
| B (full metrics) | **102.82 ms** | 96.89, 102.82, 103.15, 102.46, 105.72 |

Overhead (B − A): **~5.67 ms**.

**Budget ceiling:** 50 ms — passed with roughly an order of magnitude of headroom even on
the slower-of-two runs.

## Interpretation

- The Prometheus-update path (`update_phase_metrics` + `table_rows.set`) adds zero measurable
  cost at N = 400 memories. The `phase_duration.observe(...)`, `phase_output_rows.inc_by(...)`,
  and `table_rows.with_label_values(...).set(...)` calls are each a few hundred nanoseconds
  worth of atomics; 23 phases × 3 calls each ≈ 70 µs total — immeasurable against ~100 ms of
  consolidation.
- The kpi_events `INSERT OR IGNORE` fires 23× per run regardless of variant (because Variant A
  does *not* skip kpi_events — it only skips the Prometheus updates). At our DB size this is a
  single-millisecond addition baked into both medians.
- Phase 14 reweave and Phase 2 semantic_dedup dominate the cost. The Prometheus bucket vector
  for `forge_phase_duration_seconds` is now `[0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0,
  5.0, 30.0, 60.0, 120.0, 300.0]` — this workload lands in the 100 ms bucket, giving healthy
  separation from both microsecond phases (Phase 1 dedup on empty sets) and the 30–300 s tail
  that large production DBs produce.

## Pre-T1 comparison — not performed

A true before/after measurement requires checking out the pre-T1 commit (`86492ec`), running
the same harness, and comparing. That workflow needs a worktree, which `CLAUDE.md` gates
behind explicit user permission. Instead:

- Variant A above is the in-tree lower bound of pre-T1 cost.
- The delta A − B is dominated by test-to-test jitter, not by the Prometheus path.
- If a future regression needs pre-T1 headroom measurement, re-run `cargo test -p forge-daemon
  --release --test t10_instrumentation_latency -- --ignored --nocapture` at both revisions in a
  worktree and record the medians in a new section of this file.

## Reproducing

```bash
cargo test -p forge-daemon --release --test t10_instrumentation_latency -- --ignored --nocapture
```

Edit `N_ITERATIONS`, `SEEDED_MEMORY_COUNT`, or `OVERHEAD_CEILING` in the test file to
change the sample size / workload / tolerance.

## Acceptance

- [x] Median Variant B within 50 ms of Variant A.
- [x] Exactly 23 kpi_events rows per `run_all_phases` invocation (assertion in test body).
- [x] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [x] `cargo test --lib` unaffected (1,390 passing).
