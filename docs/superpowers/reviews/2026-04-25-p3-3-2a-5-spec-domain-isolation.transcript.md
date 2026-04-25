# Adversarial review — P3-3 2A-5 spec (domain-transfer isolation bench)

**Target:** `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md` v1
**Reviewer:** Claude Opus 4.7 (general-purpose), 2026-04-25
**Scope:** spec-only (no implementation yet)
**Commit range:** base `56185d2` → head `aa14763`

---

## Recon-fact verification (§2 of spec)

| # | Spec claim | Verified? | Evidence |
|---|------------|-----------|----------|
| 1 | `memory.project TEXT` at `db/schema.rs:324`, `idx_memory_project` at `:332` | YES | `grep -n project crates/daemon/src/db/schema.rs` shows `project TEXT,` at L324 and `CREATE INDEX … idx_memory_project ON memory(project);` at L332 |
| 2 | 7 tables carry `project TEXT` (memory, skill, domain_dna, perception, declared, entity, raw_documents) | YES | schema.rs L206 (raw_documents), L324 (memory), L456/460 (skill), L465/472 (domain_dna), L481/488 (perception), L497/501 (declared), L572/575 (entity) |
| 3 | `Request::Recall.project: Option<String>`; `BatchRecall` (via `RecallQuery`) lacks project | YES | `core/src/protocol/request.rs:68` (Recall.project), L21–23 (RecallQuery struct) |
| 4 | `recall.rs` lines 147, 176, 208, 425, 698, 850, 879 take `project: Option<&str>` | YES — all 7 confirmed | grep shows `project: Option<&str>,` at exactly 147, 176, 208, 425, 698, 850, 879. (2047, 2080 are `Option<&str>` too — `compile_context` pair.) |
| 5 | `compile_context(conn, agent, project)` at `recall.rs:2047` | YES | `pub fn compile_context(conn: &Connection, agent: &str, project: Option<&str>) -> String` at L2047 |
| 6 | Forge-identity precedent: 6 dims, composite ≥ 0.95, 14 infra checks | PARTIAL | composite gate confirmed (`COMPOSITE_THRESHOLD: f64 = 0.95` at L88, "14 InfrastructureCheck" at module doc L23). 6 dims confirmed (`[DimensionScore; 6]` at L114). |
| 7 | `bench/telemetry.rs::emit_bench_run_completed` opens short-lived rusqlite + WAL + busy_timeout | YES | telemetry.rs L150–174 |
| 8 | `forge-bench` clap dispatch — adding subcommand is small | YES | bin/forge-bench.rs L36 `#[command(subcommand)]`; existing forge-context, forge-consolidation, forge-identity branches at L519/613/etc. |
| 9 | `events-namespace.md` per-bench dim registry pattern | YES | events-namespace.md L158 "Per-bench `dimensions[].name` registry"; current rows at L167–169 for forge-identity / forge-consolidation / forge-context |
| 10 | CI bench-fast matrix `[forge-consolidation, forge-identity]` with `continue-on-error: true` | YES | ci.yml L186 `bench: [forge-consolidation, forge-identity]`; L182 `continue-on-error: true` |
| 11 | `bench/common.rs::seeded_rng(seed: u64) -> ChaCha20Rng` exists | YES — exact match | `crates/daemon/src/bench/common.rs:11-13` `pub fn seeded_rng(seed: u64) -> rand_chacha::ChaCha20Rng { … ChaCha20Rng::seed_from_u64(seed) }` |
| 12 | `bench/scoring.rs` exports composite + per-dim weighted-mean helpers | PARTIAL | scoring.rs exports `recall_any_at_k`, `recall_all_at_k`, `ndcg_at_k`. There is **no** `composite` or `weighted_mean` function in scoring.rs — `composite_score(&[DimensionScore; 6])` lives privately at `forge_identity.rs:1632`. Spec claim is wrong. |

**Recon errors found:** 2.

- **Fact 12** is wrong: `scoring.rs` does NOT export composite/weighted-mean. Spec must either add the helper or replicate the private composite-score function in `forge_isolation.rs`.
- **Open question §8.1** asserts `bench/common.rs::deterministic_embedding(seed_text)` exists, "matches forge-identity precedent". Grep finds **no** function called `deterministic_embedding` in `bench/common.rs`. The deterministic embedder lives privately in `forge_consolidation.rs:1684–1729` (function-local helper, `EMBEDDING_DIM = 768`). Spec also claims fixture dim is 384 (matches `RAW_EMBEDDING_DIM`), but the actual fixture in forge-consolidation produces 768-dim vectors and `RAW_EMBEDDING_DIM = 384` is for the `raw_documents` chunk path, NOT the `memory.embedding` path. Forge-isolation needs to (a) lift the deterministic embedder into `common.rs`, or (b) reimplement it inline; AND clarify the dim. This is an unverified-recon BLOCKER.

---

## Design assessment

### 1. Composite formula soundness — ACCEPTABLE

Weights sum to 1.00 (0.30 + 0.20 + 0.15 + 0.15 + 0.20). Math:

- D1=0.95, others=1.00 → composite = 0.30·0.95 + 0.70·1.00 = **0.985** → composite gate ≥0.95 PASS, every-dim min PASS.
- D1=0.84 (below its 0.95 min), others=1.00 → composite = 0.252 + 0.70 = **0.952** → composite gate PASS, but per-dim gate FAILS. Dual gate is **load-bearing** for D1 — without the per-dim min, a leakage regression of up to 16% could pass.
- Mirror of forge-identity master v6 §3 dual-gate. Acceptable.

### 2. Dimension coverage gaps — HIGH

Spec ships D1–D5 covering only the `memory` table. The recall.rs code path that actually assembles cross-project context is `compile_context(conn, agent, project)` at line **2047**. That function calls **9 distinct project-scoped helpers** (the 9 lines in fact 4 — note `recall.rs` actually has 7 functions taking the parameter, but `compile_context` itself orchestrates many more sub-queries):

- `db::manas::search_declared(conn, query, project)` — at L431 of recall.rs
- `db::manas::search_skills(conn, query, project)` — at L448
- domain DNA search per project — at L468
- decisions/lessons SQL with `(project = ?1 OR project IS NULL OR project = '')` — L1003, L1096
- skills filter via in-Rust `s.project.as_deref() == project` — L1184
- entities via `db::manas::list_entities(conn, project, …)` — L1291
- code-file project lookup — L1341
- perceptions via `list_unconsumed_perceptions(conn, None, project)` — L1417
- session selection by `agent = ?1 AND status = 'ended' AND project = ?2` — L706
- graph-neighbor expansion — L786 includes `m.project IS NULL OR m.project = ''`

A leakage bug in **any** of these (e.g., a missing WHERE clause in `search_skills`, a typo in the OR clause for graph neighbors) is invisible to a memory-table-only probe. The spec's success metric ("did this commit leak project X's memories into project Y's recall?") is then deceptively narrow — it only catches `recall_bm25_project_org_flipped` regressions. **Recommend** adding either (a) a Dim 6 that drives `compile_context` and asserts the **full XML output** contains no foreign-project token, or (b) a stronger §5 disclaimer that v1 only audits the `Recall` API path and explicitly defers `compile_context`/skills/entities/declared/perceptions.

### 3. Edge-case completeness for D5 — HIGH

D5 ships 3 probes. Critical missing cases:

- **Empty-string project semantics inverted.** Spec D5(a): `Some("")` "must not match any memory with non-empty project". The actual SQL at `db/ops.rs:705` is `m.project = ?2 OR m.project IS NULL OR m.project = ''`. Substituting `?2 = ""`: `m.project = '' OR m.project IS NULL OR m.project = ''` — this returns ALL rows with empty/NULL project (i.e., the global pool). The spec's expected behavior contradicts the implementation. The bench would always pass this probe trivially, **or** if "must not match" is interpreted strictly, would always fail. Either way the probe doesn't measure what the spec says.
- **SQL-injection probe missing.** `Some("alpha'; DROP TABLE memory;--")` is the canonical "show your bind-parameter posture" probe. Spec omits.
- **Unicode** (emoji, combining marks, RTL): omitted. Production project names today are ASCII, but adding Unicode probes future-proofs against new ingestion paths.
- **Prefix collision** (`alpha` vs `alphabet`): SQL `=` is exact, so safe — but a future LIKE-based regression would miss this. Worth one probe.
- **Case sensitivity** (`Alpha` vs `alpha`): SQLite `=` is binary-collation by default for TEXT, so `'Alpha' != 'alpha'`. Spec doesn't probe — could miss a regression that adds `COLLATE NOCASE` to the index.
- **Trailing whitespace** (`" alpha"`): exact-match would treat as different project; CLI/JSON layers may not trim. One probe suffices.
- **NULL via direct SQL** (`Some` is the only API surface; `Recall.project = None` is the unscoped case). Direct-SQL probes would require bypassing the API contract; likely safe to defer.

D5 with 3 probes scoring `pass_count / 3` means a **single missed case = 33% drop**, dragging the dim below its 0.85 min. Adding even 2 probes (SQL-injection + prefix-collision) without a category restructure can perversely make D5 brittler. Recommend: switch D5 scoring to `pass_count / total_probes` and add at least: SQL-injection, prefix-collision, case sensitivity. Re-set D5 min to 0.90 if probe count grows.

### 4. Calibration feasibility — MEDIUM

D1 min 0.95 needs at most 5% foreign tokens in returned set. With:
- N=5 projects × 30 templated memories each, all containing `"In project {project}, the {topic} pattern uses {detail}"`.
- Tags include `[project, "isolation_bench"]` — and **`tags` is part of `memory_fts`** (schema.rs L340: `INSERT INTO memory_fts(rowid, title, content, tags)`).
- `query=""` is the spec's broad probe in D1; `sanitize_fts5_query` returns empty for empty input, which causes `recall_bm25_project_org_flipped` to short-circuit at L569–571 with `Ok(Vec::new())`. **D1 with `query = ""` will always return the empty set** — 1.0 trivially, no signal whatsoever.

This is a BLOCKER on D1 design: either pick a non-empty broad query (e.g., `"isolation_bench"` matching the shared tag, but then global memories with the same tag confound the scoring), or use `Recall` with a `query` that selects the project token (`format!("project_{P}_secret")`) — but then "broad" is no longer broad.

Composite reachability if D1 stuck at 0.97: 0.30·0.97 + 0.70·1.00 = 0.991 ≥ 0.95 ✓. If D1 reaches only 0.95: composite = 0.985 ≥ 0.95 ✓. So D1 can be at min and composite still passes; calibration tolerance is fine on the math side. The calibration **risk is on D1 reaching even 0.85**, not on the composite.

### 5. Bench wall-clock — MEDIUM

Forge-identity ships 6 dims × 5 seeds × **per-dim isolated `DaemonState::new(":memory:")`** (forge_identity.rs L1703 `run_dim_isolated` spins a fresh state per dim). Total cost: 30 schema-init calls per run. Result: 1.0s per seed (T2 doc reports `pass_wall_duration_ms=770` once warmed).

Forge-isolation as spec'd uses a **single corpus + 5 dims × 5 seeds**. Spec implies one DaemonState per seed (not per dim — none of §3 says "isolated per dim"). 5 schema inits, 160 inserts, 25 recall calls (5 dims × 5 projects worst-case for D1+D2). On ubuntu-latest this is **likely 200–400ms per seed**, total under 1.5s.

But the spec's §3 doesn't make the per-dim-isolation choice explicit. If the implementer copies the forge-identity pattern (per-dim isolation), that's 5 dims × 5 seeds × `DaemonState::new(":memory:")` = 25 schema inits — closer to forge-identity's 30, putting wall-clock around 1.0s. Realistic. Open: spec should pin "single shared corpus per seed" vs "per-dim isolation" — this is a **lock-time decision** that affects D1's effective leakage detection (leakage IS a cross-dim property; per-dim isolation actively hides it).

### 6. CI matrix cost — ACCEPTABLE

Adding 1 ubuntu-latest job at ~1s wall-clock + ~30s build cache reuse = ~60s wall-clock. Total bench-fast goes from 2 → 3 jobs in parallel; queue time is ~0. No risk of exceeding the 15-min Tier-3 budget. The 14-day rollout policy applies to the whole matrix — if forge-isolation regresses, T17 promotion rolls all three back. Acceptable.

### 7. Out-of-scope items — bugs deferred?

- **Tag-leakage probe** (Q2 in §8): the spec correctly identifies that `tags=["beta_secret"]` on a `project=alpha` memory leaks via FTS5. Spec mitigates by NOT putting project names in tags during corpus generation. **This is a known leakage vector with an existing FTS5 trigger** (schema.rs L340: tags are FTS-indexed). The leakage class IS real. v1 deferring it is defensible (it's an isolation-test design choice, not a product bug to fix), but the spec should land a §5 line: "A memory with `project=Some(P)` whose `tags` contain a foreign-project name will leak via memory_fts. Defer fix to v2 + add tag-sanitization probe."
- **CrossProjectRecall request type** (§5): not a bug. Future feature. Safe defer.
- **`raw_documents.project`**: same column pattern. Spec notes "by-construction covered" — only true if `recall_raw_documents` shares the SQL predicate. Spec does NOT verify — needs a one-line check at impl time.
- **Skill / domain_dna / declared / entity / perception**: see Finding 2. These are the leakage surface that `compile_context` actually exposes; deferring without a flag in §5 acceptance criteria is risky.

### 8. Determinism risk under rand_chacha 0.10 — LOW

Forge-identity verified composite-stable post-bump at master 891a12c (commit message says so). Forge-isolation uses the same `seeded_rng`. rand_chacha 0.10 changed the random_range internal sampling for boundary edge cases (rand 0.10 release notes); spec uses `rng.random_range(0.7..0.99)` for confidence and templated string indices. Risk: a **single bit-shift in the sampling** could shift one memory's confidence enough to change ORDER BY tie-breaking. Mitigation: pin the corpus generator to use **integer indices**, derive confidence as `0.7 + (idx as f32 * 0.01).clamp(0.0, 0.29)`, avoiding `random_range` altogether. Recommend in §3.2.

---

## Verdict

`not-lockable` due to:

- **B1:** Recon §8.1 wrong (`deterministic_embedding` not in `common.rs`) + dim claim wrong (768 vs spec's implied 384).
- **B2:** Recon §2 fact 12 wrong (`scoring.rs` does not export composite/weighted-mean).
- **B3:** D1 with `query = ""` returns empty set due to FTS5 sanitizer short-circuit — D1 is currently un-implementable as written.

All three are spec-text edits, not implementation work. Once fixed the spec is `lockable-with-fixes` modulo the HIGH findings (D5 probe set, dimension coverage gap on `compile_context`).
