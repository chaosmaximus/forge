# Domain-Transfer Isolation Bench (2A-5) — Design v2

**Status:** DRAFT v2 — 2026-04-25. Addresses v1 adversarial review (3 BLOCKER + 3 HIGH + 4 MEDIUM + 3 LOW). Awaits second review.
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
- `compile_context(conn, agent, project)` at `recall.rs:2047` is the
  user-facing context-assembly entrypoint orchestrating 9+ project-scoped
  helpers (search_skills, search_declared, list_entities, perceptions,
  graph neighbors, decisions/lessons SQL).
- Unit tests in `recall.rs` cover happy-path project filtering, but a
  cross-project leakage probe (multi-project corpus, recall from each
  project, assert no foreign tokens) does not exist.

**After this work:**
- A new bench `forge-isolation` runs in-process per the Forge-Identity
  precedent. Generates N synthetic projects with project-specific tokens,
  seeds memories, recalls via both `Request::Recall` and `compile_context`,
  scores 6 dimensions covering precision (zero-leakage in Recall **and**
  compile_context), recall@K (own-project completeness), global visibility,
  unscoped breadth, and edge-case resilience.
- The bench emits one `kpi_events` row per run with
  `event_type='bench_run_completed'` and `metadata_json.bench_name='forge-isolation'`,
  consumable by the `bench_run_summary` `/inspect` shape from Tier 3.
- A new `forge-bench forge-isolation` CLI subcommand mirrors the
  forge-identity / forge-context flag layout (`--seed`, `--output`, `--expected-composite`).
- The bench joins the CI matrix as the third in-process bench under the
  same `continue-on-error: true` rollout policy until 14 consecutive green
  master runs accumulate.

**Success metric:** a reviewer can answer "did this commit leak project X's
memories into project Y's recall **OR** project Y's `compile_context`
output?" by reading a single composite from the last bench run.

---

## 2. Verified reconnaissance (2026-04-25, HEAD `aa14763`)

| # | Fact | Evidence |
|---|------|----------|
| 1 | `memory.project TEXT` column with index `idx_memory_project ON memory(project)`. | `db/schema.rs:324, 332` |
| 2 | 7 tables carry `project TEXT` with per-table indexes: memory, skill, domain_dna, perception, declared, entity, raw_documents. | `db/schema.rs:206-572` |
| 3 | `Request::Remember` and `Request::Recall` both accept `project: Option<String>`. `BatchRecall` (via `RecallQuery`) does NOT carry project — bench can ignore for v1 or extend later. | `crates/core/src/protocol/request.rs:62-90` |
| 4 | `recall.rs` helpers consistently take `project: Option<&str>` and propagate it as a `WHERE project = ?` filter. 9 functions touch the project filter (lines 147, 176, 208, 425, 698, 850, 879, 2047, 2080). | direct grep |
| 5 | `compile_context(conn, agent, project)` at `recall.rs:2047` is the production project-scoped context-assembly entrypoint. Calls 9+ project-scoped helpers across memory, skill, declared, domain_dna, entity, perception layers. | direct grep |
| 6 | Forge-Identity bench precedent uses ChaCha20-seeded determinism, 6 dimensions × per-dim minimums, composite ≥ 0.95 gate, 14 documented infrastructure assertions (per module doc); the underlying source has 37 `check_*` helper functions (broader than the doc count). | `crates/daemon/src/bench/forge_identity.rs` |
| 7 | Bench telemetry emit pattern: `crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed` opens short-lived rusqlite connection with WAL + busy_timeout, single INSERT, closes. No-op when `FORGE_DIR` unset. | `crates/daemon/src/bench/telemetry.rs` |
| 8 | `forge-bench` binary at `crates/daemon/src/bin/forge-bench.rs` dispatches by clap subcommand. Adding `forge-isolation` follows the existing pattern (~30-line clap variant + 3-line dispatch). | direct read |
| 9 | `bench_run_completed` events include `dimensions[].name` array; per-bench dim name registry pinned in `docs/architecture/events-namespace.md` (master v6 §M2). New bench requires a registry row. | `docs/architecture/events-namespace.md` |
| 10 | CI bench-fast matrix today: `[forge-consolidation, forge-identity]` with `continue-on-error: true`. Adding `forge-isolation` as a third matrix entry adds ~60s to the bench-CI wall-clock (single ubuntu-latest job). | `.github/workflows/ci.yml` |
| 11 | `bench/common.rs::seeded_rng(seed: u64) -> ChaCha20Rng` is the shared deterministic PRNG entrypoint (post-rand_chacha 0.10 bump). | `crates/daemon/src/bench/common.rs:11-13` |
| 12 | **Corrected from v1.** `bench/scoring.rs` exports `recall_any_at_k`, `recall_all_at_k`, `ndcg_at_k`, `dcg_from_rels` only. Composite-mean logic is private at `forge_identity.rs:1632 composite_score(&[DimensionScore; 6])` and hardcoded for 6 dims. **T2.2 lifts a generalized `composite_score(dims: &[DimensionScore], weights: &[f64]) -> f64` to `scoring.rs`; forge-identity is updated to call the lifted version.** | `crates/daemon/src/bench/scoring.rs` + `forge_identity.rs:1632` |
| 13 | **Corrected from v1.** Deterministic embedder is `pub fn generate_base_embedding(seed_key: &str) -> Vec<f32>` at `forge_consolidation.rs:1687`, dim is the local module constant `EMBEDDING_DIM` (currently a 768-dim consolidation-tuned vector — verify at T1). **T2.1 lifts this to `bench/common.rs::deterministic_embedding(seed_key: &str) -> Vec<f32>` and re-exports the original name from forge_consolidation for backward compat.** | `forge_consolidation.rs:1684-1729` |
| 14 | `Request::Recall` BM25 SQL at `db/ops.rs:691-704` uses `m.project = ?2 OR m.project IS NULL OR m.project = ''` — empty-string project filter matches the global pool (NULL/empty), NOT non-empty foreign projects. D5(a) probe redefined accordingly (v1 spec inverted this). | `db/ops.rs:691-704` |
| 15 | `Request::Recall` BM25 sanitization at `db/ops.rs:569-571` short-circuits empty queries to `Ok(Vec::new())`. v1 D1 used `query=""` and trivially scored 1.0; v2 uses the shared `"isolation_bench"` tag query (every bench memory shares this tag) so D1 returns the expected superset and the foreign-token denominator is meaningful. | `db/ops.rs:569-571` |
| 16 | `Recall` request at `request.rs:556` accepts `organization_id: Option<String>`; org filter at `db/ops.rs:691-694` adds `COALESCE(organization_id, 'default') = ?4` when present. v1 bench was implicit `None` (everything in `default` org). **v2 explicitly defers org-isolation to a future bench (§5 disclaimer); all bench memories share `organization_id = None`.** | `request.rs:556` + `db/ops.rs:691-694` |

Planner re-verifies these at implementation time. T1 **also** greps `recall_raw_*` to confirm `raw_documents.project` predicate is structurally identical (v2 L2 follow-up).

---

## 3. Architecture

### 3.1 Six dimensions

| Dim | Name | Probe | Min | Weight |
|-----|------|-------|-----|--------|
| **D1** | `cross_project_precision` | For each project P in N projects: `Recall { query: "isolation_bench", project: Some(P), limit: 200 }` (the shared tag). Foreign-token denominator excludes global memories (which legitimately appear from any project). Score = 1 − (foreign_tokens_observed / max_possible_foreign). Min 0.95. | 0.95 | 0.25 |
| **D2** | `self_recall_completeness` | For each project P: `Recall { query: "{project}_secret", project: Some(P), limit: 50 }`. Score = recall@10 averaged across N projects. Min 0.85. | 0.85 | 0.15 |
| **D3** | `global_memory_visibility` | Seed M=10 memories with `project=None` (global). For each project P, recall and assert all M global memories appear. Score = global_recall_rate averaged across projects. Min 0.90. | 0.90 | 0.10 |
| **D4** | `unscoped_query_breadth` | `Recall { query: "isolation_bench", project: None, limit: 200 }`. Returned set must span all N+1 buckets (N projects + global). Score = `bucket_coverage / (N+1)`. Min 0.85. | 0.85 | 0.10 |
| **D5** | `edge_case_resilience` | 7 sub-probes (see §3.1a). Score = pass_count / 7. Min 0.85. | 0.85 | 0.15 |
| **D6** | `compile_context_isolation` | **NEW per H1 fix.** For each project P: `compile_context(conn, "isolation_bench_agent", Some(P))` returns full XML. Search the XML for **foreign-project tokens** (e.g., `"alpha_secret_"`, `"beta_secret_"`, ...). Score = 1 − (foreign_tokens_found / max_possible). Min 0.95. v1 covers only the memory-layer portion of compile_context (skills/declared/etc layers deferred to v2 — see §5). | 0.95 | 0.25 |

**Composite:** weighted mean across the 6 dims (weights sum to 1.00).
**Pass gate (dual):** composite ≥ 0.95 AND every dim ≥ its min. The dual
gate ensures D1 = 0.84 alone (composite 0.946 with others perfect) does not
sneak past — the per-dim min catches it. Math worked through in
v1 review Q1 response.

D1 + D6 weighted equal at 0.25 each because both audit zero-leakage at the
two user-facing surfaces (Recall API and compile_context output). D5
weighted 0.15 second because edge-case bugs surface real production issues.

### 3.1a D5 — 7 sub-probes

Per H3 fix, expanded from 3 to 7 probes:

1. **`empty_string_targets_global`** — `Recall { project: Some("") }` returns ONLY global-pool memories (NULL or empty `project`); no project-scoped memories. Pass = no `{x}_secret_` tokens from any non-global project in result.
2. **`special_chars_no_panic`** — `Recall { project: Some("p@#$%") }` does not panic. Returns own (zero) memories. Pass = result is `Ok(Vec::new())` or `Ok(<global only>)`.
3. **`overlong_project_no_panic`** — 256-char project name. Pass = result is `Ok(...)` (any shape).
4. **`sql_injection_inert`** — `Recall { project: Some("alpha'; DROP TABLE memory;--") }` does not drop the table. Pass = `memory` table row count post-call equals pre-call count.
5. **`prefix_collision_isolated`** — Seed an extra project `"alphabet"` with 5 memories. `Recall { project: Some("alpha") }` returns only `alpha`'s memories, not `alphabet`'s. Pass = no `alphabet_secret_` tokens in result.
6. **`case_sensitivity_strict`** — `Recall { project: Some("ALPHA") }` returns 0 memories (assuming SQL `=` is case-sensitive in SQLite default config); not the lowercase `"alpha"` corpus. Pass = result excludes `alpha_secret_` tokens.
7. **`trailing_whitespace_strict`** — `Recall { project: Some(" alpha") }` returns 0 memories. Pass = excludes `alpha_secret_` tokens.

7 probes × `pass_count / 7` scoring: single failure = 14% drop (still ≥ 0.85 min, robust to one regression).

### 3.2 Dataset generator

`bench/forge_isolation.rs::generate_corpus(rng: &mut ChaCha20Rng) -> Corpus`:

```
Projects (6 total — N=5 + 1 prefix-collision sentinel):
  ["alpha", "beta", "gamma", "delta", "epsilon", "alphabet"]
  ("alphabet" exists ONLY for D5 probe 5; not part of D1-D4 N-counting)

Per-project memories (the 5 main projects):
  30 each (mix: 20 lessons + 10 decisions)
  Title: format!("{project}_secret_{idx}")
  Content: deterministic templated string with embedded project token
    e.g., "In project {project}, the {topic} pattern uses {detail}"
  Tags: vec![project, "isolation_bench"]
  Confidence: 0.7 + (idx as f32 * 0.01).clamp(0.0, 0.29)  // v2 M4 fix —
    deterministic-integer-derived; avoids rand_range sampling edge.
  Metadata: {"bench": "forge-isolation", "seed_idx": idx}

Prefix-collision sentinel ("alphabet"):
  5 memories with title "alphabet_secret_0..4"

Global memories (project=None):
  10 (mix: 6 patterns + 4 decisions)
  Title: format!("global_pattern_{idx}")
  Tags: vec!["global", "isolation_bench"]
  Confidence: deterministic per above
  Metadata: {"bench": "forge-isolation", "seed_idx": idx, "scope": "global"}

Total: 5 × 30 + 5 + 10 = 165 memories
```

Embeddings use the **lifted** `bench::common::deterministic_embedding(seed_key)`
(per T2.1 + B1 fix). Dim matches `forge_consolidation::EMBEDDING_DIM` (768
at recon time; T1 verifies).

### 3.3 Score formulas

```text
D1 score = 1 − (foreign_tokens_returned / max_possible_foreign)
   averaged across N main projects.
   foreign-token set excludes global memories (legitimately recallable
   from any project) but includes the alphabet-sentinel memories when
   probing alpha/beta/etc.

D2 recall@10 per project P:
   expected_set = {memory_id : memory.project == P AND
                                 query_token in memory.content}
   returned_set = first 10 of recall(query="{P}_secret",
                                       project=Some(P), limit=50)
   score_P = |expected_set ∩ returned_set| / min(10, |expected_set|)
   D2 = mean across projects.

D3 score = mean over projects P of:
   global_seen_count_P / total_global_count

D4 score = bucket_coverage:
   query = "isolation_bench"
   returned = recall(query, project=None, limit=200)
   bucket_set = {memory.project for memory in returned}
   D4 = |bucket_set ∩ {None, "alpha", "beta", "gamma", "delta", "epsilon"}| / 6

D5 score = pass_count / 7 (per §3.1a)

D6 score per project P:
   xml = compile_context(&conn, "isolation_bench_agent", Some(P))
   foreign_tokens = sum(occurrences(xml, "{Q}_secret_") for Q in projects if Q != P)
   max_possible = (N-1) × 30 (other-projects' memory count)
   score_P = 1 − (foreign_tokens / max_possible)
   D6 = mean across projects.
   Note: covers memory-layer portion of compile_context only;
   skills/declared/perception layers deferred to v2 (§5).

Composite = 0.25*D1 + 0.15*D2 + 0.10*D3 + 0.10*D4 + 0.15*D5 + 0.25*D6
```

### 3.4 Infrastructure assertions

8 fail-fast checks (was 7 in v1; added D6 fixture check). Fewer than
forge-identity's documented 14 because the isolation surface is smaller —
only memory.project + idx_memory_project + corpus shape + compile_context
fixture need pre-flight.

1. `memory_project_index_exists` — `PRAGMA index_info(idx_memory_project)` returns rows
2. `memory_project_column_exists` — `pragma_table_info('memory')` includes `project`
3. `recall_accepts_project_filter` — type-system trivially true; runtime check via `Recall { project: Some("test_alpha"), ... }` returns `Ok`
4. `seeded_rng_deterministic` — same seed twice → same dataset
5. `corpus_size_matches_spec` — generated corpus has exactly `5×30 + 5 + 10 = 165` memories
6. `project_distribution_correct` — count by project_id == 30 each (main 5) + 5 (alphabet) + 10 None
7. `embedding_dim_matches_consolidation` — lifted embedder returns dim matching `forge_consolidation::EMBEDDING_DIM`
8. `compile_context_returns_xml` — `compile_context(&conn, "test_agent", Some("test_proj"))` returns non-empty `String` containing `<context>`

Any check failing → abort with summary failure (composite=0, pass=false).

### 3.5 Telemetry integration

Standard `bench_run_completed` emit at the tail of execution per Tier 3 §3.2.
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
  - compile_context_isolation
```

### 3.6 CI integration

Add `forge-isolation` as a third matrix entry to `.github/workflows/ci.yml`'s
`bench-fast` job. Same `continue-on-error: true` rollout policy until 14
consecutive green master runs (T17 promotion gate from Tier 3 D4 covers
this bench too — promotion happens for the whole matrix at once).

Adds ~60s to bench-CI wall-clock; no impact on the 15-min total CI budget.

### 3.7 Single shared corpus per seed (M1 fix)

**Mandate:** all 6 dims read from a **single shared `DaemonState`** seeded
with the corpus once per `--seed` invocation. Per-dim isolated `:memory:`
DBs (as in `forge_identity.rs:1703 run_dim_isolated`) actively HIDE
cross-dim project leakage because each dim sees a fresh slate. Forge-identity's
isolation pattern is appropriate for *its* property-testing surface; for
forge-isolation it's the wrong primitive.

Implementation: `run_bench(seed)` builds one `DaemonState`, calls
`seed_corpus(&mut state, &corpus)`, then runs D1..D6 sequentially against
that state. `infrastructure_checks` runs first against the same state.

---

## 4. Architecture decisions

- **D1 — Number of projects.** N=5 main + 1 prefix-collision sentinel = 6
  total project IDs. Per-project memories: 30 (main) + 5 (sentinel) + 10
  globals = 165 total. Runs in <500ms based on forge-identity comparison
  (1.0s for 6 dims × per-dim isolation; we reuse one DaemonState so it
  should be faster despite 1 extra dim).
- **D2 — Recall mechanism.** Use `Request::Recall` directly via the
  in-process daemon helpers (same pattern as forge-identity). Don't spawn a
  daemon subprocess — keeps the bench fast and deterministic.
- **D3 — Embedding model.** Use the lifted `bench::common::deterministic_embedding`
  (T2.1). Same precedent as forge-consolidation/identity to keep CI runtime
  sub-second.
- **D4 — Composite weighting.** D1 0.25, D2 0.15, D3 0.10, D4 0.10,
  D5 0.15, D6 0.25. D1 + D6 highest because they audit zero-leakage at the
  two user-facing surfaces; D5 second because edge cases catch real bugs.
- **D5 — Pass gate.** Composite ≥ 0.95 AND every dim ≥ its min (dual gate).
- **D6 — D5 edge cases.** v2 ships 7 probes (empty string redefined per H2,
  special chars, 256-char name, SQL injection, prefix collision, case
  sensitivity, trailing whitespace). v3+ extends with Unicode RTL/combining
  chars + project-name-in-tags bypass.
- **D7 — Calibration target.** 1.0 composite on all 5 seeds before lock —
  same as forge-identity (3 cycles to reach 1.0 there). Plan for 2-3 cycles;
  halt-and-flag at 5.
- **D8 — Single shared DaemonState (§3.7).** Mandatory for leakage-detection
  signal preservation.
- **D9 — `organization_id` deferred.** All bench memories share
  `organization_id = None` (default org). Cross-org isolation is a separate
  property and gets its own future bench. v1 bench does NOT exercise the
  `?4` org-filter SQL path.
- **D10 — `compile_context` partial coverage.** D6 audits the memory-layer
  rendering inside compile_context. Skills, declared, domain_dna,
  perception, entity layers contribute their own subtrees to the XML; v1
  does not seed those tables, so leakage in their helpers is not caught.
  Documented disclaimer in §5; v2 extends with a multi-table corpus.

---

## 5. Out of scope (with explicit disclaimers)

- **Cross-project recall API.** No new `Request::CrossProjectRecall` variant.
  v1 measures isolation; intentional cross-project queries are a separate
  spec.
- **Project deletion / migration.** Bench doesn't probe DELETE or UPDATE
  paths.
- **Concurrent recall stress.** Single-thread, sequential recalls.
- **Skill / domain_dna / declared / entity / perception layers.** v1 probes
  only `memory` table layer + the memory-layer portion of `compile_context`.
  A leakage bug in `search_skills`, `list_unconsumed_perceptions`, or the
  graph-neighbor helpers would NOT be caught by v1. v2 extends the corpus
  to seed these tables.
- **CrossProjectRecall request type.** Out of scope; isolation is a one-way
  property.
- **Network probes.** All in-process.
- **`raw_documents.project` separately.** Same column pattern as memory
  (recon T1 verifies the predicate is `project = ? OR project IS NULL OR
  project = ''` — structurally identical). If v1 catches the memory-layer
  leakage class, raw layer is by-construction covered for the shared SQL
  shape, but raw-specific helpers (e.g., `recall_raw_chunks_bm25`) are not
  exercised.
- **`organization_id` cross-org isolation (per D9).** v1 explicitly leaves
  `organization_id = None` on all bench memories. Cross-org leakage is a
  separate concern and gets its own future bench. Disclaimer documented;
  no v1 coverage.
- **Tag-substring leakage.** `tags` is part of `memory_fts` (schema.rs:340 —
  INSERT triggers tag column). A query containing a project name as
  substring would match foreign-project memories with that name in tags.
  v1 sidesteps this footgun by NOT putting project names in tags (only
  the project's own name and the shared `"isolation_bench"` tag). v2 must
  add a tag-sanitization probe and may need to drop tags from FTS.

---

## 6. Dependencies / blockers

* **LOCKED:** Forge-Identity bench precedent (master v6 + 2A-4d.3 shipped).
* **SHIPPED:** Tier 3 telemetry layer (`bench_run_completed` emit) +
  Tier 3 leaderboard surface (`bench_run_summary` `/inspect` shape).
* **No new schema.** Uses existing `memory.project` + `idx_memory_project`.
* **No new request variants.** Uses existing `Request::Remember` +
  `Request::Recall` + `compile_context()` with the existing
  `project: Option<String>` field.
* **T2.1 prerequisite (B1 fix):** lift `generate_base_embedding` from
  `forge_consolidation.rs` to `bench/common.rs` as `deterministic_embedding`,
  re-export the original name from forge_consolidation for backward compat.
* **T2.2 prerequisite (B2 fix):** lift `composite_score` from
  `forge_identity.rs:1632` to `bench/scoring.rs` with N-dim signature
  `composite_score(dims: &[DimensionScore], weights: &[f64]) -> f64`.
  Update forge-identity to call the lifted version with its 6-tuple weights.

---

## 7. Task breakdown

| Task | Description | Agent-friendly? |
|------|-------------|-----------------|
| **T1** | Re-verify the 16 recon facts at HEAD (whatever HEAD is current at impl time). **Specifically grep `recall_raw_*` to confirm `raw_documents.project` predicate is structurally identical (v2 L2 follow-up).** Also verify `forge_consolidation::EMBEDDING_DIM` value at impl time. | Yes — recon |
| **T2.1** | Lift `generate_base_embedding` from `forge_consolidation.rs:1687` to `bench/common.rs::deterministic_embedding(seed_key: &str) -> Vec<f32>`. Re-export the original name from forge_consolidation (no caller churn). Tests: byte-identical output for same input pre/post lift. | Yes |
| **T2.2** | Lift `composite_score` from `forge_identity.rs:1632` to `bench/scoring.rs::composite_score(dims: &[DimensionScore], weights: &[f64]) -> f64` with debug_assert weights.len() == dims.len() && (sum(weights) − 1.0).abs() < 1e-9. Update forge-identity to call lifted fn with hardcoded 6-tuple weights. Tests: forge-identity composite unchanged byte-for-byte. | Yes |
| **T2.3** | `crates/daemon/src/bench/forge_isolation.rs` skeleton: `IsolationScore` + `BenchConfig` + 6 dimension stubs returning `DimensionScore { name, score: 0.0, min, pass: false }` + composite scorer call site (uses lifted T2.2) + corpus generator stub returning `Corpus { memories: vec![] }` + 8 infrastructure-assertion stubs. Integration test stub running scorer on empty fixtures. **§3.7 mandate: single shared `DaemonState` per seed (no per-dim isolation).** | Yes |
| **T3** | Implement corpus generator (per §3.2). 165 memories, deterministic confidence (no rand_range — M4 fix). Adds `bench/forge_isolation/corpus.rs` if file size warrants. | Yes |
| **T4** | Implement D1 (cross_project_precision — §3.1 query is `"isolation_bench"` tag, not empty — B3 fix) + D2 (self_recall_completeness). | Yes |
| **T5** | Implement D3 (global_memory_visibility) + D4 (unscoped_query_breadth) + D6 (compile_context_isolation — H1 fix). | Yes |
| **T6** | Implement D5 (edge_case_resilience — 7 probes per §3.1a + H2/H3 fixes) + 8 infrastructure assertions. | Yes |
| **T7** | `forge-bench forge-isolation` CLI subcommand in `bin/forge-bench.rs` + argument plumbing (seed, output, expected-composite). | Yes |
| **T8** | Wire into `bench/telemetry.rs::emit_bench_run_completed` call path. Add `forge-isolation` row to `docs/architecture/events-namespace.md` per-bench dim registry. | Yes |
| **T9** | Calibration loop: run on 5 seeds, iterate until 1.0 composite (halt-and-flag at 5 cycles per locked decision). | Partially — interactive |
| **T10** | Adversarial review on T1-T9 diff (Claude general-purpose). | Yes |
| **T11** | Address review BLOCKER + HIGH; defer LOW with rationale. | Yes |
| **T12** | `.github/workflows/ci.yml` — add `forge-isolation` to `bench-fast` matrix with `continue-on-error: true`. | Yes |
| **T13** | Results doc at `docs/benchmarks/results/2026-04-XX-forge-isolation-stage1.md` mirroring forge-identity precedent. | Yes |
| **T14** | Close 2A-5: HANDOFF append, Stage 1 task complete, MEMORY index entry. | Yes |

**Critical path:** T1 → {T2.1, T2.2 parallel} → T2.3 → T3 → {T4, T5, T6 parallel-safe after T3} → T7 → T8 → T9 → T10 → T11 → T12 → T13 → T14.

**Estimated commits:** 10-14 (depends on calibration cycle count).

---

## 8. Open questions (v2 → v3 triggers)

1. **Embedding determinism for D2 recall@K.** Lifted `deterministic_embedding`
   produces a 768-dim consolidation-tuned vector. The Recall path also
   accepts `query_embedding: Option<Vec<f32>>` (bench-gated, per Tier 3 D10);
   bench can either (a) pass query embeddings explicitly via this param or
   (b) let the daemon's BM25 path do its own embedding. v2 leaves the choice
   to T4 implementation; if (b) leads to <1.0 D2 calibration, switch to (a).
2. **Tag-leakage class — explicit defer to v2.** Real leakage vector
   (FTS over tags). v1 sidesteps via corpus design (no foreign project
   names in tags); v2 must add a Dim 7 tag-sanitization probe and possibly
   drop tags from FTS. Documented as out-of-scope in §5.
3. **`compile_context` non-memory layers (D6 partial coverage).** v1 D6
   audits memory-layer leakage via compile_context. Skills, declared,
   domain_dna, perception, entity layers each contribute subtrees to the
   compile_context XML output; v1 does NOT seed those tables. A leakage bug
   in `search_skills` or graph-neighbor helpers is invisible. v2 extends
   the corpus to seed these tables (1 row per project per layer, minimum).
4. **Bench wall-clock target.** Forge-identity at 1.0s; forge-isolation
   should beat that since it shares one DaemonState across 6 dims (vs.
   forge-identity's 6 isolated states). Target ≤ 1.0s on ubuntu-latest.
   T1 measures; if exceeds 2s, demote to nightly.

---

## 9. Acceptance criteria

- [ ] All 6 dimensions land with non-zero implementations.
- [ ] T2.1 + T2.2 lifts complete with forge-identity composite byte-identical.
- [ ] Composite ≥ 0.95 on 5 seeds (calibration locked).
- [ ] 8 infrastructure assertions all pass on a fresh state.
- [ ] `forge-bench forge-isolation --seed 42` runs in < 1.5s on
      ubuntu-latest.
- [ ] `bench_run_completed` event emitted with
      `metadata_json.bench_name='forge-isolation'` and 6-element
      `dimensions[]` array.
- [ ] CI matrix includes the bench under `continue-on-error: true`.
- [ ] Adversarial review verdict `lockable-as-is` or `lockable-with-fixes`
      with all HIGH addressed.
- [ ] Results doc + events-namespace registry updated.
- [ ] `cargo clippy --workspace --features bench --tests -- -W clippy::all -D warnings` clean.

---

## 10. References

- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design.md` — bench harness precedent (v2 LOCKED).
- `docs/superpowers/reviews/2026-04-25-p3-3-2a-5-spec-domain-isolation.yaml` — v1 review (verdict: not-lockable; 13 findings).
- `docs/benchmarks/forge-identity-master-design.md` v6 — bench-internal pattern source.
- `docs/benchmarks/results/forge-consolidation-2026-04-17.md` — calibration / results-doc precedent.
- `docs/architecture/events-namespace.md` — `bench_run_completed` v1 contract + per-bench dim registry.
- `crates/daemon/src/bench/{common.rs, scoring.rs, telemetry.rs, forge_identity.rs, forge_consolidation.rs}` — implementation precedent.
- `crates/daemon/src/recall.rs:147,176,208,2047` — project-scoped recall + compile_context entrypoints.
- `crates/daemon/src/db/schema.rs:324-332` — memory.project column + index.
- `crates/daemon/src/db/ops.rs:569-571,691-704` — recall BM25 sanitization + project-filter SQL.

---

## Changelog

- **v1 (2026-04-25):** Initial draft. 5 dims, memory-only probes,
  3-probe D5. Adversarial review returned `not-lockable` with 3 BLOCKER +
  3 HIGH + 4 MED + 3 LOW findings.
- **v2 (2026-04-25):** Address all v1 review findings. Key changes:
  - **B1 fix:** §2 fact 13 corrected; T2.1 lifts `deterministic_embedding`
    to `bench/common.rs`; §3.2 + §8.1 cite the correct fixture.
  - **B2 fix:** §2 fact 12 corrected; T2.2 lifts `composite_score` to
    `bench/scoring.rs` with N-dim signature.
  - **B3 fix:** §3.1 D1 query changed from `""` to shared
    `"isolation_bench"` tag; foreign-token denominator excludes globals
    (and includes alphabet sentinel for prefix-collision audit).
  - **H1 fix:** §3.1 added Dim 6 `compile_context_isolation` driving the
    user-facing context-assembly entrypoint; weight 0.25 (equal to D1).
  - **H2 fix:** §3.1a D5(a) redefined to align with actual SQL semantics
    (empty-string project requests global pool); spec §2 fact 14 documents.
  - **H3 fix:** §3.1a D5 expanded from 3 to 7 probes (added SQL injection,
    prefix collision, case sensitivity, trailing whitespace).
  - **M1 fix:** §3.7 mandates single shared `DaemonState` per seed.
  - **M2 fix:** §4 D9 + §5 disclaimer for `organization_id` deferral.
  - **M3 fix:** §5 explicit disclaimer for tag-substring leakage class.
  - **M4 fix:** §3.2 confidence formula deterministic-integer-derived
    (no `rand_range` sampling edge).
  - **L1 fix:** §3.4 explicit "fewer than forge-identity because surface
    is smaller" rationale.
  - **L2 fix:** T1 task line item added for `recall_raw_*` predicate grep.
  - **L3:** acknowledged as cosmetic; no spec change.
