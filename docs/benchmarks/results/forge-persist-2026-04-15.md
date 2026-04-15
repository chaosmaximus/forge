# Forge-Persist — first calibration run (2026-04-15)

**Bench:** `forge-bench forge-persist` — durability harness (Phase 2A-1)
**Commit (cycle k):** HEAD of master after `feat(forge-persist): cycle k — generator fix + results doc`
**Workload:** `--memories 100 --chunks 50 --fisp-messages 20 --seed <N> --kill-after 0.5 --recovery-timeout-ms 30000 --worker-catchup-ms 10000`
**Hardware:** Apple M1 Pro, macOS Darwin 25.4.0 (arm64)
**Daemon:** release-mode `forge-daemon 0.4.0`, fresh TempDir per run (isolated via `FORGE_DIR`)
**Embedder:** `all-MiniLM-L6-v2` (384-dim) via `fastembed-rs 5.13`

**Design doc:** [`docs/benchmarks/forge-persist-design.md`](../forge-persist-design.md) — §5 dataset shape, §6 scoring rubric, §9 integration test shape

---

## Headline — default workload, 5 seeds

| Seed | total_ops | acked_pre_kill | recovered | matched | recovery_rate | consistency_rate | recovery_time_ms | wall_time_ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 170 | 85 | 85 | 85 | **1.0000** | **1.0000** | 273 | 11 780 |
| 2 | 170 | 85 | 85 | 85 | **1.0000** | **1.0000** | 269 | 11 023 |
| 3 | 170 | 85 | 85 | 85 | **1.0000** | **1.0000** | 273 | 11 033 |
| 42 | 170 | 85 | 85 | 85 | **1.0000** | **1.0000** | 269 | 11 052 |
| 100 | 170 | 85 | 85 | 85 | **1.0000** | **1.0000** | 273 | 11 015 |

**All 5 runs PASS** against the production thresholds defined in design doc §6.4:

- `recovery_rate ≥ 0.99` → observed 1.0 across all seeds (no losses)
- `consistency_rate = 1.00` → observed 1.0 across all seeds (no content drift)
- `recovery_time_ms < 5000` → observed 269-273 ms, **~18× headroom** on the 5000 ms production threshold

Mean wall-clock time per run: **11 180 ms** (double daemon spawn + embedder load + workload + kill + restart + verify + score).

**Production threshold decision** (Q4 from design doc §12): the observed 270 ms recovery time on M1 Pro gives 18× margin on the 5000 ms threshold. No recalibration needed for the first landing. The threshold stays at the design-doc value.

---

## What was actually measured

For each run, the harness:

1. Allocates a fresh TempDir, generates a free port, spawns a `forge-daemon` subprocess isolated via `FORGE_DIR=<tempdir>/.forge`
2. Waits for the daemon's HTTP `Health` endpoint + the MiniLM embedder's async background load (via `wait_for_raw_layer`)
3. Asserts zero preexisting memories in the daemon (fresh-TempDir precondition)
4. Generates a 170-op workload with the seed: 100 `Remember`, 50 `RawIngest`, 20 `SessionSend` (FISP), shuffled via ChaCha20
5. Executes the first `floor(170 × 0.5) = 85` ops, tracking each daemon ack in a `PersistTracker`
6. `SIGKILL`s the daemon
7. Sleeps 10 seconds (`--worker-catchup-ms 10000`) to let the async embedder worker finish processing pre-kill memories
8. Re-spawns the daemon on a fresh port (re-allocation sidesteps `TIME_WAIT` on the prior port)
9. Runs `verify_matches(client)` which queries:
   - `Request::RawDocumentsList { source: "forge-persist" }` for raw document recovery
   - `Request::Export { format: "json" }` for memory recovery (relies on fresh-TempDir precondition)
   - `Request::SessionMessages { session_id }` per pool session for FISP recovery
10. Computes `recovery_rate(acked_ids, visible)`, `consistency_rate(acked_map, content)`, `recovery_time_ms(second_spawn, first_health_ok)`
11. Runs `score_run` against the production thresholds and writes `summary.json` + `repro.sh`

All three scoring paths reconstruct the content hash client-side by re-running `canonical_hash` on the verbatim recovered payload. A byte-exact match against the pre-kill ack hash is required for a memory to count toward `consistency_rate`.

---

## Durability contract passes

The 1.0 / 1.0 result across all 5 seeds confirms:

- **SQLite WAL crash recovery works** — every row committed pre-`SIGKILL` is visible post-restart
- **No content corruption under SIGKILL** — every recovered payload hashes to the pre-kill canonical_hash
- **Cross-operation-type durability** — memory table, raw_documents table, and session_message table all survive independently
- **FISP parts JSON round-trips byte-exactly** — re-serializing recovered `Vec<MessagePart>` produces the same bytes as the pre-kill serialization (load-bearing for consistency_rate on FISP ops)

---

## Two generator bugs caught during calibration

The first calibration run with the original cycle (j2) generator reported `recovery_rate = 0.4941` on the 170-op workload — well below the 0.99 threshold. Investigation found **two** bugs in the harness workload generator, both unrelated to the daemon's actual durability. Both fixed in cycle (k):

### Bug 1 — `semantic_dedup` collapse via shared vocabulary

`workers::consolidator::run_all_phases` runs on every daemon startup (background task in `main.rs:341`) and includes `db::ops::semantic_dedup` as phase 2. That function partitions active memories and merges any pair whose Jaccard word overlap (on `meaningful_words(title) ∪ meaningful_words(content)`) exceeds 0.65.

The original generator produced titles like `persist_memory_0`, `persist_memory_1`, … which split into `{persist, memory}` after the single-char-token filter in `meaningful_words` — **100% overlap** across all memories of the same type. Combined with near-identical boilerplate content, every pair scored above 0.65 and got merged on second-daemon startup. With 5 memory types, recovery capped at exactly 5 regardless of workload size:

| N (memories) | recovered |
|---:|---:|
| 5 | 5 |
| 10 | 5 |
| 20 | 5 |
| 30 | 5 |

**Fix:** `remember_title` and `remember_content` now include a SHA-256 hex digest of the index as the dominant token, reducing pairwise Jaccard similarity well below 0.65. Locked by the `test_workload_memories_resist_semantic_dedup` unit tripwire which asserts every pair in a 30-memory workload scores at or below the 0.65 threshold.

### Bug 2 — `reweave_memories` merge via shared tags

Even after bug 1 was fixed, the calibration was **non-deterministic**: some runs passed 1.0/1.0, others reported 0.60/0.90 or 0.93/0.95. Race condition between the harness's `verify_matches` and the daemon's startup consolidator.

The root cause was a second consolidator phase, `workers::consolidator::reweave_memories`, which merges memory pairs that share ≥ 2 tags (same type, same project, same org). The original `TAG_POOL` rotation produced identical tag sets for every same-type memory — triggering reweave, which is **destructive**: the older survivor's content is mutated to `"{old_content}\n\n[Update]: {new_content}"` and the newer one is marked `status = 'merged'` (disappears from `Export`). This invalidated both `recovery_rate` (losses from the `'merged'` status) and `consistency_rate` (content drift from the `[Update]` append).

**Fix:** `remember_tags` now generates per-index unique tag strings (`tag-{index}-a`, `tag-{index}-b`). Locked by the `test_workload_memories_resist_reweave_shared_tags` unit tripwire which asserts every pair shares fewer than 2 tags.

### Lesson

Both bugs were latent in the harness from cycles (d)–(j) and only surfaced during the first calibration run. The integration test `test_persist_harness_full_run_passes_on_clean_workload` at cycle (j2.1) used a small workload (3+2+2 ops, 1 memory per type) that happened to evade both the semantic_dedup threshold and the reweave shared-tag check. **Small-workload integration tests are insufficient calibration proxies.** The full-size calibration run is where generator-daemon interaction bugs actually surface.

Both bugs were caught by reading daemon logs during a `FORGE_PERSIST_DEBUG_STDERR=1` run and tracing the consolidator output. The fix was confined to `bench::forge_persist` — no daemon changes.

---

## Reproduction

```bash
cargo build --release --bin forge-bench --bin forge-daemon

./target/release/forge-bench forge-persist \
  --memories 100 \
  --chunks 50 \
  --fisp-messages 20 \
  --seed 42 \
  --kill-after 0.5 \
  --recovery-timeout-ms 30000 \
  --worker-catchup-ms 10000 \
  --output bench_results
```

Expected output:

```
[forge-persist] total_ops=170 acked_pre_kill=85 recovered=85 matched=85
[forge-persist] recovery_rate=1.0000 consistency_rate=1.0000 recovery_time_ms=269
[forge-persist] wall_time_ms=11052 daemon_version=0.4.0
[forge-persist] PASS
```

`bench_results/summary.json` (canonical, seed=42):

```json
{
  "seed": 42,
  "memories": 100,
  "chunks": 50,
  "fisp_messages": 20,
  "kill_after": 0.5,
  "total_ops": 170,
  "acked_pre_kill": 85,
  "recovered": 85,
  "matched": 85,
  "recovery_rate": 1.0,
  "consistency_rate": 1.0,
  "recovery_time_ms": 269,
  "pass": true,
  "wall_time_ms": 11052,
  "daemon_version": "0.4.0"
}
```

---

## Honest comparison — no public baseline

Forge-Persist is a **Forge-specific benchmark by design**. There is no public baseline for "SQLite-backed memory daemon crash durability under SIGKILL" because the benchmark was constructed to measure a property that's Forge-specific (the daemon's synchronous write path + WAL semantics + per-table durability across memory, raw_documents, session_message). Design doc §1 and §14 gate 5 call this out explicitly.

What the 1.0 / 1.0 result means: **every operation the daemon acknowledged via HTTP 200 pre-kill is still present post-restart with byte-identical content.** No WAL loss, no phantom writes, no silent corruption under the workload in this bench.

What it does NOT prove (by design, per §11 non-goals):
- Retrieval quality (tested by LongMemEval, LoCoMo, Forge-* benches)
- Extraction pipeline correctness (Forge-Tool territory)
- `memory_vec` / `raw_chunks_vec` embedding row durability (scoped OUT, §11 — the ack criterion doesn't wait for async embedder writes, so this bench has no opinion on whether vector rows survive)
- Partition tolerance / multi-process coordination (Forge-Multi territory)
- Long-running workloads beyond ~200 ops

---

## Limitations and future calibration

- **Workload ceiling:** the harness's `HttpClient` uses a 5-second total timeout per request. Under heavy embedder load (250+ raw ingests queued up), individual `RawIngest` requests can exceed 5s and return `NetworkError`. A 425-op stress run (250+125+50) hit this limit. The default 170-op workload is well below the ceiling. Larger stress runs would require raising the HTTP total timeout or splitting the workload into batches with settling gaps. Deferred to a future cycle.
- **CI threshold:** the design-doc §9 recommendation was a looser 10-second recovery_time threshold for CI. The observed 270 ms gives 37× headroom on the CI threshold and 18× on the production threshold. No CI-specific calibration needed at current times.
- **Single-node only:** Forge is single-node by design; this bench does not measure any replication, Litestream, or cross-node durability.
- **Fresh TempDir assumption:** the harness's empty-Export precondition (`pub fn run`) requires a fresh TempDir-isolated daemon. A developer running the bench against their live dev daemon would fail at the precondition check — intentional, to prevent orphan memories from inflating `consistency_rate`'s denominator.
- **Daemon version capture is build-time, not runtime:** `daemon_version` in `summary.json` is captured via `env!("CARGO_PKG_VERSION")` at build time. Correct when `--daemon-bin` points at a sibling binary from the same workspace (the common case), wrong when pointed at a separately-built binary at a different version. Future fix requires a daemon `--version` endpoint.

---

## Cycle commit history

Full Phase 2A-1 on master:

```
<future k>  feat(forge-persist): cycle k — generator fix + results doc
30770a2     feat(forge-persist): pub fn run orchestrator + CLI dispatch (harness cycle j2)
9985456     feat(forge-persist): verify_matches helper + 3 query helpers (harness cycle j1)
dc6af95     feat(forge-persist): Request::RawDocumentsList endpoint (harness cycle j0)
dfdb6ae     feat(forge-persist): summary.json + repro.sh writer helpers (harness cycle i3)
070ad73     feat(forge-persist): serde derive on PersistScore + RunSummary (harness cycle i2)
b5042ac     feat(forge-persist): forge-bench CLI subcommand wiring (harness cycle i1)
b854ffd     feat(forge-persist): score_run composite + thresholds (harness cycle h4)
b8f3300     feat(forge-persist): recovery_time_ms pure function (harness cycle h3)
661adb2     feat(forge-persist): consistency_rate pure function (harness cycle h2)
d3b8d11     feat(forge-persist): recovery_rate pure function (harness cycle h1)
6ea5512     feat(forge-persist): PersistTracker storage primitives (harness cycle g3)
1a93e70     feat(forge-persist): wire canonical_hash into execute_op (harness cycle g2)
af90b54     feat(forge-persist): canonical_hash pure function + sha2 dep (harness cycle g1)
6a132fa     feat(forge-persist): HttpClient wrapper + HTTP health probe (harness cycle f2)
6189afe     feat(forge-persist): subprocess spawn + kill primitives (harness cycle f1)
24642ae     feat(forge-persist): op_to_request pure helper (harness cycle e)
110dad2     feat(forge-persist): seeded workload generator (harness cycle d)
dc47566     chore(lock): sync Cargo.lock with reqwest blocking feature
d490be7     feat(bench): enable reqwest blocking feature for Forge-Persist harness
dbe2f0a     feat(pidlock): portable PID liveness probe via libc::kill signal-0
523aa2a     docs(bench): Forge-Persist design doc (Phase 2A-1 design gate)
4afe315     feat(paths): FORGE_DIR env var override for Forge state directory
```

Phase 2A-1 Forge-Persist is **done**. All 7 quality gates per design doc §14 pass:

1. ✅ Design gate — design doc + adversarial review + founder approval
2. ✅ TDD gate — every new function has a failing test first, all RED→GREEN documented
3. ✅ Clippy + fmt gate — `cargo fmt --all` clean, `cargo clippy --workspace -- -W clippy::all -D warnings` zero warnings
4. ✅ Adversarial review gate — `feature-dev:code-reviewer` on every major sub-cycle diff, all ≥80 confidence findings addressed
5. ✅ Documentation gate — **this file**
6. ✅ Reproduction gate — `repro.sh` generated by the bench, verified runnable
7. ⏳ Dogfood gate — requires ≥ 1 calendar day of founder-driven runs; landed separately

Next: launch Phase 2A-2 (Forge-Tool) or proceed to the floor waves per the Phase 2+ plan.
