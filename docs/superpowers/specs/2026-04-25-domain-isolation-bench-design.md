# Domain-Transfer Isolation Bench (2A-5) — Design v1

**Status:** DRAFT v1 — 2026-04-25. Awaits adversarial review.
**Phase position:** First sub-phase of P3-3.
**Predecessors:** master v6 / Forge-Identity bench precedent (2A-4d.3 shipped at `9aac8a8`).
**Successors:** 2A-6 multi-agent coordination bench (depends on isolation primitives).

---

## 1. Goal

**Validate cross-project memory leakage prevention as a measurable quality
dimension.** Today, the `memory.project` column scopes recall (via
`Request::Recall { project: Option<String>, ... }`) and the indexer/extractor
write project-tagged rows. This is a security/isolation boundary, but it
has no automated end-to-end check that a malformed query, a JOIN
regression, or a missing-WHERE-clause bug would catch.

**Before this work:**
- Project scoping is enforced via per-call `WHERE project = ?` in `recall::*`
  helpers (see `recall.rs` lines 147, 176, 208, 425, 698, 850, 879).
- Index `idx_memory_project` exists. Same for skill, domain_dna, perception,
  declared, entity, raw_documents.
- Unit tests in `recall.rs` cover happy-path project filtering, but a cross-
  project leakage probe (multi-project corpus, recall from each project,
  assert no foreign tokens) does not exist.

**After this work:**
- A new bench `forge-isolation` runs in-process per the Forge-Identity
  precedent. Generates N synthetic projects with project-specific tokens,
  seeds memories, recalls from each project, scores 5 dimensions covering
  precision (zero leakage), recall@K (own-project completeness), global
  visibility, and edge cases.
- The bench emits one `kpi_events` row per run with
  `event_type='bench_run_completed'` and `metadata_json.bench_name='forge-isolation'`,
  consumable by the `bench_run_summary` `/inspect` shape from Tier 3.
- A new `forge-bench forge-isolation` CLI subcommand mirrors the
  forge-identity / forge-context flag layout (`--seed`, `--output`, `--expected-composite`).
- The bench joins the CI matrix as the third in-process bench under the
  same `continue-on-error: true` rollout policy until 14 consecutive green
  master runs accumulate.

**Success metric:** a reviewer can answer "did this commit leak project X's
memories into project Y's recall?" by reading a single composite from the
last bench run.

---

## 2. Verified reconnaissance (2026-04-25, HEAD `479126e`)

| # | Fact | Evidence |
|---|------|----------|
| 1 | `memory.project TEXT` column with index `idx_memory_project ON memory(project)`. | `db/schema.rs:324, 332` |
| 2 | 7 tables carry `project TEXT` with per-table indexes: memory, skill, domain_dna, perception, declared, entity, raw_documents. | `db/schema.rs:206-572` |
| 3 | `Request::Remember` and `Request::Recall` both accept `project: Option<String>`. `BatchRecall` (via `RecallQuery`) does NOT carry project — bench can ignore for v1 or extend later. | `crates/core/src/protocol/request.rs:62-90` |
| 4 | `recall.rs` helpers consistently take `project: Option<&str>` and propagate it as a `WHERE project = ?` filter. 9 functions touch the project filter (lines 147, 176, 208, 425, 698, 850, 879, 2047, 2080). | direct grep |
| 5 | `compile_context(conn, agent, project)` at `recall.rs:2047` is the production project-scoped context-assembly entrypoint. | direct grep |
| 6 | Forge-Identity bench precedent uses ChaCha20-seeded determinism, 6 dimensions × per-dim minimums, composite ≥ 0.95 gate, 14 infrastructure assertions. | `crates/daemon/src/bench/forge_identity.rs` |
| 7 | Bench telemetry emit pattern: `crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed` opens short-lived rusqlite connection with WAL + busy_timeout, single INSERT, closes. No-op when `FORGE_DIR` unset. | `crates/daemon/src/bench/telemetry.rs` |
| 8 | `forge-bench` binary at `crates/daemon/src/bin/forge-bench.rs` dispatches by clap subcommand. Adding `forge-isolation` follows the existing pattern (~30-line clap variant + 3-line dispatch). | direct read |
| 9 | `bench_run_completed` events include `dimensions[].name` array; per-bench dim name registry pinned in `docs/architecture/events-namespace.md` (master v6 §M2). New bench requires a registry row. | `docs/architecture/events-namespace.md` |
| 10 | CI bench-fast matrix today: `[forge-consolidation, forge-identity]` with `continue-on-error: true`. Adding `forge-isolation` as a third matrix entry doubles bench-CI cost from 2 to 3 jobs (~60s each on ubuntu-latest). | `.github/workflows/ci.yml` |
| 11 | `bench/common.rs::seeded_rng(seed: u64) -> ChaCha20Rng` is the shared deterministic PRNG entrypoint (post-rand_chacha 0.10 bump). | `crates/daemon/src/bench/common.rs:11-13` |
| 12 | `bench/scoring.rs` exports composite scorer + per-dim weighted-mean helpers. | `crates/daemon/src/bench/scoring.rs` |

Planner re-verifies these at implementation time.

---

## 3. Architecture

### 3.1 Five dimensions

| Dim | Name | Probe | Min |
|-----|------|-------|-----|
| **D1** | `cross_project_precision` | For each project P in N projects: recall from P's context with `query = ""` (broad) AND `project = Some(P)`; assert ZERO foreign-project tokens in returned memories. Score = (1 − foreign_token_rate). Min 0.95 (zero false-positives is the goal; 0.95 leaves room for embedding-score collisions on synthetic dataset). | 0.95 |
| **D2** | `self_recall_completeness` | For each project P: recall from P with project-specific seed query; assert recall@K ≥ K_expected for K=10. Score = recall@10 averaged across N projects. Min 0.85. | 0.85 |
| **D3** | `global_memory_visibility` | Seed M memories with `project=None` (global). Recall from each project P; assert all M global memories appear in P's recall. Score = global_recall_rate. Min 0.90. | 0.90 |
| **D4** | `unscoped_query_breadth` | Recall with `project=None` (no filter); assert returned set spans all N+1 buckets (N projects + global). Score = bucket_coverage_rate (number of buckets with ≥1 hit / N+1). Min 0.85. | 0.85 |
| **D5** | `edge_case_resilience` | Three sub-probes: (a) empty-string project (`Some("")`) — must not match any memory with non-empty project; (b) special-char project name (`Some("p@#$%")`) — must not crash, returns own memories only; (c) project name 256 chars long — must not crash. Score = pass_rate across 3 probes. Min 0.85. | 0.85 |

**Composite:** weighted mean (D1 0.30, D2 0.20, D3 0.15, D4 0.15, D5 0.20).
Pass gate: composite ≥ 0.95 AND every dim ≥ min.

D1 weighted highest because zero-leakage is the safety-critical property.
D5 weighted second because edge cases catch real-world bugs.

### 3.2 Dataset generator

`bench/forge_isolation.rs::generate_corpus(rng: &mut ChaCha20Rng) -> Corpus`:

```
N = 5 synthetic projects: ["alpha", "beta", "gamma", "delta", "epsilon"]
Per-project memories: 30 (mix: 20 lessons + 10 decisions)
Global memories (project=None): 10 (mix: 6 patterns + 4 decisions)
Total: 5 × 30 + 10 = 160 memories
```

Each memory carries:
- `title`: `format!("{project}_secret_{idx}")` for project-tagged; `"global_pattern_{idx}"` for unscoped
- `content`: deterministic templated string with embedded project token
  (e.g., `"In project {project}, the {topic} pattern uses {detail}"`) so
  D1's foreign-token detection has a stable substring to grep for
- `tags`: `[project, "isolation_bench"]` for project-tagged; `["global", "isolation_bench"]` for unscoped
- `confidence`: `rng.random_range(0.7..0.99)`
- `metadata`: `{"bench": "forge-isolation", "seed_idx": idx}`

Embeddings use the shared bench-fixture deterministic embedder (matches
forge-identity precedent — `bench/common.rs::deterministic_embedding(seed_text)`).

### 3.3 Score formulas

```
D1 score = 1 - (foreign_tokens_returned / total_returned)
   averaged across N projects.

D2 recall@10 per project P:
   expected_set = {memory_id : memory.project == P AND
                                 query_token in memory.content}
   returned_set = first 10 of recall(query=project_token_query, project=Some(P))
   score_P = |expected_set ∩ returned_set| / min(10, |expected_set|)
   D2 = mean across projects.

D3 score = mean over projects P of:
   global_seen_count_P / total_global_count

D4 score = bucket_coverage:
   query = "isolation_bench" (a tag-shared marker)
   project = None
   returned = recall(query, project=None, limit=200)
   bucket_set = {memory.project for memory in returned}
   D4 = |bucket_set ∩ {None, "alpha", "beta", "gamma", "delta", "epsilon"}| / 6

D5 score = mean over 3 probes of:
   probe_pass: bool (no panic, no foreign tokens)
   D5 = pass_count / 3
```

### 3.4 Infrastructure assertions

7 fail-fast checks before dimensions run (mirrors forge-identity §6 pattern):

1. `memory_project_index_exists` — `PRAGMA index_info(idx_memory_project)` returns rows
2. `memory_project_column_exists` — `pragma_table_info('memory')` includes `project`
3. `recall_accepts_project_filter` — type-system trivially true; spot-checked at runtime by calling `Recall { project: Some("test_alpha"), ... }` and asserting result is `Ok`
4. `seeded_rng_deterministic` — same seed twice → same dataset
5. `corpus_size_matches_spec` — generated corpus has exactly `N×30 + 10 = 160` memories
6. `project_distribution_correct` — count by project_id == 30 each + 10 None
7. `embedding_dim_matches_recall_path` — generated embeddings match `recall.rs::EMBEDDING_DIM`

Any check failing → abort with summary failure (composite=0, pass=false).

### 3.5 Telemetry integration

Standard bench_run_completed emit at the tail of execution per Tier 3 §3.2.
No new event type. New `dimensions[].name` registry rows added to
`docs/architecture/events-namespace.md`:

```
bench_name: "forge-isolation"
dimensions:
  - cross_project_precision
  - self_recall_completeness
  - global_memory_visibility
  - unscoped_query_breadth
  - edge_case_resilience
```

### 3.6 CI integration

Add `forge-isolation` as a third matrix entry to `.github/workflows/ci.yml`'s
`bench-fast` job. Same `continue-on-error: true` rollout policy until 14
consecutive green master runs (T17 promotion gate from Tier 3 D4 covers
this bench too — promotion happens for the whole matrix at once).

---

## 4. Architecture decisions

- **D1 — Number of projects.** N=5. Rationale: enough to detect leakage
  patterns that 2 wouldn't (e.g., a bug where rows are returned in insertion
  order and the boundary is the second project's start); not so many that
  calibration runtime balloons. Each project gets 30 memories = 150 total +
  10 globals = 160 total — runs in <300ms on ubuntu-latest based on
  forge-identity comparison (1.0s for 6 dims × 5 seeds).
- **D2 — Recall mechanism.** Use `Request::Recall` directly via the
  in-process daemon helpers (same pattern as forge-identity). Don't spawn a
  daemon subprocess — keeps the bench fast and deterministic.
- **D3 — Embedding model.** Use the bench-fixture deterministic embedder
  (no fastembed call). Same precedent as forge-consolidation/identity to
  keep CI runtime sub-second.
- **D4 — Composite weighting.** D1 0.30, D2 0.20, D3 0.15, D4 0.15, D5 0.20.
  D1 highest because zero-leakage is the safety-critical property; D5
  second because edge cases find real bugs.
- **D5 — Pass gate.** Composite ≥ 0.95 AND every dim ≥ its min. Mirrors
  forge-identity master v6 §3 pattern.
- **D6 — Dim 5 edge cases.** v1 ships 3 probes (empty string, special chars,
  256-char name). Future probes (Unicode in project name, NULL via direct
  SQL, project in tags vs project column conflict) deferred.
- **D7 — Calibration target.** 1.0 composite on all 5 seeds before lock —
  same as forge-identity (3 cycles to reach 1.0 there). Plan for 2-3
  cycles; halt-and-flag at 5.

---

## 5. Out of scope

- **Cross-project recall API.** No new `Request::CrossProjectRecall` variant.
  v1 measures isolation; future work on intentional cross-project queries
  is a separate spec.
- **Project deletion / migration.** Bench doesn't probe DELETE or UPDATE
  paths.
- **Concurrent recall stress.** Single-thread, sequential recalls.
- **Skill / domain_dna / declared / entity / perception layers.** v1 probes
  only `memory` table layer. Other tables share the same `project` column
  pattern; if v1 finds bugs, v2 extends to other layers.
- **CrossProjectRecall request type.** Out of scope; isolation is a one-way
  property.
- **Network probes.** All in-process.
- **`raw_documents.project` separately.** Same column pattern as memory;
  if v1 catches the memory-layer leakage class, raw layer is by-construction
  covered. Defer to v2 if not.

---

## 6. Dependencies / blockers

* **LOCKED:** Forge-Identity bench precedent (master v6 + 2A-4d.3 shipped).
* **SHIPPED:** Tier 3 telemetry layer (`bench_run_completed` emit) +
  Tier 3 leaderboard surface (`bench_run_summary` `/inspect` shape).
* **No new schema.** Uses existing `memory.project` + `idx_memory_project`.
* **No new request variants.** Uses existing `Request::Remember` +
  `Request::Recall` with the existing `project: Option<String>` field.

---

## 7. Task breakdown

| Task | Description | Agent-friendly? |
|------|-------------|-----------------|
| **T1** | Re-verify the 12 recon facts at HEAD `479126e` (or whatever HEAD is current at impl time). | Yes — recon |
| **T2** | `crates/daemon/src/bench/forge_isolation.rs` skeleton: `IsolationScore` + `BenchConfig` + 5 dimension stubs returning `DimensionScore { name, score: 0.0, min, pass: false }` + composite scorer + corpus generator stub returning `Corpus { memories: vec![] }` + 7 infrastructure-assertion stubs. Integration test stub running the scorer on empty fixtures. | Yes |
| **T3** | Implement corpus generator (per §3.2). Adds `bench/forge_isolation/corpus.rs` if file size warrants. | Yes |
| **T4** | Implement D1 (cross_project_precision) + D2 (self_recall_completeness). | Yes |
| **T5** | Implement D3 (global_memory_visibility) + D4 (unscoped_query_breadth). | Yes |
| **T6** | Implement D5 (edge_case_resilience) + 7 infrastructure assertions. | Yes |
| **T7** | `forge-bench forge-isolation` CLI subcommand in `bin/forge-bench.rs` + argument plumbing (seed, output, expected-composite). | Yes |
| **T8** | Wire into `bench/telemetry.rs::emit_bench_run_completed` call path. Add `forge-isolation` row to `docs/architecture/events-namespace.md` per-bench dim registry. | Yes |
| **T9** | Calibration loop: run on 5 seeds, iterate until 1.0 composite (halt-and-flag at 5 cycles per locked decision). | Partially — interactive |
| **T10** | Adversarial review on T1-T9 diff (Claude general-purpose). | Yes |
| **T11** | Address review BLOCKER + HIGH; defer LOW with rationale. | Yes |
| **T12** | `.github/workflows/ci.yml` — add `forge-isolation` to `bench-fast` matrix with `continue-on-error: true`. | Yes |
| **T13** | Results doc at `docs/benchmarks/results/2026-04-XX-forge-isolation-stage1.md` mirroring forge-identity precedent. | Yes |
| **T14** | Close 2A-5: HANDOFF append, Stage 1 task complete, MEMORY index entry. | Yes |

**Critical path:** T1 → T2 → T3 → {T4, T5, T6 parallel-safe after T3} → T7 → T8 → T9 → T10 → T11 → T12 → T13 → T14.

**Estimated commits:** 8-12 (depends on calibration cycle count).

---

## 8. Open questions (v1 → v2 triggers)

1. **Embedding determinism for D2 recall@K.** `bench/common.rs::deterministic_embedding(seed_text)` is the assumed fixture. T1 verifies the function exists and returns a `Vec<f32>` of dim 384 (matches `RAW_EMBEDDING_DIM`). If the fixture isn't sufficient, T3 falls back to creating a `bench/forge_isolation/embeddings.rs` helper.
2. **Cross-project memory tagging via `tags` field.** A memory with `project=Some("alpha")` and `tags=vec!["beta_secret"]` could in principle leak into beta's recall via tag-text-search. v1 deliberately avoids this footgun by NOT putting project names in `tags`; v2 could add a Dim 6 tag-leakage probe.
3. **`raw_documents.project` separately.** Same column pattern as memory; if v1 catches the memory-layer leakage class, raw layer is by-construction covered. Defer to v2 if not.
4. **Bench wall-clock target.** Forge-identity at 1.0s; forge-isolation likely sub-second given 160 memories. Target ≤ 1.5s on ubuntu-latest. T1 measures; if exceeds 2s, demote to nightly.

---

## 9. Acceptance criteria

- [ ] All 5 dimensions land with non-zero implementations.
- [ ] Composite ≥ 0.95 on 5 seeds (calibration locked).
- [ ] 7 infrastructure assertions all pass on a fresh state.
- [ ] `forge-bench forge-isolation --seed 42` runs in < 1.5s on
      ubuntu-latest.
- [ ] `bench_run_completed` event emitted with
      `metadata_json.bench_name='forge-isolation'` and 5-element
      `dimensions[]` array.
- [ ] CI matrix includes the bench under `continue-on-error: true`.
- [ ] Adversarial review verdict `lockable` or `lockable-with-fixes`
      with all HIGH addressed.
- [ ] Results doc + events-namespace registry updated.
- [ ] `cargo clippy --workspace --features bench --tests -- -W clippy::all -D warnings` clean.

---

## 10. References

- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design.md` — bench harness precedent (v2 LOCKED).
- `docs/benchmarks/forge-identity-master-design.md` v6 — bench-internal pattern source.
- `docs/benchmarks/results/forge-consolidation-2026-04-17.md` — calibration / results-doc precedent.
- `docs/architecture/events-namespace.md` — `bench_run_completed` v1 contract + per-bench dim registry.
- `crates/daemon/src/bench/{common.rs, scoring.rs, telemetry.rs, forge_identity.rs}` — implementation precedent.
- `crates/daemon/src/recall.rs:147,176,208,2047` — project-scoped recall entrypoints.
- `crates/daemon/src/db/schema.rs:324-332` — memory.project column + index.

---

## Changelog

- **v1 (2026-04-25):** Initial draft. Author scoped to 5 dimensions (D1
  precision, D2 recall, D3 global, D4 breadth, D5 edge-cases) covering the
  isolation-correctness surface as observable from the existing
  `memory.project` column + `Request::Recall` API. Open questions flagged
  for adversarial review on v1.
