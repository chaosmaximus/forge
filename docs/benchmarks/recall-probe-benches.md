# Recall-probe benches — on-demand pattern

**Status:** Locked at v0.6.0-rc.3 (P3-3.5 W8).
**Scope:** the long-context / multi-document recall benchmarks
(`longmemeval`, `locomo`) that depend on **external datasets** and are
therefore deliberately excluded from CI.

## Why these benches aren't in CI

The CI bench-fast matrix today runs the **6 in-process** benches:

```
[forge-consolidation, forge-identity, forge-context,
 forge-isolation, forge-coordination, forge-persist]
```

These are deterministic, self-contained, and complete in seconds.

`longmemeval` and `locomo` are different:

* They expect a **multi-GB external dataset** (LongMemEval cleaned
  corpus, LoCoMo conversations) cloned into a known on-disk location.
* Per-bench wall-clock is on the order of 10-60 minutes, depending on
  recall@K mode.
* They benchmark **embedding quality + retrieval recall**, not
  consolidation logic; running them on every PR would 10×+ the CI
  budget for marginal signal.

So CI runs the structural benches; recall-probe benches run **on
demand**, at release boundaries, and after breaking-change-class
commits.

## Where the datasets live

| Dataset | Source | Approx. size |
|---------|--------|-------------:|
| LongMemEval cleaned | upstream community fork (cleaned for licence + dedup); locally pinned at `~/.forge-datasets/longmemeval/` | ~2 GB |
| LoCoMo | `snap-research/locomo` GitHub repo + companion data drop | ~600 MB |

Datasets are **not** committed to this repo. The on-demand procedure
expects an operator to have the local copy or a writable cache.

## On-demand invocation

### LongMemEval

```bash
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"

forge-bench longmemeval \
    --mode recall_at_5 \
    --path ~/.forge-datasets/longmemeval/cleaned.json \
    --seed 42 \
    --output bench_results_longmemeval_recall5

# Modes: recall_at_5, recall_at_10, recall_at_20
```

### LoCoMo

```bash
forge-bench locomo \
    --mode recall_at_10 \
    --path ~/.forge-datasets/locomo/ \
    --seed 42 \
    --output bench_results_locomo_recall10
```

Both subcommands emit `bench_run_completed` events with the same
schema as the in-process benches (per `events-namespace.md` per-bench
dim registry: longmemeval / locomo are 0-dimension probes whose
composite = `mean_recall_at_K`).

## Last-runs reference

The most recent reference run is captured in:

* [`results/longmemeval-2026-04-13.md`](results/longmemeval-2026-04-13.md)
* [`results/locomo-2026-04-13.md`](results/locomo-2026-04-13.md)

These are 13 days stale at the v0.6.0-rc.3 cut. Per user-locked
decision (2026-04-26), datasets aren't available in this environment
yet, so recall-probe re-run is **deferred to v0.6.1+**.

## Recalibration cadence

| Trigger | Action |
|---------|--------|
| Per-release boundary | Re-run both benches; update results docs; document any drift > 5% |
| Embedding model swap (e.g., MiniLM → newer) | Re-run + adversarial review; update composites.json if drift is intentional |
| Dataset upgrade | Re-run + capture in a new dated results doc; old doc preserved for trend |
| Daemon recall-path refactor | Re-run before merging; flag a recall-regression issue if drift > 5% |

When a recall-probe re-run is **expected to drift** because of a
planned daemon change, document the expected magnitude in the commit
message before running, then capture the actual delta in the results
doc.

## Why no `composites.json` floor

The `composites.json` baselines registry intentionally **excludes**
longmemeval/locomo. Reasons:

* They depend on dataset versions that aren't pinned in this repo,
  so the floor isn't deterministic without a paired dataset hash.
* The headline metric is recall@K — already heavily seed/dataset-dependent.

Treat the latest results doc as the de-facto baseline; do not gate CI
on it.

## Backlog (deferred)

* **Pin dataset hashes** alongside the bench invocation so a "fresh
  vs prior" pairwise comparison becomes meaningful. Out of scope for
  v0.6.0.
* **CI on-demand mode** (workflow_dispatch input that runs these on
  ubuntu-latest + the GH-hosted-runners with attached LFS dataset).
  Out of scope for v0.6.0.

## Related

* Plan: [`../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.5 W8)
* Bench design specs: `docs/benchmarks/longmemeval-design.md`, `docs/benchmarks/locomo-design.md` (if present)
* Events registry: [`../architecture/events-namespace.md`](../architecture/events-namespace.md) (longmemeval / locomo rows)
* In-process bench docs: `2026-04-26-forge-{consolidation,context,persist}-pre-release.md`
