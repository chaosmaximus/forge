# Forge Benchmark Initiative — Plan

**Status:** draft — awaiting founder approval before implementation
**Date:** 2026-04-13
**Purpose:** establish Forge as a benchmarked AI memory system with reproducible public numbers across every major memory benchmark, covering all architectural modes (raw, extraction, consolidation, hybrid) with custom benchmarks exercising Forge's unique capabilities.

---

## 1. Why this exists

We have ~1,245 tests and zero published memory benchmarks. Every serious competitor publishes numbers on LongMemEval at minimum. Without numbers we cannot be compared, cited, or trusted by the community we are trying to reach.

Two goals:

- **Credibility.** Match or beat MemPalace's 96.6% on LongMemEval raw mode. Publish extraction and consolidation results honestly, whatever they are.
- **Self-measurement.** We do not currently know whether our 8-layer extraction pipeline adds retrieval value on top of raw storage. This initiative measures that empirically and tells us what to fix.

The honesty rail is the load-bearing constraint. If extraction scores lower than raw, we say so and explain what extraction buys that raw does not (tools, identity, domain DNA, behavioral patterns). The field's central integrity problem is judge-prompt drift and selective reporting. We will not participate.

---

## 2. Benchmarks

### 2.1 Standard (public, reproducible)

| Benchmark | Size | What it stresses | Source |
|---|---|---|---|
| **LongMemEval** | 500 Q, ~48 sessions each | Session-level retrieval across 6 question types | arxiv 2410.10813, HF `xiaowu0162/longmemeval-cleaned` |
| **LoCoMo** | 10 convos, 19–32 sessions, 1,986 QAs | Multi-hop, temporal-inference, adversarial speaker confusion | Snap Research, `github.com/snap-research/locomo` |
| **ConvoMem** | 75k+ QAs, 6 categories | Scale, changing facts, abstention, implicit connections | Salesforce arxiv 2511.10523, HF `Salesforce/ConvoMem` |
| **MemBench** | 8.5k+ items, 11 fine-grained categories | Factual + reflective memory, noisy / conditional / aggregative | ACL 2025 Findings, arxiv 2506.21605 |

### 2.2 Custom (Forge-specific moat coverage)

| Benchmark | What it tests | Why no standard exists |
|---|---|---|
| **Forge-Persist** | Memory survives daemon restart | All competitors are MCP servers or libraries; none run standalone |
| **Forge-Multi** | Cross-agent memory via FISP | No standard exists for multi-agent coordination |
| **Forge-Transfer** | Domain isolation + controlled transfer | No standard exists for project scoping |
| **Forge-Tool** | Tool/skill recall from transcripts | Competitors don't index tools |
| **Forge-Identity** | Time-ordered preference updates | LoCoMo has temporal but not identity-scoped |

---

## 3. Test modes

For each standard benchmark, we run four modes:

| Mode | Ingest path | Retrieval path | What it measures |
|---|---|---|---|
| **Raw** | Session text → chunks → embed → `raw_chunks_vec` | Top-K chunks → their session IDs | Pure retrieval quality (the MemPalace recipe) |
| **Extraction** | Session → Manas pipeline → memories tagged `source_session_id` | Top-K memories → their source session IDs | Cost of extraction — what we lose by summarizing |
| **Extraction + Consolidation** | Extract, then run full consolidation (dedup, link, strengthen) | Top-K consolidated memories → union of source session IDs | Whether consolidation recovers extraction loss |
| **Hybrid** | Both raw and extraction fire on ingest | RRF-merge raw chunks + extracted memories | Whether extraction adds value on top of raw |

The hybrid number is the one that matters most for our publishing story. If hybrid > raw, we have proven extraction adds retrieval value. If hybrid ≈ raw, we publish both numbers honestly and anchor the extraction value proposition to non-retrieval axes (tool recall, identity persistence, behavioral learning).

Optional fifth mode: **+LLM rerank** using Haiku. MemPalace shows rerank delivers +2 to +3 points on LongMemEval and ~+10 on LoCoMo. We add this as a separate row in the results table and never mix it with raw/extract/consolidate/hybrid scores.

---

## 4. Raw layer implementation spec

### 4.1 Codebase landscape (as-is)

- Ingest enters the daemon via the file watcher (`crates/daemon/src/workers/watcher.rs` → `workers/extractor.rs`) or through `Request::Backfill` / `Request::IngestDeclared` HTTP handlers (`server/handler.rs` line 1011+).
- Extraction calls Ollama or Claude API via `extraction/` modules and stores structured `Memory` rows.
- Existing sqlite-vec tables are `memory_vec` and `code_vec`, both 768-dimensional cosine (declared in `db/schema.rs` lines 67–80). sqlite-vec 0.1.9 is brute-force KNN — not HNSW despite CLAUDE.md's outdated claim.
- The existing embedder worker (`workers/embedder.rs`) polls for unembedded memories every 30 s and calls Ollama HTTP with `nomic-embed-text` (768-dim). There is NO Rust-native embedder in the tree today.

### 4.2 Schema additions

Two new SQL tables plus one new `vec0` virtual table plus one FTS5 table, all declared in `db/schema.rs::create_schema` following the existing idempotent pattern:

```sql
CREATE TABLE IF NOT EXISTS raw_documents (
  id TEXT PRIMARY KEY,
  project TEXT,
  session_id TEXT,
  source TEXT NOT NULL,
  text TEXT NOT NULL,
  timestamp TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS raw_chunks (
  id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES raw_documents(id) ON DELETE CASCADE,
  chunk_index INTEGER NOT NULL,
  text TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE(document_id, chunk_index)
);

CREATE VIRTUAL TABLE IF NOT EXISTS raw_chunks_vec USING vec0(
  id TEXT PRIMARY KEY,
  embedding float[384] distance_metric=cosine
);

CREATE VIRTUAL TABLE IF NOT EXISTS raw_chunks_fts USING fts5(
  text, content='raw_chunks', content_rowid='rowid'
);
```

**Critical:** `raw_chunks_vec` is 384-dim to match `all-MiniLM-L6-v2` (MemPalace's benchmark embedder). Do NOT reuse the existing 768-dim `memory_vec` table. The existing extraction path is untouched; raw and extraction operate on separate vec indices.

All CRUD helpers live in a new `crates/daemon/src/db/raw.rs` module.

### 4.3 Embedder integration

New module `crates/daemon/src/embed/` with:

- `Embedder` trait (`fn dim(&self) -> usize`, `fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>`)
- `minilm.rs` — fastembed-rs wrapper for `all-MiniLM-L6-v2`
- Shared `Arc<dyn Embedder>` on `DaemonState` via `OnceCell`

Add `fastembed = "4"` to `crates/daemon/Cargo.toml`. Model downloads to `~/.cache/fastembed/` on first use (~90 MB). Pre-fetch on daemon startup behind a config flag; fail loud with clear error if download fails. Surface cache path in `forge-next doctor`.

### 4.4 Chunking

New file `crates/daemon/src/chunk_raw.rs` (leave `chunk.rs` untouched — it's transcript-line parsing for the extractor). Single function `chunk_text(text, size, overlap, min)` with defaults `(800, 100, 50)` matching MemPalace for benchmark parity. Grapheme-safe char boundaries (`str::is_char_boundary`).

### 4.5 Ingest path

New background worker `workers/raw_ingestor.rs` mirroring `workers/extractor.rs`. The watcher's single `mpsc::channel::<PathBuf>` in `workers/mod.rs::spawn_workers` (line 55) becomes a fan-out: extraction worker AND raw worker both receive every path. Independent state; raw ingest is LLM-free and must not be debounced by the extractor's API-cost throttle.

For on-demand scripted ingest (bench runs), add an inline `raw::ingest_text(conn, embedder, text, project, session_id, source)` helper callable from `server/handler.rs` request handlers.

### 4.6 Retrieval API

New protocol variant `Request::RawSearch { query, project?, session_id?, k, max_distance? }` → `ResponseData::RawSearch { hits: Vec<RawHit> }`. Handler delegates to `raw::search(...)` in a new `crates/daemon/src/raw.rs` module. Contract tests in `crates/core/src/protocol/contract_tests.rs`.

Default `max_distance = 0.6` (MemPalace's empirical threshold on LongMemEval).

### 4.7 Query router (phase 2, stub only)

`recall.rs::hybrid_recall_scoped_org` at line 207 is the RRF fusion point. Phase 1 leaves a `// TODO(raw-fuse):` comment at that line. Phase 2 adds raw chunks as a fourth list fed into the existing `rrf_merge` function, with synthesized IDs `raw:<chunk_id>` and a new `source: "raw"` variant on `MemoryResult`.

### 4.8 CLI surface

New `crates/cli/src/commands/bench.rs`:

```
forge-next bench longmemeval <path> --mode {raw|extract|consolidate|hybrid} [--limit N]
forge-next bench locomo <path>       --mode ... [--granularity session|turn]
forge-next bench convomem            --category <cat> [--limit N]
forge-next bench membench <path>     --mode ... [--category <cat>]
```

Output: newline-delimited JSON results file `bench_<benchmark>_<mode>_<timestamp>.jsonl` plus summary stats to stdout. JSONL format compatible with the LongMemEval paper's eval scripts for independent verification.

### 4.9 Test strategy

Same-file `#[cfg(test)] mod tests` + `tempfile::TempDir` following existing crate conventions. Integration test `tests/raw_layer.rs` ingests a small fixture corpus and verifies retrieval. fastembed model download is gated behind an env flag in CI (pre-seeded cache directory) to avoid flaky CI from network issues.

### 4.10 Build order

| Step | Deliverable | Estimate |
|---|---|---|
| 1 | Schema + `db/raw.rs` CRUD | 0.5 day |
| 2 | `embed/` module + fastembed wiring | 1 day |
| 3 | `chunk_raw.rs` chunker | 0.25 day |
| 4 | `raw::ingest_text` helper | 0.5 day |
| 5 | `workers/raw_ingestor.rs` + fan-out | 1 day |
| 6 | Protocol variants + handler + `raw::search` | 0.75 day |
| 7 | CLI `bench` subcommands | 1.5 days |
| 8 | Integration tests | 0.75 day |
| 9 | Risk mitigations (reaper, preload, doctor) | 0.5 day |
| **Total** | | **6.75 days** |

Parallelizable: steps 2+3 with step 1; step 7 with step 8.

### 4.11 Risks (implementation-level)

1. **Schema freeze on 384-dim.** `raw_chunks_vec` embedding dimension cannot be altered after deploy without a data migration. Freeze at 384; guard with schema_version.
2. **Multi-vec-table contention.** Three `vec0` tables on one connection under WAL is untested in our codebase. Load-test in integration.
3. **Model download on first run.** Fastembed pulls ~90 MB. Air-gapped users hang. Mitigate via startup preload flag + clear error path.
4. **Dimension mismatch regression.** Copy-paste of 768-dim storage helpers into the 384-dim path will silently break KNN. Wrap `raw_chunks_vec` writes in an assertion helper.
5. **Storage growth.** ~1.5 KB per chunk × ~15k chunks per day for heavy users = ~8 GB per year. Add a reaper tick + retention config; default 90 days, free tier 30 days.

---

## 5. Benchmark harness design

### 5.1 Data acquisition

| Benchmark | Command | License | Auth |
|---|---|---|---|
| LongMemEval | `curl https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json` | MIT | None |
| LoCoMo | `git clone https://github.com/snap-research/locomo` | CC-BY-NC | None |
| ConvoMem | `git clone https://huggingface.co/datasets/Salesforce/ConvoMem` (LFS) | CC-BY-NC-4.0 | None (research-only flag in our runner) |
| MemBench | Manual fetch from Baidu Pan (captcha) or Google Drive | MIT (code); data license unclear | Manual |

### 5.2 Rust deserializers

Four typed structs in a new `forge-bench` module (or inside `cli`):

- `LongMemEvalEntry { question_id, question_type, question, question_date, answer, answer_session_ids: Vec<String>, haystack_dates, haystack_session_ids, haystack_sessions: Vec<Vec<Turn>> }` where `question_type` is a strict enum (6 variants) and `_abs` suffix on `question_id` flags abstention (30/500 questions in `longmemeval_s_cleaned.json`).
- `LocomoSample` with custom deserialization for the `session_N` sibling-key pattern — iterate a counter until `f"session_{n}"` is missing. Category is an `i32` in 1..=5.
- `ConvoMemFile { evidence_items: Vec<EvidenceItem> }` where `EvidenceItem { question, answer, message_evidences, conversations }`.
- `MemBenchItem` using `serde(untagged)` to handle flat-turn-list vs list-of-sessions shape variance, plus both `user`/`assistant` and `user_message`/`assistant_message` field naming.

All four formats fit `serde_json::Value` first, narrowed to typed structs.

### 5.3 Scoring primitives

Reimplement in Rust (each formula < 30 lines):

```rust
fn recall_any(retrieved: &[String], ground_truth: &HashSet<String>) -> f64
fn recall_all(retrieved: &[String], ground_truth: &HashSet<String>) -> f64
fn ndcg(rankings: &[usize], correct_ids: &HashSet<String>, corpus_ids: &[String], k: usize) -> f64
fn f1_token(prediction: &str, ground_truth: &str) -> f64   // for LoCoMo
```

Unit tests against the exact formulas in MemPalace's Python bench scripts, asserting bit-identical outputs on a small fixture. Non-negotiable.

### 5.4 Runner flow

For each benchmark × mode combination:

1. Load corpus file → iter examples.
2. For each example:
   a. Create a fresh project in Forge (raw mode → empty `raw_documents`; extraction → empty memories; etc).
   b. Ingest each haystack session via the appropriate daemon request (raw: `RawIngest`, extract: `IngestDeclared`, etc).
   c. Query via `RawSearch` or `Recall`.
   d. Resolve retrieved IDs → source session IDs.
   e. Score against ground truth.
   f. Emit JSONL record `{question_id, retrieved_ids, score, mode, timestamp}`.
3. Aggregate JSONL → per-category R@K, NDCG@K, overall.
4. Print summary table; save JSONL + summary to disk.

Ephemeral project isolation matters — each question gets a fresh slate so benchmark state doesn't cross-contaminate.

### 5.5 Output format (publishable)

```
bench_results/
  2026-04-13_longmemeval_raw/
    summary.json        — per-category scores + overall R@5/R@10/NDCG@10
    raw.jsonl           — per-question record
    system_info.json    — commit SHA, hardware, model versions, seed
    repro.sh            — exact command to reproduce this run
```

Every publication gets a `repro.sh`. Readers can rerun from a clean checkout. This is the integrity rail.

---

## 6. Custom benchmark specs

### 6.1 Forge-Persist — memory survives daemon restart

**Setup.**
1. Start daemon on a TempDir.
2. Ingest 100 synthetic sessions (mixed: decisions, preferences, tool usage, identity statements).
3. Run baseline — score R@5 on 100 hand-written questions.
4. `forge-next daemon stop` → wait 5 s → `forge-next daemon start`.
5. Re-run the same 100 queries.

**Metric:** `persistence_score = R@5_after / R@5_before`. Target 1.0 (lossless).

**Why it matters.** MemPalace is an MCP server; it cannot survive the host process exit. Only a standalone daemon can pass this test at all.

**Expected difficulty:** trivial for Forge, structurally impossible for MCP-based competitors.

### 6.2 Forge-Multi — cross-agent memory via FISP

**Setup.**
1. Start daemon.
2. Spawn two Forge sessions: Agent A (role = "researcher"), Agent B (role = "engineer").
3. Agent A ingests a transcript establishing fact A (e.g. "The DB schema uses snake_case").
4. Agent B ingests a transcript establishing fact B (e.g. "We deployed on 2026-03-15").
5. Ask a question requiring BOTH facts: "When did we ship the snake_case migration?"
6. Agent C (fresh session) queries without prior context and must resolve both facts via cross-session retrieval.

**Metric:** `cross_agent_recall` — % of questions where top-K contains source chunks from both A and B.

**Why it matters.** No standard memory benchmark tests multi-agent coordination. This is Forge-native.

### 6.3 Forge-Transfer — domain isolation + controlled cross-project recall

**Setup.**
1. Create Project A (code): ingest 50 Rust-code transcripts.
2. Create Project B (legal): ingest 50 contract-review transcripts.
3. Run 20 queries scoped to Project A. Verify zero Project B chunks surface (`leak_rate < 0.01`).
4. Run 20 queries scoped to Project B. Same check.
5. Enable cross-project mode. Run 20 cross-domain queries. Verify correct chunks surface from both.

**Metric:** `(isolation_precision, cross_project_recall)` — a pair, not a single number.

**Why it matters.** Our Reality Engine claims "swap domains, keep the mind." This is the empirical test of that claim.

### 6.4 Forge-Tool — tool and skill recall

**Setup.**
1. Seed transcripts with 20 tool-use patterns: shell commands, API calls, CLI idioms, build commands, test runners.
2. Query in natural language: "how do I run the Rust test suite in release mode?"
3. Verify the correct tool pattern surfaces in top-3.

**Metric:** `tool_recall@3`.

**Why it matters.** Competitors extract facts, not tools. This is a Forge-only test by definition.

### 6.5 Forge-Identity — time-ordered preference updates

**Setup.**
1. Seed transcripts stating preferences over time with explicit timestamps:
   - 2026-01-01: "I prefer Postgres for everything."
   - 2026-02-15: "Actually, I'm switching to DuckDB for analytics."
   - 2026-03-01: "For transactional, still Postgres."
2. Query: "what's my preference for analytics databases today?"
3. Correct answer: DuckDB.

**Metric:** `latest_state_accuracy` — % of queries where the newest applicable fact wins.

**Why it matters.** Tests the Ahankara layer + consolidation's update path. LoCoMo has temporal-inference but doesn't scope it to identity state.

---

## 7. Publishing plan

### 7.1 Cadence

| Phase | Published | Where | When |
|---|---|---|---|
| **Phase 0** — raw baseline | LongMemEval raw R@5 | Blog post + landing page | End of week 1 |
| **Phase 1** — all modes on LongMemEval | Raw, extraction, consolidation, hybrid | Benchmark results page on the site | End of week 2 |
| **Phase 2** — LoCoMo all modes | Multi-hop + temporal, same 4 modes | Same page | End of week 3 |
| **Phase 3** — ConvoMem + custom benchmarks | 3 ConvoMem categories + Forge-Persist + Forge-Multi | Dedicated benchmarks section | End of week 4 |
| **Phase 4** — MemBench + remaining custom | All 11 MemBench categories + remaining 3 custom | Same | End of month 2 |

### 7.2 Publication format

One Markdown doc per benchmark on our site, same template:

- Benchmark overview: what it measures, source, size
- Our setup: commit SHA, hardware, embedding model, mode configuration
- Results table: all modes, with and without rerank, R@K / NDCG@K per category
- Reproducibility: exact `forge-next bench` command, dataset download instructions
- Comparison: published numbers from other systems with sources and caveats
- Honest limitations: which modes we lost on and what that means
- Raw data: link to the JSONL output files

### 7.3 The honesty rail (load-bearing)

**Every published number meets all three:**

1. **Reproducible.** One `forge-next bench` command from a clean checkout against the dataset's canonical hash.
2. **Sourced.** Own runs cite commit SHA + JSONL. Competitor numbers cite paper / blog post / GitHub with permalink.
3. **Honest.** If extraction loses to raw, we say so. If a custom benchmark is designed to favor us, we say so explicitly.

**Things we will not do:**

- Cherry-pick modes. Every mode we run gets published, even bad ones.
- Cite competitor numbers without source. (MemPalace's competitive table quotes BM25 ~70%, Contriever ~78%, Stella ~85% — none of these match the LongMemEval paper's own Table 3. We will not repeat that.)
- Use judge prompts without publishing them.
- Run our system with rerank and compare against a competitor's raw number.
- Publish a "hybrid v4" number without the benchmark script that produces it in the same commit.

**Sanity check on every harness run:** every benchmark run must also report the LongMemEval paper's own Stella V5 reference retriever score on the same hardware (Table 3: R@5 = 0.732 at Value = Session, K = V + fact). If our harness reports Stella above 0.75 or below 0.70, the harness is broken. Non-negotiable QA gate.

---

## 8. Competitor landscape (as of April 2026)

### 8.1 Most trustworthy — peer-reviewed

- **LongMemEval paper oracle** (GPT-4o, full history in context): **91.84%** accuracy.
- **LongMemEval paper Stella V5** retriever best config: **R@5 = 73.2%**, NDCG@10 = 65.2% — the academic reference retriever baseline.
- **Long-context-only GPT-4o** on LongMemEval_S: **60.6%** (30-point drop from oracle).
- **Llama 3.1 70B** long-context: **33.4%**.

Source: arxiv 2410.10813 Table 3 and Figure 3.

### 8.2 Credible non-paper claims

| System | Benchmark | Score | Notes |
|---|---|---|---|
| **MemPalace raw mode** | LongMemEval R@5 | **96.6%** | Reproducible via public script. Independently verified on M2 Ultra |
| MemPalace hybrid v4 + Haiku rerank | LongMemEval R@5 | 100% | Authors acknowledge the rerank pipeline is not yet in the public benchmark scripts |
| Mastra Observational Memory (gpt-5-mini) | LongMemEval accuracy | 94.87% | Self-reported; no independent reproduction |
| **Hindsight** (Gemini-3 Pro) | LongMemEval accuracy | **91.4%** | arxiv 2512.12818, peer-review quality, academic co-authors |
| Supermemory (production) | LongMemEval accuracy | ~85% | Blog claim; exact methodology undisclosed |
| Supermemory ASMR (experimental) | LongMemEval accuracy | ~99% | Not in production; research-only |
| Zep (gpt-4o) | LongMemEval accuracy | 71.2% | arxiv 2501.13956, peer-review quality |
| Mem0 base | LoCoMo J (LLM-judge) | 66.88% | arxiv 2504.19413 |
| Mem0 graph | LoCoMo J | 68.44% | Same paper |
| MemPalace hybrid v5 no rerank | LoCoMo R@10 | 88.9% | Session granularity, top-10 |
| Memori | LoCoMo accuracy | 81.95% | memorilabs.ai |
| MemPalace | ConvoMem overall recall | 92.9% | Per-category 86–100% |
| Mem0 | ConvoMem accuracy | 30–45% | As reported in the ConvoMem paper |

### 8.3 Systems that have NOT published LongMemEval numbers

- **Letta / MemGPT** — Issue #3115 is an open feature request on their own repo.
- **OpenViking / ByteDance.**
- **Most other "memory" startups** outside this list.
- **MemBench** has no cross-system leaderboard at all.

### 8.4 Integrity issues we refuse to contribute to

1. **Judge-prompt drift.** The same memory system scores 13–20 points apart across different vendors' LoCoMo reproductions. Nobody publishes the judge prompt. (Example: Mem0's paper puts Zep at J = 65.99; Memori's leaderboard puts Zep at 79.09% on the same LoCoMo.)
2. **Cherry-picking modes.** "Hybrid v4 with Haiku rerank 100%" gets cited as "MemPalace 100%" without the rerank qualification.
3. **Unverified baselines.** MemPalace's competitive table lists BM25 ~70%, Contriever ~78%, Stella ~85% — none match the LongMemEval paper's Table 3 (Stella V5 best config is 0.732 R@5, not 0.85). We will cite the paper, not the derivative table.
4. **Reproduction discrepancies.** Mem0's reproduction of A-MEM puts it at J = 48.38 versus A-MEM's own ~65. The field has not solved reproducibility at all.

---

## 9. Timeline and ownership

| Week | Work | Owner |
|---|---|---|
| Week 1 | Raw layer schema + CRUD + embedder + chunker | Daemon (1 engineer) |
| Week 1 | LongMemEval harness (raw mode only) | CLI (1 engineer) |
| Week 2 | LongMemEval all four modes + first publication | Daemon + CLI |
| Week 3 | LoCoMo harness + all four modes | Daemon + CLI |
| Week 4 | ConvoMem 3 categories + Forge-Persist + Forge-Multi | Daemon + CLI |
| Month 2 | MemBench + Forge-Transfer + Forge-Tool + Forge-Identity | Daemon + CLI |
| Month 2 | Blog post series, landing page section, investor update | Marketing |

**Gate before Phase 1 publication.** Founder approves the raw mode number, whatever it is. If raw mode scores below 94% (2.5 points below MemPalace), we investigate the harness before publishing.

---

## 10. Risks (initiative-level)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Our raw mode scores significantly below MemPalace | Medium | High | Use the exact same embedder (`all-MiniLM-L6-v2`) and chunker (800/100). If we still fall short, the gap is the harness or the sqlite-vec brute-force KNN. Investigate before publishing. |
| Our extraction mode scores significantly below raw | High | Medium | This is the diagnostic we want. Publish honestly with commentary on what extraction buys that raw doesn't. |
| Community catches an error in our harness within 48 h | High | Medium | Sanity-check every run against Stella V5's paper number. Publish `repro.sh` and JSONL. Apologize and fix publicly, following MemPalace's April 7 template. |
| Custom benchmarks look self-serving | Medium | Medium | Publish the benchmark spec and dataset generator alongside results. Invite competitors to run them. Do not claim "we win" on custom benchmarks — claim "here's a dimension nobody else tests." |
| Storage growth hits free-tier users hard | Medium | High | Reaper ticks on retention. Free tier capped at 30 days raw retention. Document clearly. |
| Fastembed model download blocks first run | Medium | Medium | Preload flag at daemon startup; doctor reports cache path; clear error message if download fails. |

---

## 11. Observability & continuous KPI tracking

### 11.1 Why in-system observability

Benchmark runs are point-in-time snapshots. Real users want to know: *is my Forge getting better or worse? Is the raw layer hurting my latency? Did the last upgrade break anything?* Observability answers those continuously, and it turns benchmarks from one-time marketing into live product-health signals.

We track three layers of signal:

1. **Offline benchmarks (CI-driven).** On every commit, a 50-question subset of LongMemEval and 2-conversation subset of LoCoMo run in all four modes, plus all 5 custom benchmarks. Scores land in `kpi_benchmarks`. A regression > 2 points against the last green main commit fails the build.
2. **Online operational metrics.** Every `recall`, `raw_search`, `ingest`, `extract`, and `consolidate` call emits an event with latency, result count, and success. Aggregated hourly into `kpi_snapshots`.
3. **Implicit quality signals.** Proxies for production retrieval quality that don't require ground truth:
   - Re-query rate within 5 minutes (user rephrasing → retrieval was likely bad).
   - Memory click-through / action-on-result rate (user acts on a surfaced memory).
   - Explicit negative feedback ("that's wrong").

### 11.2 Schema additions

Four new tables, all in `db/schema.rs::create_schema`:

```sql
CREATE TABLE IF NOT EXISTS kpi_events (
  id TEXT PRIMARY KEY,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL,          -- 'recall', 'raw_search', 'ingest', 'extract', 'consolidate'
  project TEXT,
  latency_ms INTEGER,
  result_count INTEGER,
  success INTEGER NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_kpi_events_timestamp ON kpi_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_kpi_events_type ON kpi_events(event_type);

CREATE TABLE IF NOT EXISTS kpi_snapshots (
  id TEXT PRIMARY KEY,
  taken_at INTEGER NOT NULL,
  kpi_name TEXT NOT NULL,            -- 'recall_p50_ms', 'requery_rate_5min', 'storage_bytes', ...
  value REAL NOT NULL,
  window TEXT NOT NULL,              -- '1h', '1d', '7d', '30d'
  project TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_kpi_snapshots_taken_at ON kpi_snapshots(taken_at);
CREATE INDEX IF NOT EXISTS idx_kpi_snapshots_name ON kpi_snapshots(kpi_name);

CREATE TABLE IF NOT EXISTS kpi_benchmarks (
  id TEXT PRIMARY KEY,
  run_at INTEGER NOT NULL,
  benchmark TEXT NOT NULL,           -- 'longmemeval', 'locomo', 'convomem', 'membench', 'forge_persist', ...
  mode TEXT NOT NULL,                -- 'raw', 'extract', 'consolidate', 'hybrid', 'custom'
  metric TEXT NOT NULL,              -- 'r_at_5', 'ndcg_at_10', 'persistence_score', ...
  category TEXT,                     -- per-category breakdown
  value REAL NOT NULL,
  n_questions INTEGER NOT NULL,
  full_run INTEGER NOT NULL,         -- 0 = subset, 1 = full benchmark
  commit_sha TEXT,
  hardware TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_kpi_benchmarks_run_at ON kpi_benchmarks(run_at);
CREATE INDEX IF NOT EXISTS idx_kpi_benchmarks_bm ON kpi_benchmarks(benchmark, mode, metric);

CREATE TABLE IF NOT EXISTS uat_stories (
  id TEXT PRIMARY KEY,               -- 'UAT-1', 'UAT-2', ...
  name TEXT NOT NULL,
  description TEXT NOT NULL,
  benchmark TEXT NOT NULL,           -- link to source benchmark
  metric_name TEXT NOT NULL,
  metric_threshold REAL NOT NULL,
  last_run_at INTEGER,
  last_value REAL,
  last_passed INTEGER                -- 0 or 1
);
```

### 11.3 Event emission

Every request handler in `server/handler.rs` emits a `kpi_events` row on completion. New helper module `crates/daemon/src/kpi/emit.rs` exposes:

```rust
pub fn record_event(
    conn: &Connection,
    event_type: KpiEventType,
    project: Option<&str>,
    latency: Duration,
    result_count: Option<usize>,
    success: bool,
    metadata: Value,
) -> Result<()>
```

Non-blocking (writes go through a bounded `tokio::sync::mpsc` channel to a small writer task so the hot path is unaffected).

### 11.4 Background worker

New `crates/daemon/src/workers/kpi_collector.rs`. Runs every hour. For each KPI in a fixed list, queries `kpi_events` over the last hour/day/week, computes aggregates (p50, p95, p99, throughput, success rate, re-query rate), writes rows to `kpi_snapshots`. Re-query rate is computed by correlating events within 5-minute windows per project.

### 11.5 Prometheus metrics surface

The daemon already exposes `/metrics` (see `deploy/grafana/` dashboards). Add the following metric families, all sourced from `kpi_snapshots` and `kpi_benchmarks`:

```
forge_recall_latency_ms{quantile="0.5|0.95|0.99",project}
forge_recall_result_count{quantile,project}
forge_recall_requeries_5min{project}
forge_raw_search_latency_ms{quantile,project}
forge_ingest_latency_ms{quantile,project}
forge_ingest_throughput_per_sec{project}
forge_storage_bytes{table="raw_chunks|memories|code_chunks"}
forge_memories_total{layer}
forge_worker_backlog{worker="extractor|raw_ingestor|consolidator"}

forge_bench_latest{benchmark,mode,metric}       # gauge, written from kpi_benchmarks on every run
forge_bench_trend_7d{benchmark,mode,metric}     # 7-day delta

forge_uat_last_passed{story}                    # 0/1 per story
forge_uat_last_value{story}
forge_uat_pass_rate_30d                         # rolling pass rate
```

New Grafana dashboard row: **Benchmark & Quality** — panels for latest bench scores per benchmark/mode, 30-day trend line per metric, UAT pass/fail matrix, and implicit quality proxies.

### 11.6 CLI surface

New subcommand group in `crates/cli/src/commands/kpi.rs`:

```
forge-next kpi show                          # current snapshot table
forge-next kpi history <metric> --window 7d  # time series for one metric
forge-next bench latest                      # latest benchmark run per (bench, mode)
forge-next bench history <bench> --mode <mode> --window 30d
forge-next uat run                           # run all UAT stories, write to uat_stories table
forge-next uat status                        # latest pass/fail per story
forge-next telemetry enable | disable        # opt-in aggregate telemetry
```

### 11.7 Continuous benchmarking in CI

New `.github/workflows/bench.yml`:

- On every PR and nightly on main:
  - Run a 50-question subset of LongMemEval in all 4 modes.
  - Run LoCoMo on 2 conversations (subset).
  - Run all 5 custom benchmarks.
  - Store results in `kpi_benchmarks` inside the PR's test DB.
  - Diff against the last green main commit (stored in a dedicated `bench-history` branch as JSONL).
  - Fail the build on a regression > 2 points.
  - Post the diff as a PR comment.
- Full 500-question benchmarks run weekly on a dedicated runner and get published to the site via the phase cadence in §7.

---

## 12. UAT user stories (from benchmarks)

Every benchmark becomes a user story. User stories are product assertions about what Forge does for a user. We run them as integration tests, track them in the `uat_stories` table, and surface pass/fail status on the dashboard.

### 12.1 Story format

`Given / When / Then` + measurable metric + linked benchmark. One test file per story under `tests/uat/`.

### 12.2 The 7 core stories

**UAT-1: Memory survives daemon restart.**
- **Given:** I have 500 memories across 10 projects.
- **When:** the daemon restarts (maintenance, reboot, upgrade).
- **Then:** all memories are still retrievable with the same quality.
- **Metric:** `persistence_score = 1.0`
- **Benchmark:** Forge-Persist.

**UAT-2: Cross-agent context.**
- **Given:** my research agent learned fact A and my engineering agent learned fact B.
- **When:** I ask a question requiring both.
- **Then:** Forge surfaces both.
- **Metric:** `cross_agent_recall ≥ 0.8`
- **Benchmark:** Forge-Multi.

**UAT-3: Domain isolation.**
- **Given:** I have a code project and a legal project.
- **When:** I query within the code project.
- **Then:** zero legal-project memories surface.
- **Metric:** `leak_rate < 0.01`
- **Benchmark:** Forge-Transfer.

**UAT-4: Tool recall.**
- **Given:** I used a shell command 5 times last month.
- **When:** I ask "how do I do X?"
- **Then:** that exact command is in the top 3.
- **Metric:** `tool_recall@3 ≥ 0.9`
- **Benchmark:** Forge-Tool.

**UAT-5: Latest state wins.**
- **Given:** my preference changed over time (e.g., switched databases).
- **When:** I ask "what do I use for X?"
- **Then:** my newest preference wins, not my oldest.
- **Metric:** `latest_state_accuracy ≥ 0.9`
- **Benchmark:** Forge-Identity.

**UAT-6: Standard retrieval.**
- **Given:** 50+ conversation sessions.
- **When:** I ask a question answered in one session.
- **Then:** that session is in the top 5.
- **Metric:** `R@5 ≥ 0.94` on a 50-question LongMemEval subset.
- **Benchmark:** LongMemEval.

**UAT-7: Multi-hop reasoning.**
- **Given:** facts spread across sessions.
- **When:** I ask a question requiring multiple facts.
- **Then:** all relevant sessions are in the top 10.
- **Metric:** `R@10 ≥ 0.85` on a 2-conversation LoCoMo subset.
- **Benchmark:** LoCoMo.

### 12.3 Automation

- One integration test per story under `crates/daemon/tests/uat/uat_<n>_<slug>.rs`.
- `forge-next uat run` executes all stories locally against a TempDir-backed daemon, writes results to `uat_stories`.
- CI runs UAT on every PR and posts a pass/fail matrix as a PR comment.
- The Grafana **Benchmark & Quality** row shows green/red per story per day.
- Users can run `forge-next uat run` on their own instance to validate their Forge install — we publish this as the recommended post-install check.

### 12.4 Why this is powerful

- Benchmarks stop being marketing and become product health signals.
- Every release is gated on "did we regress on any user story?" — UAT failures block release.
- Users verify our claims on their own machine — not just trust our published numbers.
- Landing page claims like "Forge remembers across sessions" link to the live UAT-1 badge showing pass/fail for the last 30 days. No more unverifiable hand-waving.
- The 7 stories map directly to the landing page's hero statements: restart, cross-agent, domain, tool, identity, retrieval, reasoning. Content and product are aligned.

---

## 13. Analytics & telemetry

### 13.1 Principles

- **Local-first.** Everything lives in the user's SQLite. No data leaves the machine by default.
- **Opt-in aggregate only.** If the user opts in, we collect anonymized KPI summaries — never memory content, never queries, never personal data.
- **User-visible.** The user can see exactly what's tracked at `forge-next kpi show` and `forge-next telemetry status`. No hidden tracking.

### 13.2 What we track locally (always on)

All four KPI tables from §11.2 — `kpi_events`, `kpi_snapshots`, `kpi_benchmarks`, `uat_stories`. These are local, personal, and exposed to the user via the CLI and the canvas dashboard.

### 13.3 What we track in aggregate (opt-in only)

Only if the user runs `forge-next telemetry enable`:

- Daily anonymized ping:
  ```
  {
    "version":              "0.7.1",
    "hardware_tier":        "laptop|workstation|server",
    "storage_bytes_bucket": "<100MB|1GB|10GB|>10GB",
    "bench_r_at_5":         0.962,
    "uat_pass_rate":        1.0,
    "p95_recall_latency_ms": 84,
    "memories_total_bucket": "<1k|10k|100k|>100k"
  }
  ```
- No project names. No session IDs. No memory content. No queries. No IP address stored.
- Signed and sent over HTTPS to our telemetry endpoint; stored for 90 days; used only for aggregate health dashboards (how are our users' Forges doing in the wild?).
- Disabled anytime with `forge-next telemetry disable`. A `forge-next telemetry status` command shows whether telemetry is on and what the last ping contained.

### 13.4 User-facing "Brain Health" dashboard (canvas)

New panel in the Tauri canvas app:

- Memory count per layer: raw chunks, extracted memories, skills, tools, identity facets.
- Storage used: raw layer / extraction layer / total.
- Recall p50 latency: last 24h vs last 7d trend.
- UAT summary: "7 of 7 green" or which story is red.
- Benchmark trend: "Your Forge scores 96.2% on LongMemEval subset (stable for 14 days)."
- Implicit quality: "3% of your queries are re-phrases — lower is better."

This panel is also the source for a shareable badge users can post: **"My Forge: 96% LongMemEval / 7/7 UAT / 1,400 memories / Rust daemon."**

### 13.5 Operator / admin dashboard (Grafana)

Dashboards under `deploy/grafana/` get three rows added:

- **Benchmark & Quality:** latest bench scores, 30-day trend per (benchmark, mode, metric), UAT pass rate matrix.
- **Operational:** request latency p50/p95/p99, worker backlogs, storage growth, embedder cache hit rate.
- **Quality proxies:** re-query rate, click-through rate, negative feedback count, action-on-result rate.

Users running Forge in Kubernetes via the Helm chart get these dashboards pre-configured.

### 13.6 Schema cost

The four KPI tables grow slowly: ~1 KB/event × ~1000 events/day for a heavy user = ~1 MB/day → ~365 MB/year raw. The collector worker prunes `kpi_events` after 30 days (`kpi_snapshots` retains aggregates forever). Default retention configurable in `config.kpi.retention_days`.

---

## 14. Approvals required

**Items 1–4 were approved by the founder on 2026-04-13 in the planning conversation.** Items 5–7 are new additions from the observability extension and require a second approval pass.

| # | Item | Section | Status |
|---|---|---|---|
| 1 | Four test modes and their definitions | §3 | ✅ APPROVED 2026-04-13 |
| 2 | Raw layer schema (384-dim, separate vec table, fastembed-rs) | §4 | ✅ APPROVED 2026-04-13 |
| 3 | Five custom benchmark specs | §6 | ✅ APPROVED 2026-04-13 |
| 4 | Publishing cadence and honesty rail | §7 | ✅ APPROVED 2026-04-13 |
| 5 | KPI observability (4 tables, collector worker, Prometheus metrics, CLI group, CI bench workflow) | §11 | ⏳ PENDING |
| 6 | 7 UAT user stories + automation + release gating | §12 | ⏳ PENDING |
| 7 | Analytics model (local-first, opt-in aggregate, brain-health dashboard, retention) | §13 | ⏳ PENDING |

**After approval of 5–7:** the daemon team extends the raw layer build order with steps 10–13 (KPI schema, collector, Prometheus exporters, UAT integration tests). Phase 0 publication still happens end of week 1, but the observability scaffolding ships in week 2 alongside Phase 1.

**Revised build order** (adds ~3 eng-days on top of the 6.75 already estimated):

| Step | Deliverable | Estimate |
|---|---|---|
| 10 | KPI schema + event emitter helper | 0.5 day |
| 11 | kpi_collector worker + aggregations | 0.75 day |
| 12 | Prometheus metric exports + Grafana row | 0.5 day |
| 13 | UAT test harness + 7 integration tests + CLI `uat` group | 1 day |
| 14 | `.github/workflows/bench.yml` + diff gating | 0.25 day |
| **Observability addendum total** | | **3 days** |

Parallelizable with the main raw-layer build.

---

## Appendix A — Data format quick reference

| Benchmark | Top-level shape | QID field | Evidence field | Correct answer field | Category field |
|---|---|---|---|---|---|
| LongMemEval | `list[entry]` | `question_id` (+ `_abs` suffix for abstention) | `answer_session_ids` (list[str]) | `answer` (str) | `question_type` (6 enum values) |
| LoCoMo | `list[sample]`, qa nested | sample_id + index | `qa[i].evidence` (list[str] like `"D1:3"`) | `qa[i].answer` or `adversarial_answer` | `qa[i].category` (int 1–5) |
| ConvoMem | file per persona, `evidence_items` array | position | `message_evidences` (list[{speaker,text}]) | `answer` (str) | directory name (6 values) |
| MemBench | topic-keyed or role-keyed dict | `tid` (int) | `QA.target_step_id` (list[[sid, ?]]) | `QA.ground_truth` (letter A–D) + `QA.answer` | filename (11 values) |

## Appendix B — Known gotchas per benchmark

**LongMemEval.**
- `answer_session_ids` is a LIST (plural). Not `answer_session_id`.
- Abstention is a `question_id` suffix (`_abs`), not a `question_type` value. 30/500 in `longmemeval_s_cleaned.json`.
- The cleaned variant removes noisy sessions; results on raw LongMemEval will differ.
- Date format is non-ISO: `YYYY/MM/DD (DayAbbrev) HH:MM`.
- MemPalace indexes only `role == "user"` turns in session mode — the `single-session-assistant` category suffers. Index all turns.

**LoCoMo.**
- Sessions are sibling keys (`session_1`, `session_2`, ...), not a list. Iterate by counter.
- `category` is an integer 1–5.
- Adversarial (category 5) uses `adversarial_answer`. F1 against `answer` will always fail.
- `top-k > session_count` trivially scores 100% — honest runs use top-k = 10.
- Session granularity ≈ 60% R@10 baseline; dialog granularity ≈ 48%.

**ConvoMem.**
- HF dataset viewer has a CastError. Download raw files; don't use `datasets.load_dataset()`.
- One file contains many questions from one persona.
- 27.5 GB total — sample by category.
- CC-BY-NC-4.0 — research-only flag in our runner.
- Substring scoring is forgiving and may inflate numbers.

**MemBench.**
- Data is not in the git repo. Manual fetch from Baidu Pan or Google Drive.
- `message_list` shape is inconsistent across files.
- Field names drift (`user`/`assistant` vs `user_message`/`assistant_message`).
- Primary metric is MCQ accuracy, not retrieval; retrieval is stage 1 only.
- `noisy` category is the designed hard case.

---

**End of plan v1.**
