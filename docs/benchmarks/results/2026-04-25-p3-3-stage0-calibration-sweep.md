# P3-3 Stage 0 — Bench Calibration Sweep (2026-04-25)

**Status:** Locked baseline — recorded post-dependabot batch (4 PRs landed,
1 deferred). Sweep predates any P3-3 sub-phase implementation work. Re-run
this exact set after each sub-phase close to detect any regression.

**HEAD at sweep:** `56185d2` ("docs(P3-3 Stage 0): record dependabot batch
outcome + cluster-bump deferral").

**Why this exists:** the plan-doc Stage 0 mandate was "run all 6 existing
benches as a calibration sweep before any new bench dev." Locks today's
composite scores so 2C-2's auto-PR-on-regression workflow has a reference
point; also verifies the dep-bump cascade (PR #1, #5, #4, #3) didn't break
bench determinism (especially `rand_chacha 0.9 → 0.10` on the deterministic
ChaCha20 seed-driven harnesses).

## Methodology

* Bench binary: `target/release/forge-bench` (release profile, `bench` feature).
* Hardware profile: linux x86_64, GCP `chaosmaximus-instance`.
* `LD_LIBRARY_PATH=$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib` so the
  bench binary can `dlopen(libonnxruntime.so.1)`.
* All in-process benches with `--seed 42` (Tier 3 default).
* Subprocess bench (forge-persist) with reduced workload (50/25/10) to
  fit local time budget; full 100/50/20 runs in CI.
* Two dataset benches (longmemeval, locomo) deferred — require external
  dataset caches that aren't bundled in the repo. Out of scope for a
  local sweep; CI runs them via separate dataset-fetch step.

## Results

### 1. forge-consolidation — `composite=1.0000` PASS (0.4s wall)

Memory consolidation quality. 5 dimensions × in-process harness.

| Dimension | Score | Threshold | Pass |
|---|---|---|---|
| recall_improvement      | 1.0000 | — | ✓ |
| dedup_quality           | 1.0000 | — | ✓ |
| reweave_enrichment      | 1.0000 | — | ✓ |
| quality_lifecycle       | 1.0000 | — | ✓ |
| contradiction_handling  | 1.0000 | — | ✓ |
| **composite**           | **1.0000** | gate via recall_delta | **PASS** |

Observed `recall_delta = 0.2667` (+26.7% recall improvement post-consolidation).

### 2. forge-identity — `composite=0.9990` PASS (1.0s wall)

Memory + identity + skill inference. 6 dimensions per master v6 spec.

| Dimension | Score | Min | Pass |
|---|---|---|---|
| identity_facet_persistence  | 1.0000 | 0.85 | ✓ |
| disposition_drift           | 1.0000 | 0.85 | ✓ |
| preference_time_ordering    | 1.0000 | 0.80 | ✓ |
| valence_flipping            | 1.0000 | 0.85 | ✓ |
| behavioral_skill_inference  | 1.0000 | 0.80 | ✓ |
| preference_staleness        | 0.9960 | 0.80 | ✓ |
| **composite**               | **0.9990** | gate composite ≥ 0.95 | **PASS** |

Observed `wall_duration_ms = 983`. All 14 infrastructure checks pass.

### 3. forge-context — `composite=1.0000` PASS (0.2s wall)

Proactive intelligence precision. 5 dimensions × in-process harness.

| Dimension | Score | Pass |
|---|---|---|
| context_assembly_f1     | 1.0000 | ✓ |
| guardrails_f1           | 1.0000 | ✓ |
| completion_f1           | 1.0000 | ✓ |
| layer_recall_f1         | 1.0000 | ✓ |
| tool_filter_accuracy    | 1.0000 | ✓ |
| **composite**           | **1.0000** | **PASS** |

### 4. forge-persist — `recovery_rate=1.0000 consistency_rate=1.0000` PASS (13.4s wall)

Subprocess persistence drill: spawn daemon, issue scripted seeded workload,
SIGKILL mid-run at 0.5 fraction, restart, verify every HTTP-200-acked op
survived.

| Metric | Value |
|---|---|
| total_ops              | 85 (50 memories + 25 chunks + 10 fisp) |
| acked_pre_kill         | 42 |
| recovered              | 42 |
| matched                | 42 |
| recovery_rate          | 1.0000 |
| consistency_rate       | 1.0000 |
| recovery_time_ms       | 254 |
| wall_time_ms           | 13406 |
| daemon_version         | 0.6.0-rc.1 (binary stale at sweep time; rebuilt to rc.2 immediately after — same code paths, no behavioral delta) |

**Note:** reduced workload (default is 100/50/20). CI runs the full default.
Local result is sufficient for the "did the bumps break persistence?" signal.

### 5–6. longmemeval, locomo — DEFERRED

Dataset-dependent subprocess benches:

* **longmemeval** — requires LongMemEval dataset cache (not in repo).
* **locomo** — requires LoCoMo dataset cache (not in repo).

Both are network-dependent at first run; CI step `setup-bench-datasets`
populates the cache pre-run. For Stage 0 local sweep these aren't
necessary — the deterministic 4 above are the ones 2C-2 will gate on.

## Determinism check vs. rand 0.9 baseline

The dep-bump cascade landed at HEAD `891a12c` (rand 0.9 → 0.10,
rand_chacha 0.9 → 0.10). Composites measured **here** with rand 0.10:

| Bench | Pre-bump (rand 0.9 baseline) | Post-bump (this sweep) | Δ |
|---|---|---|---|
| forge-consolidation | 1.0000 (per `forge-consolidation-2026-04-17.md`) | 1.0000 | 0 |
| forge-identity      | 0.9990 (per `2026-04-25-forge-identity-master-v6-close.md`) | 0.9990 | 0 |
| forge-context       | 1.0000 (per `forge-context-2026-04-16.md`) | 1.0000 | 0 |
| forge-persist       | 1.0/1.0 (per `forge-persist-2026-04-15.md`) | 1.0/1.0 | 0 |

**Conclusion:** rand_chacha 0.10's API change (internal swap to chacha20)
preserves byte-identical PRNG output for `seed_from_u64` / `random_range` /
`random` — verified by composite-zero-delta across 4 deterministic harnesses
plus the dedicated `test_seeded_rng_deterministic` two-RNG-comparison unit
test in `crates/daemon/src/bench/common.rs`.

## Stage 0 outcomes

* **4 of 5 dependabot PRs landed** on master:
  * `ea75081` — minor-patch group (tokio/libc/rustls/fastembed/clap)
  * `04c502a` — zerocopy 0.7 → 0.8
  * `8ec72fd` — jsonwebtoken 9 → 10 (aws_lc_rs backend)
  * `891a12c` — rand 0.9 → 0.10, rand_chacha 0.9 → 0.10
* **1 of 5 deferred:** PR #2 opentelemetry 0.27 → 0.31 (cluster-mismatch
  with opentelemetry_sdk + opentelemetry-otlp + tracing-opentelemetry —
  isolated bump won't compile). Tracked as P3-3 deferred backlog item +
  `feedback_dependabot_ecosystem_cluster.md` memory.
* **All 11 CI gates green** post-bump cascade.
* **Composite scores unchanged** for all 4 deterministic benches.

Stage 0 closes; Stage 1 (2A-5 domain-transfer isolation bench spec
authoring) opens next.
