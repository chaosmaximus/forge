# Forge benchmark improvement roadmap — 2026-04-13

**Author:** forge-planner (verified against repo HEAD)
**Status:** PLAN-only. Awaiting founder approval before any wave starts.
**Scope:** lift retrieval recall on LongMemEval and LoCoMo only. Custom Forge-* benchmarks, KPI observability, UAT, ConvoMem, MemBench, and four-mode 500-Q runs are intentionally out of scope and called out in §4.

## 0. Ground truth (verified against the repo)

Before sequencing, here is what is and is not in the codebase right now. Every claim below was checked against HEAD.

**What exists:**

- `crates/daemon/src/raw.rs` (399 LOC). `raw::search` at lines 189–206 takes a query string, embeds once via `Embedder::embed`, calls `db::raw::search_chunks` (KNN only), filters by project/session and a cosine cutoff. Returns `Vec<RawHit>`. **No keyword side. No fusion. No reranking.**
- `crates/daemon/src/db/raw.rs:120` — `search_chunks` is the KNN endpoint over the `raw_chunks_vec` vec0 table (384-dim, hardwired).
- `crates/daemon/src/db/schema.rs:227–244` — the `raw_chunks_fts` FTS5 contentless table and its three triggers (`raw_chunks_fts_insert`, `raw_chunks_fts_delete`, `raw_chunks_fts_update`). The table is populated automatically on every chunk insert, but **nothing in the codebase ever reads from it.** `db/raw.rs:443` only references it inside a smoke test that asserts the schema is queryable. This is the unused half of the hybrid story.
- `crates/daemon/src/db/ops.rs:211` — `sanitize_fts5_query` is **private**. The bench harness has duplicated copies at `bench/longmemeval.rs:664` and `bench/locomo.rs:511`. Promoting it to `pub(crate)` is a 1-line refactor that unblocks reuse.
- `crates/daemon/src/embed/minilm.rs` — `MiniLMEmbedder` wraps `fastembed::TextEmbedding` with `EmbeddingModel::AllMiniLML6V2`. Dimension is hardcoded as `MINILM_DIM = 384` and asserted on every call. The `Embedder` trait at `embed/mod.rs:20–26` is genuinely pluggable — `dim()` is part of the contract — but **`db/raw.rs` creates the vec0 table with `float[384]`** so swapping the embedder requires either a schema change or a parallel table.
- `crates/daemon/src/recall.rs` — `hybrid_recall` (line 187) and `hybrid_recall_scoped_org` (line 234) operate on the **extracted memory layer** (`memory_fts` + `memory_vec`), NOT on raw chunks. The `rrf_merge` function at lines 11–64 is a clean, generic implementation that takes `&[Vec<(String, f64)>]`. Reusable.
- `crates/daemon/src/bench/longmemeval.rs` — `BenchMode::{Raw, Extract, Consolidate, Hybrid}` enum at lines 122–149. All four modes are wired through `forge-bench`. Hybrid mode does RRF between raw embedding hits and `memory` BM25 hits — but the raw side itself is still pure KNN. The phase-2 "RRF *inside* the raw layer" is unbuilt.
- `crates/daemon/src/bench/locomo.rs` — `run_sample_raw` at line 262 (verified — there is no `run_sample_consolidate` or `run_sample_hybrid` for LoCoMo yet — only Raw and Extract).
- `crates/daemon/src/bin/forge-bench.rs` — clap CLI at lines 27–89. The `--mode` flag exists for both subcommands and `BenchMode::parse` accepts `raw|extract|consolidate|hybrid` for LongMemEval. **LoCoMo currently only branches on `raw|extract`** in `run_locomo` (line 363+). Adding `consolidate|hybrid` to LoCoMo is part of this roadmap.

**What does not exist:**

- No `query/` module. No temporal date parser. No quoted-phrase detector. No person-name extractor. No preference regex set.
- No `rerank/` module. No LLM rerank wired anywhere in the bench or daemon path.
- No `extraction::sidecars` module.
- No bge-large embedder.
- The `// TODO(raw-fuse):` comment described as existing at `recall.rs:207` is **NOT present in source** — that comment is in `docs/benchmarks/plan.md:137` as a description of what Phase 1 was supposed to leave behind, not as a code marker. Phase 1 left an unwritten seam, not a written one. This is a documentation/code drift that wave 1 will fix.

**Headline numbers as committed (`docs/benchmarks/results/`):**

| Bench | Forge raw R@5 | Forge raw R@10 | MemPalace baseline | Gap |
|---|---:|---:|---:|---:|
| LongMemEval 500-Q | 0.9520 | 0.9780 | 0.9660 R@5 | -1.4 pp R@5 |
| LoCoMo full | 0.7638 | 0.8746 | 0.8890 R@10 (hybrid v5) / 0.9240 (bge hybrid) | -1.4 / -5.0 pp R@10 |

Per-category gaps (LongMemEval): single-session-preference is -6.6 pp (the dominant gap), single-session-user is -2.8 pp, temporal-reasoning is -1.5 pp, knowledge-update and multi-session are within 1 pp. Per-category (LoCoMo R@10): temporal-inference 0.7604 and adversarial 0.8789 are the weakest. These distributions tell us which techniques will actually move the dial.

## 1. Recommended sequence — 4 waves

The waves are ordered by `(expected_gain × confidence) / cost`. Each wave is a single PR / single commit batch / single bench publication. A wave does not start until the prior wave is benched, the numbers are reviewed, and the founder signs off on shipping.

### Wave 1 — Hybrid raw (BM25 + vec inside the raw layer)

**Goal:** ship the missing keyword leg of the raw layer. This is the load-bearing commit because every per-query technique in waves 2 and 3 depends on having a keyword pipeline to plug into. Without it, kw-overlap, quoted phrase, name boost, and temporal anchors have nowhere to live.

**Targets:**

- LongMemEval R@5: **0.952 → 0.958** (+0.6 pp)
- LoCoMo R@10: **0.8746 → 0.890** (+1.5 pp, comparable to MemPalace's hybrid v5)

**Techniques included:**

1. **T1** — promote `sanitize_fts5_query` to `pub(crate)` and delete the two bench duplicates
2. **T2** — add `db::raw::search_chunks_bm25` that runs FTS5 MATCH against `raw_chunks_fts` joined back to `raw_chunks` and `raw_documents`
3. **T3** — add `raw::hybrid_search` orchestrator that calls `search_chunks` (KNN) and `search_chunks_bm25` in parallel, then RRF-merges
4. **T4** — wire `raw::hybrid_search` into both bench runners as the new default for raw mode
5. **T5** — increase the candidate pool from top-50 to `max(50, 10×K)` so downstream rerank/feature techniques have headroom

**File changes:**

- `crates/daemon/src/db/ops.rs:211` — change `fn` to `pub(crate) fn`. -2 LOC including doc tweak.
- `crates/daemon/src/db/raw.rs` — add `pub fn search_chunks_bm25`. ~80 LOC.
- `crates/daemon/src/raw.rs` — add `pub fn hybrid_search`. ~50 LOC plus a private `rrf_merge_raw` helper.
- `crates/daemon/src/bench/longmemeval.rs:250–258` — call `raw::hybrid_search` instead of `raw::search`. ~5 LOC.
- `crates/daemon/src/bench/locomo.rs:312–320` — same. ~5 LOC.
- `crates/daemon/src/bench/longmemeval.rs:660–700` — delete `sanitize_for_fts`, import from `db::ops`. -40 LOC.
- `crates/daemon/src/bench/locomo.rs:510–540` — same. -30 LOC.
- `crates/daemon/src/bin/forge-bench.rs` — add `--raw-hybrid` flag (default on for wave 1). ~15 LOC.

**Estimated eng-days:** 3 days. Day 1: T1+T2 + unit tests. Day 2: T3+T4 + integration tests. Day 3: full 500-Q + LoCoMo bench runs, results doc update.

**Risk:** BM25 over `raw_chunks_fts` may regress against pure-KNN on categories where the chunker has split a session into highly-similar chunks. Mitigation: RRF damps this; fall back to weighted fusion `0.7*vec_rrf + 0.3*bm25_rrf` if per-category regressions appear.

**How we bench it:** existing harness. `forge-bench longmemeval ... --mode raw` (now hybrid-fused under the hood) and compare against committed baseline.

---

### Wave 2 — Embedder upgrade to bge-large

**Goal:** the single biggest experimentally-measured lever in the LoCoMo data. MemPalace's BENCHMARKS.md documented a +3.5 pp R@10 jump on overall LoCoMo from MiniLM → bge-large, with the gain concentrated in temporal-inference (+10 pp) and single-hop (+17 pp on the long-tail questions).

**Targets:**

- LongMemEval R@5: **0.958 → 0.965** (+0.7 pp)
- LoCoMo R@10: **0.890 → 0.918** (+2.8 pp toward MemPalace's 0.924 bge-large hybrid number)

**Techniques included:**

6. **T6** — add `crates/daemon/src/embed/bge_large.rs` implementing `Embedder` over `fastembed::EmbeddingModel::BGELargeENV15`, dim=1024
7. **T7** — schema migration: add `raw_chunks_vec_large` vec0 table with `float[1024]`
8. **T8** — add a config flag `[embedder] model = "minilm" | "bge-large"`
9. **T9** — bench harness selects the right vec table at search time based on the embedder's `dim()`

**File changes:**

- `crates/daemon/src/embed/mod.rs` — `pub mod bge_large;`. +1 LOC.
- `crates/daemon/src/embed/bge_large.rs` — new file modeled on `minilm.rs`. ~140 LOC.
- `crates/daemon/src/db/schema.rs` — new `raw_chunks_vec_large` vec0 table. +20 LOC.
- `crates/daemon/src/db/raw.rs:120` — `search_chunks` dispatches on dim. +30 LOC.
- `crates/daemon/src/raw.rs:189` — `search` / `hybrid_search` use `embedder.dim()` to pick the table. +5 LOC.
- `crates/daemon/src/config.rs` — add `EmbedderModel` enum. +15 LOC.
- `crates/daemon/src/bin/forge-bench.rs` — `--embedder {minilm|bge-large}` flag. +20 LOC.

**Estimated eng-days:** 3 days.

**Risk:**

- bge-large weights are ~1.3 GB vs MiniLM's 90 MB. First-call download cost is real and CI bench runners need pre-warmed caches.
- 1024-dim KNN over the same corpus is ~2.7× the latency of 384-dim KNN.
- Schema migration in production daemons: existing user databases have `raw_chunks_vec` populated with 384-dim vectors. Dual-table approach keeps old data working and new chunks embedded with whichever model is configured at ingest time.

**Dependencies:** wave 1.

---

### Wave 3 — Query feature engineering (the MemPalace hybrid v3–v5 progression)

**Goal:** the meat of the recall-improvement story. Per-query techniques live here: keyword overlap fused with embeddings, temporal date anchors, quoted phrase detection, person-name boost, preference sidecars. Each technique plugs into the wave-1 hybrid raw path.

**Targets:**

- LongMemEval R@5: **0.965 → 0.978** (+1.3 pp; this is where we close the bulk of the -6.6 pp single-session-preference gap)
- LoCoMo R@10: **0.918 → 0.928** (+1.0 pp)

**Techniques included:**

10. **T10** — `query::keywords` module: tokenize → strip stopwords (50-word list from MemPalace `longmemeval_bench.py:501–551`) → kw-overlap distance reduction at weight **0.30** for LongMemEval and **0.50** for LoCoMo (MemPalace's grid-searched values)
11. **T11** — `query::temporal` module: 12 regex patterns (yesterday, last week, N days ago, etc.), parse into a date range, boost chunks whose source document `timestamp` falls inside the range up to 40%, linear ramp to 3× tolerance
12. **T12** — `query::phrases` module: detect quoted spans `"..."`, 60% distance reduction on verbatim matches
13. **T13** — `query::names` module: detect capitalized bigrams not at sentence start, 40% distance reduction. LoCoMo strips speaker names from keyword pool via 78-word `NOT_NAMES` filter (locomo_bench.py:196–326) and re-boosts names separately at 0.20
14. **T14** — `extraction::sidecars` module: 16 preference regex patterns (MemPalace v3 set at longmemeval_bench.py:1138) that synthesize a one-line `User has mentioned: X` document into `raw_documents` at ingest time. **Closes most of the -6.6 pp single-session-preference gap.**
15. **T15** — assistant-reference two-pass: 16 trigger phrases ("you said", "you mentioned", etc.) → rebuild temporary scratch index including assistant turns, query, RRF-merge with primary results

**New modules:** `query/{mod,keywords,temporal,phrases,names}.rs`, `extraction/sidecars.rs`.

**Dependencies:** wave 1 (hybrid orchestrator to plug into) and wave 2 (bge-large baseline locked in first).

**Estimated eng-days:** 5 days.

**Risk:**

- **Per-technique deltas are not additive.** Stacking may show diminishing returns or interference. Mitigation: bench after each technique, backout switches per feature.
- `NOT_NAMES` and 16 preference regexes are tuned to LongMemEval/LoCoMo specifically — may overfit. Mitigation: keep as data not policy, gate production rollout.
- Temporal regex misfires on European dates and idiosyncratic phrases. Mitigation: unit-test every pattern; `tracing::warn` on misparse.

---

### Wave 4 — LLM rerank (Gemini Flash, top-20)

**Goal:** the last lever before the realistic 0.97–0.98 ceiling MemPalace's own clean held-out 450 number documents.

**Targets:**

- LongMemEval R@5: **0.978 → 0.982** (+0.4 pp; matching the realistic clean MemPalace ceiling)
- LoCoMo R@10: **0.928 → 0.940** (+1.2 pp)

**Techniques included:**

16. **T16** — `crates/daemon/src/rerank/mod.rs` and `crates/daemon/src/rerank/gemini.rs`. Reuses the existing Gemini HTTP client from `extraction/gemini.rs`. Sends top-20 candidates with the query and asks Gemini for a relevance ordering. 6-line prompt, `max_output_tokens=256`. MemPalace prompt at `longmemeval_bench.py:2765–2814` as template.
17. **T17** — bench harness rerank flag: `forge-bench longmemeval ... --rerank gemini-2.5-flash --rerank-top 20`

**Dependencies:** wave 3 (rerank needs a high-quality top-K to reorder; reranking a noisy pool just shuffles garbage).

**Estimated eng-days:** 2 days.

**Risk:**

- Outbound API dependency in the daemon — meaningful production architecture change. Mitigation: rerank always opt-in, default off.
- Gemini Flash rate limits on sequential 500-Q runs. Mitigation: concurrency limiter already exists at `--extract-concurrency`.

---

## 2. Per-technique cards (abbreviated)

For each of T1–T17 the full roadmap includes: source reference with file:line, what it does in one sentence, where it lands in Forge (verified file path + function name), Rust pseudocode (5–20 lines), expected gain in pp with citation, dependencies, and a one-sentence test plan.

**The load-bearing technique to internalize:** T14 (preference sidecars) is the single largest per-category delta on LongMemEval. It closes the -6.6 pp single-session-preference gap — our current weakest category — by generating synthetic `User has mentioned: X` documents at ingest time. Copy the 16 regex patterns verbatim from `longmemeval_bench.py:1138` for first run.

## 3. Decision points for the founder

These need an explicit yes/no/different before wave 1 starts.

**D1 — bge-large schema migration: dual-table or replace-in-place?**

- **Recommendation: dual-table.** (`raw_chunks_vec` for 384-dim, `raw_chunks_vec_large` for 1024-dim, dispatched on `embedder.dim()`)
- **Why:** existing production data is already 384-dim. Replace-in-place requires re-embedding every chunk on first daemon start with bge-large enabled (hours of work, non-recoverable if download fails). Dual-table makes the swap reversible.
- **Cost:** ~20 LOC of dispatch glue and 1 extra vec0 table.

**D2 — Borrow MemPalace's regex sets verbatim, or tune on Forge data first?**

- **Recommendation: borrow verbatim for wave 3, tune later if numbers warrant.**
- **Why:** the 16 preference regexes and 78-word `NOT_NAMES` were grid-searched on LongMemEval and LoCoMo respectively. Re-tuning would consume 1–2 days per set with over-fitting risk. The published results doc can honestly say "borrowed verbatim from MemPalace" consistent with our parity-first approach.

**D3 — Wave 4 (LLM rerank): in scope or deferred?**

- **Recommendation: in scope, but ship in a separate publication wave.**
- **Why:** rerank adds outbound API dependency. Even opt-in, the bench publication shows "Forge with rerank" as a thing. We want to ship that conversation deliberately, not as a footnote. Building it after wave 3 gives us a clean A/B.
- **Alternative:** defer entirely if Forge's positioning "memory works without a cloud call at query time" is pristine-required.

**D4 — Scope of wave 1 (the first commit):**

- **Recommendation: all 5 techniques (T1–T5) in one PR.**
- **Why:** T1 is a 1-line refactor, T2 is the new BM25 endpoint, T3 is the orchestrator, T4 is the bench wiring, T5 is the pool size. Splitting them creates 4 PRs that all need T3 to be testable. One PR is ~200 LOC of new code plus ~70 LOC of bench cleanup.

**D5 — Bench mode definition: does "raw" become hybrid, or do we add `raw-knn-only`?**

- **Recommendation: keep "raw" but it becomes hybrid by default**, and add a `--raw-mode {knn|hybrid}` flag with `hybrid` as the default. Old-style pure KNN is preserved behind `--raw-mode knn` so we can keep publishing the parity number against MemPalace.
- **Why:** the published "Forge raw" number (0.952 pure KNN) should stay intact for cross-system comparison. Keeping the flag costs ~10 LOC.

## 4. What this roadmap does NOT address

Explicitly out of scope. None of the items below are blocked or deprioritized — they are separate workstreams with their own plans.

- **Custom Forge-* benchmarks** (Forge-Persist, Forge-Multi, Forge-Transfer, Forge-Tool, Forge-Identity) — the answer to the published finding that extraction is retrieval-useless. Separate roadmap, separate publication wave. plan.md §6.
- **KPI observability layer** — `kpi_events`, `kpi_snapshots`, `kpi_benchmarks`, `uat_stories` tables already exist in `db/schema.rs`. Collector worker, Prometheus exporters, CLI group pending per plan.md row 5.
- **UAT integration tests** — plan.md §12, row 6 pending.
- **ConvoMem and MemBench harnesses** — plan.md phases 3–4.
- **Full 500-question four-mode comparison** — extract + consolidate at 500-Q scale. Currently only 50-Q four-mode and 500-Q raw are published.

This roadmap is **only** about lifting retrieval recall on LongMemEval and LoCoMo from 0.952/0.8746 toward 0.978/0.940.

## 5. Final publishable comparison table — what longmemeval-2026-04-13.md looks like after wave 1

| System | Mode | Embedder | Rerank | R@5 | Source |
|---|---|---|---|---:|---|
| Forge raw KNN-only (committed baseline) | sqlite-vec, no LLM | MiniLM-L6-v2 | no | **0.9520** | `bench_results/longmemeval_raw_1776097031` |
| **Forge hybrid raw (wave 1, projected)** | **sqlite-vec + FTS5 BM25 RRF, no LLM** | **MiniLM-L6-v2** | **no** | **0.955–0.960** | this doc |
| MemPalace raw (published) | ChromaDB default, no LLM | MiniLM-L6-v2 | no | 0.9660 | BENCHMARKS.md |
| MemPalace hybrid v4 (clean held-out 450) | tuned + sidecars | MiniLM-L6-v2 | no | 0.984 | MemPalace honesty disclosure |
| MemPalace hybrid v4 + Haiku rerank (full 500, contaminated) | tuned + LLM rerank | MiniLM-L6-v2 | yes (Haiku) | 1.0000 | BENCHMARKS.md |
| Mastra Observational Memory | LLM observer | — | yes (gpt-5-mini) | 0.9487* | mastra.ai |
| Hindsight | LLM extraction | — | yes (Gemini-3) | 0.9140* | arxiv 2512.12818 |
| LongMemEval paper Stella V5 | dense retriever, no LLM | Stella V5 1.5B | no | 0.7320 | arxiv 2410.10813 Table 3 |

\* end-to-end accuracy, not retrieval recall — flagged as a different metric family per the existing doc's "Two metric families" caveat.

After wave 2 (bge-large) the Forge row splits into MiniLM and bge-large variants, both published. After wave 3 (query features) we expect parity with MemPalace's clean 0.984; after wave 4 (rerank) we expect the 0.97–0.98 ceiling MemPalace's honesty disclosure documents as the realistic bound.

**Headline narrative we want to publish at the end of wave 4:** *"Forge has matched MemPalace's clean held-out LongMemEval R@5 number using a fully open-source pipeline that runs on a single CPU core, with the optional rerank tier matching their 0.984 ceiling."*

---

## Summary for sign-off

**Total eng-days across all four waves:** 13 days (3 + 3 + 5 + 2). Realistic calendar is ~3 weeks given bench reruns, founder review gates between waves, and the overhead of writing publishable results docs after each wave.

**Critical path:** wave 1 → wave 2 → wave 3 → wave 4. No parallelism is possible because every wave depends on the prior one's hybrid orchestrator being in place.

**The single load-bearing claim:** the missing keyword leg of the raw layer is the foundation that unblocks everything. `raw_chunks_fts` exists, is populated, and is never queried. Wave 1 is one PR that connects it. Without that PR, none of the per-query techniques in waves 2–4 have anywhere to land.

**Founder action requested:** approve or amend §3 D1–D5, then approve wave 1 to start. Waves 2–4 each get their own approval gate after the prior wave's bench numbers come in.

---

**One correction to the research input:** the `// TODO(raw-fuse):` comment described as existing at `recall.rs:207` is not in source — it lives in `docs/benchmarks/plan.md:137` as a description of an unwritten seam. Wave 1 makes that seam real. Aside from this single drift, every other code claim in the research input was verified accurate against repo HEAD.
