# Forge-Context — pre-release re-run — 2026-04-26

**Status:** PASS at HEAD `a9fa9af` (v0.6.0-rc.3).
**Bench:** `forge-bench forge-context --seed 42`
**Predecessor:** [`forge-context-2026-04-16.md`](forge-context-2026-04-16.md) (10 days stale at re-run time).
**Hardware:** Linux x86_64, GCP `chaosmaximus-instance` (`6.8.0-1053-gcp`).
**Bench binary:** `target/release/forge-bench` (release profile, `bench` feature, rebuilt at HEAD `a9fa9af`).

---

## 1. Summary

| Metric | Value | Threshold | Pass |
|--------|------:|----------:|:----:|
| `composite`             | **1.0000** | ≥ 0.95 | ✓ |
| `context_assembly_f1`   | 1.0000 | ≥ 0.95 | ✓ |
| `guardrails_f1`         | 1.0000 | ≥ 0.95 | ✓ |
| `completion_f1`         | 1.0000 | ≥ 0.95 | ✓ |
| `layer_recall_f1`       | 1.0000 | ≥ 0.95 | ✓ |
| `tool_filter_accuracy`  | 1.0000 | ≥ 0.95 | ✓ |
| `total_queries`         | 29     | — | — |
| Wall-clock              | 171 ms | — | — |

Composite is the unweighted F1 average across the 4 dims plus the tool
filter accuracy gate.

## 2. Per-dimension breakdown

* **D1 context_assembly_f1 (1.0000)** — `<forge-context>` XML assembly
  produces exact string matches for all 29 query scenarios. Decisions /
  lessons / skills sections render in canonical order with no missing
  facets.
* **D2 guardrails_f1 (1.0000)** — All anti-pattern guardrail injections
  fire when expected and stay silent otherwise. Zero false-positive
  guardrails, zero false-negatives across the seed corpus.
* **D3 completion_f1 (1.0000)** — Active-protocols section completes for
  every query that should trigger one; absent for queries with no
  matching protocol.
* **D4 layer_recall_f1 (1.0000)** — Identity facet recall, decision
  recall, and lesson recall all hit ground truth top-K for every query.
* **Tool filter accuracy (1.0000)** — `<tools>` filtering against the
  mock 38-tool environment selects exactly the expected tool subset for
  every scenario.

## 3. Reproduction

```bash
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"
cargo build --release --features bench --bin forge-bench
./target/release/forge-bench forge-context \
    --seed 42 \
    --output bench_results_context_42
```

## 4. Comparison vs 2026-04-16 baseline

| Metric | 2026-04-16 | 2026-04-26 | Δ |
|--------|-----------:|-----------:|--:|
| composite              | 1.0000 | 1.0000 | 0 |
| context_assembly_f1    | 1.0000 | 1.0000 | 0 |
| guardrails_f1          | 1.0000 | 1.0000 | 0 |
| completion_f1          | 1.0000 | 1.0000 | 0 |
| layer_recall_f1        | 1.0000 | 1.0000 | 0 |
| tool_filter_accuracy   | 1.0000 | 1.0000 | 0 |

No regression detected across 10 days of P3-1 + P3-2 + P3-3 changes.

## 5. References

* Spec: `docs/benchmarks/forge-context-design.md`
* Plan: `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5 W1)
* Predecessor result: `docs/benchmarks/results/forge-context-2026-04-16.md`
* Implementation: `crates/daemon/src/bench/forge_context.rs`
* CLI subcommand: `crates/daemon/src/bin/forge-bench.rs::run_forge_context`
