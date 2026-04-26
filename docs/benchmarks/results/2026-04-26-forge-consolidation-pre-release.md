# Forge-Consolidation — pre-release re-run — 2026-04-26

**Status:** PASS at HEAD `a9fa9af` (v0.6.0-rc.3).
**Bench:** `forge-bench forge-consolidation --seed 42`
**Design doc:** [`../forge-consolidation-design.md`](../forge-consolidation-design.md)
**Predecessor:** [`forge-consolidation-2026-04-17.md`](forge-consolidation-2026-04-17.md) (9 days stale at re-run time).
**Hardware:** Linux x86_64, GCP `chaosmaximus-instance` (`6.8.0-1053-gcp`).
**Bench binary:** `target/release/forge-bench` (release profile, `bench` feature, rebuilt at HEAD `a9fa9af`).

---

## 1. Summary

| Metric | Value | Threshold | Pass |
|--------|------:|----------:|:----:|
| `composite`               | **1.0000** | ≥ 0.80 | ✓ |
| `dedup_quality`           | 1.0000 | — | ✓ |
| `contradiction_handling`  | 1.0000 | — | ✓ |
| `reweave_enrichment`      | 1.0000 | — | ✓ |
| `quality_lifecycle`       | 1.0000 | — | ✓ |
| `recall_improvement`      | 1.0000 | — | ✓ |
| `recall_delta` (raw)      | 0.2667 | > 0.0 | ✓ |
| `recall_baseline_mean`    | 0.6667 | — | — |
| `recall_post_mean`        | 0.9333 | — | — |
| Wall-clock                | 389 ms | — | — |
| Infrastructure failures   | 0      | 0 | ✓ |

`composite = 0.25*1 + 0.20*1 + 0.15*1 + 0.15*1 + 0.25*1 = 1.0000`.

## 2. Per-dimension breakdown

* **D1 dedup_quality (1.0000)** — Phases 1, 2, 7. 18 ground-truth dedup
  victims observed (= 18 expected). Phase 1 removed 6, Phase 2 merged 8,
  Phase 7 merged 4.
* **D2 contradiction_handling (1.0000)** — Phases 9a, 9b, 12. 8 pairs
  detected (4 valence + 4 content), 8 expected; 4 resolutions
  synthesised by Phase 12.
* **D3 reweave_enrichment (1.0000)** — Phases 5, 14, 17, 18. reweave_f1=1,
  promotion=1, protocol=1, anti-pattern=1. 4 patterns (Phase 5),
  10 reweave pairs (Phase 14), 7 protocols (Phase 17), 3 anti-patterns
  (Phase 18).
* **D4 quality_lifecycle (1.0000)** — Phases 4, 6, 10, 15, 21, 22. All 6
  sub-accuracies at 1.0 (decay, recon, quality, activation, staleness,
  pressure).
* **D5 recall_improvement (1.0000)** — `mean_recall@10` lifted from
  baseline 0.6667 to post-consolidation 0.9333 (delta +0.2667). With
  `--expected-recall-delta` unset (first-calibration mode), any positive
  delta scores 1.0.

## 3. 22 consolidation phases — observed counts

| Phase | Action | Count |
|------:|-------|------:|
| 1  | dedup remove                  | 6  |
| 2  | semantic dedup merge          | 8  |
| 3  | link memories                 | 41 |
| 4  | decay fade                    | 9 (of 153 checked) |
| 5  | promote patterns              | 4 |
| 6  | reconsolidate contradicting   | 5 |
| 7  | embedding merge               | 4 |
| 8  | strengthen by access          | 15 |
| 9a | valence contradictions        | 4 |
| 9b | content contradictions        | 4 |
| 10 | activation decay              | 12 |
| 11 | entity detection              | 13 |
| 12 | synthesise contradiction resolutions | 4 |
| 13 | knowledge gaps                | 1 |
| 14 | reweave                       | 10 |
| 15 | quality scoring               | 118 |
| 16 | portability classification    | 118 |
| 17 | extract protocols             | 7 |
| 18 | tag anti-patterns             | 3 |
| 19 | notifications                 | 2 |
| 20 | topic supersede               | 6 (20 candidates, 4 false-positives skipped) |
| 21 | session staleness fade        | 1 |
| 22 | quality pressure              | 118 |

All 22 phases fired and produced expected counts.

## 4. Reproduction

```bash
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"
cargo build --release --features bench --bin forge-bench
./target/release/forge-bench forge-consolidation \
    --seed 42 \
    --output bench_results_consolidation_42
```

**Note:** do NOT pass `--expected-recall-delta 0.0` — `Some(0.0)` is treated
as INVALID by the scoring code (see `forge_consolidation.rs:2761-2772`),
which collapses the dimension score to 0.0. Omit the flag entirely for
first-calibration mode (any positive delta scores 1.0), or pass a positive
expected value (e.g. `--expected-recall-delta 0.20` to require ≥ 20% lift).

## 5. Comparison vs 2026-04-17 baseline

| Metric | 2026-04-17 | 2026-04-26 | Δ |
|--------|-----------:|-----------:|--:|
| composite               | 1.0000 | 1.0000 | 0 |
| dedup_quality           | 1.0000 | 1.0000 | 0 |
| contradiction_handling  | 1.0000 | 1.0000 | 0 |
| reweave_enrichment      | 1.0000 | 1.0000 | 0 |
| quality_lifecycle       | 1.0000 | 1.0000 | 0 |
| recall_improvement (score) | 1.0000 | 1.0000 | 0 |

No regression detected across 9 days of P3-1 + P3-2 + P3-3 changes.

## 6. Sensitivity (P3-3.7 W14)

Drift-fixture tests under `mod drift_fixtures` in
`crates/daemon/src/bench/forge_consolidation.rs` plant adversarial
regressions and assert D1 catches them:

| Test | Plants | Asserts |
|------|--------|---------|
| `d1_catches_planted_dedup_miss`              | re-activates 6 of 18 expected dedup victims | F1 drops below 0.95 |
| `d1_catches_planted_signal_preservation_failure` | supersedes a Category 3 control          | F1 collapses to 0.0  |

Run with:
```bash
cargo test -p forge-daemon --lib --features bench \
    bench::forge_consolidation::drift_fixtures -- --nocapture
```

D2-D5 sensitivity is covered by the calibrated PASS state (clean
corpus → 1.0 composite). Adding planted-regression tests for D2
(contradictions) and D3-D5 is deferred to v0.6.1+; the dims either
score against ground-truth pair sets (plantable but bounded by
synthesis correctness) or against state the consolidator phases
recompute on every pass (harder to plant durably).

## 7. References

* Spec: `docs/benchmarks/forge-consolidation-design.md`
* Plan: `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5 W1)
* Predecessor result: `docs/benchmarks/results/forge-consolidation-2026-04-17.md`
* Implementation: `crates/daemon/src/bench/forge_consolidation.rs`
* CLI subcommand: `crates/daemon/src/bin/forge-bench.rs::run_forge_consolidation`
