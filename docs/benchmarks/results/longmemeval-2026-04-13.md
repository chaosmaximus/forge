# LongMemEval — Forge raw layer baseline (2026-04-13) + wave 1 hybrid (2026-04-14)

**Bench:** `forge-bench longmemeval`, mode `raw`
**Dataset:** [`xiaowu0162/longmemeval-cleaned`](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned) → `longmemeval_s_cleaned.json` (500 questions, 265 MB)
**Commit (2026-04-13):** [`05f62a9`](https://github.com/) — original raw KNN baseline
**Commit (2026-04-14):** [`9995c73`](https://github.com/) — wave 1 hybrid BM25+KNN RRF dispatch
**Hardware:** Apple M1 Pro, macOS Darwin 25.4.0 (arm64)
**Embedder:** `all-MiniLM-L6-v2` (384-dim) via `fastembed-rs 5.13` — unchanged between both runs
**Runtime:** 628.57 s for full-500 KNN parity re-run; 1186.85 s for full-500 hybrid (wave 1)

---

## Wave 1 hybrid update (2026-04-14)

Wave 1 of the [improvement roadmap](../improvement-roadmap-2026-04-13.md) added the long-missing BM25 leg to the raw layer (`raw_chunks_fts`, trigger-populated but previously never queried) and fused it with the existing KNN leg via **pure Reciprocal Rank Fusion** (k=60, no score blending). The new `--raw-mode hybrid` is the default; `--raw-mode knn` preserves the 2026-04-13 baseline byte-for-byte.

### Headline — full 500 questions

| Metric | KNN (2026-04-13) | KNN re-run (2026-04-14) | **Hybrid (NEW)** | MemPalace raw |
|---|---:|---:|---:|---:|
| **Mean Recall@5** | 0.9520 | 0.9520 ✓ parity | **0.9640** | **0.9660** |
| Mean Recall@10 | 0.9780 | 0.9780 | **0.9840** | — |
| Mean Recall_all@10 | 0.9300 | 0.9300 | **0.9340** | — |
| Mean NDCG@10 | 0.8858 | 0.8858 | **0.9195** | 0.8890 |

In ~400 LOC of hybrid plumbing — no embedder change, no new retrieval model, no rerank tier — wave 1 closes the gap to MemPalace from **-1.4 pp** to **-0.2 pp**, effectively within run-to-run noise.

### Per question_type Recall@5

| Question type | n | KNN | Hybrid | Delta |
|---|---:|---:|---:|---:|
| knowledge-update | 78 | 0.9872 | **1.0000** | **+1.3 pp** |
| temporal-reasoning | 133 | 0.9474 | **0.9850** | **+3.8 pp** |
| single-session-user | 70 | 0.9286 | **0.9857** | **+5.7 pp** |
| multi-session | 133 | 0.9774 | 0.9699 | −0.8 pp |
| single-session-assistant | 56 | 0.9286 | 0.9107 | −1.8 pp |
| **single-session-preference** | 30 | 0.8667 | **0.8000** | **−6.7 pp ← regression** |
| **Overall** | **500** | **0.9520** | **0.9640** | **+1.2 pp** |

### Honest finding — hybrid isn't uniformly better

Four categories improved substantially (knowledge-update, temporal-reasoning, and single-session-user all by +1 to +6 pp). Two regressed. **The largest regression is in single-session-preference, which was already the largest gap in the KNN baseline.** Wave 1 made the biggest gap worse.

**Diagnosis:** preference questions typically reference long-ago statements that are semantically paraphrased ("where did I take yoga classes?" → "I went to a yoga studio in Camden last year"). The BM25 leg fires on literal keyword overlap, which for paraphrased preferences is a noisier signal than the KNN semantic match. Adding BM25 candidates into the RRF pool lets keyword-match noise outrank the correct semantic hit within the top-5.

**This is the exact gap MemPalace's hybrid v3 addressed** with 16 preference-pattern regexes that synthesize `User has mentioned: X` sidecar documents at ingest time — captured as **T14 in Wave 3** of the roadmap. Wave 1 doesn't have those sidecars yet; the preference regression is expected to flip back to positive after Wave 3 lands.

Publishing the regression here — not hiding it — because the plan's honesty rail (§7.3) requires all numbers to ship, not just the improvements.

### Reproduction

```bash
# Hybrid (new default on --mode raw)
forge-bench longmemeval $LME --mode raw --raw-mode hybrid --output bench_results/

# Pure KNN (parity reproduction of the 2026-04-13 baseline)
forge-bench longmemeval $LME --mode raw --raw-mode knn --output bench_results/
```

Where `LME=/tmp/longmemeval-data/longmemeval_s_cleaned.json`. KNN runs ~10 min, hybrid ~20 min on the full 500-Q set (the BM25 query + RRF merge roughly doubles per-question wall time).

### Published JSONL data — wave 1 runs

- Hybrid full 500-Q: `bench_results/longmemeval_raw_1776175748/`
- KNN parity full 500-Q (2026-04-14 re-run): `bench_results/longmemeval_raw_1776175101/`

---

## Headline

**Forge raw mode scores 95.20% R@5 on the full 500-question LongMemEval.** That sits 1.4 points below MemPalace's published 96.6% on the same dataset, same embedder, same chunker, same scoring formulas. Single-session-assistant is at parity with MemPalace; the gap is concentrated in single-session-preference where MemPalace's later "hybrid v3" added regex pattern extraction that we have not yet built.

| Metric | Forge raw | MemPalace raw | Delta |
|---|---:|---:|---:|
| **Mean Recall@5** | **0.9520** | **0.9660** | **-1.4 pp** |
| Mean Recall@10 | 0.9780 | — | — |
| Mean Recall_all@10 | 0.9300 | — | — |
| Mean NDCG@10 | 0.8858 | 0.8890 | -0.3 pp |

## Per question_type Recall@5

| Question type | n | Forge raw | MemPalace raw | Delta |
|---|---:|---:|---:|---:|
| knowledge-update | 78 | **0.9872** | 0.9900 | -0.3 pp |
| multi-session | 133 | **0.9774** | 0.9850 | -0.8 pp |
| temporal-reasoning | 133 | **0.9474** | 0.9620 | -1.5 pp |
| single-session-user | 70 | **0.9286** | 0.9570 | -2.8 pp |
| single-session-assistant | 56 | **0.9286** | 0.9290 | -0.0 pp ← parity |
| single-session-preference | 30 | **0.8667** | 0.9330 | -6.6 pp ← weakest |
| **Overall** | **500** | **0.9520** | **0.9660** | **-1.4 pp** |

MemPalace raw column from [BENCHMARKS.md](https://github.com/MemPalace/mempalace/blob/main/benchmarks/BENCHMARKS.md) "LongMemEval — Breakdown by Question Type".

**Observations.**
1. **Single-session-assistant is at parity.** This is the category MemPalace cited as their weakest in raw mode (92.9%) because their bench script indexes only `role == "user"` turns. We match their methodology exactly here — and reproduce their score within 0.04pp. The harness is correctly mirroring their corpus build.
2. **Single-session-preference is our biggest gap (-6.6 pp).** Per MemPalace's own writeup, they fixed this category in their later "hybrid v3" with 16 regex preference patterns that synthesize "User has mentioned: X" sidecar documents at index time. Forge does not yet have that layer — adding it is the obvious next experiment, and is captured under "hybrid mode" in [plan.md](../plan.md) §3.
3. **Multi-session and knowledge-update are within 1 point.** Both depend on cross-session linking, which neither bench mode does explicitly — the win comes from KNN finding the right session by content alone.
4. **No category exceeds MemPalace.** That's expected — we are running their recipe, not improving on it. The 1.4-point gap is concentrated in two categories where their baseline scores were also already weakest.

## Setup

| | |
|---|---|
| Total questions | 500 |
| Haystack sessions per question | 38–62 (median 48) |
| Granularity | Session — chunks deduped to `haystack_session_id` at scoring time |
| Top-K retrieved | 50 chunks |
| Cosine cutoff | None (set to 2.0 to disable filtering, matching MemPalace) |
| Chunker | 800-char window, 100 overlap, 50-char minimum |
| Corpus build | **User turns only**, joined with `\n`, no role prefix (matching MemPalace's reference exactly — see `longmemeval_bench.py` lines 188–192) |
| Embedder | `all-MiniLM-L6-v2` 384-dim (fastembed-rs 5.13) |
| Vector store | sqlite-vec 0.1.9 (brute-force cosine KNN) |

The bench builds a **fresh in-memory SQLite database per question** — no cross-question state. It ingests every haystack session as a single `raw_documents` row containing only the user turns joined by newlines, embeds each chunk, runs a top-50 KNN against the question text, dedupes hits to session granularity, and scores against `answer_session_ids`.

## Reproduction

```bash
git clone https://github.com/<your-fork>/forge
cd forge
cargo build --release --bin forge-bench

mkdir -p /tmp/longmemeval-data
curl -fsSL -o /tmp/longmemeval-data/longmemeval_s_cleaned.json \
  https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json

cargo run --release --bin forge-bench -- longmemeval \
  /tmp/longmemeval-data/longmemeval_s_cleaned.json \
  --output bench_results/
```

First invocation downloads `all-MiniLM-L6-v2` ONNX weights (~90 MB) to `~/.cache/fastembed/`. Subsequent runs are instant on the model load. Full per-question JSONL and `summary.json` for this run live at `bench_results/longmemeval_raw_<unix_secs>/`.

## Comparison to other published systems

| System | Mode | Metric | Score | Source | LLM at query |
|---|---|---|---:|---|---|
| **MemPalace raw** | ChromaDB default, no LLM | R@5 | **0.9660** | [BENCHMARKS.md](https://github.com/MemPalace/mempalace/blob/main/benchmarks/BENCHMARKS.md) | no |
| **Forge raw (this run)** | sqlite-vec, no LLM | **R@5** | **0.9520** | this doc | no |
| MemPalace hybrid v4 + Haiku rerank | tuned + LLM rerank | R@5 | 1.0000 | BENCHMARKS.md | yes (Haiku) |
| Mastra Observational Memory (gpt-5-mini) | LLM observer | accuracy | 0.9487 | [mastra.ai](https://mastra.ai/research/observational-memory) | yes (gpt-5-mini) |
| Hindsight (Gemini-3 Pro) | LLM extraction | accuracy | 0.9140 | [arxiv 2512.12818](https://arxiv.org/html/2512.12818v1) | yes (Gemini-3) |
| LongMemEval paper oracle (gpt-4o full-context) | LLM in-context | accuracy | 0.9184 | [arxiv 2410.10813](https://arxiv.org/abs/2410.10813) Figure 3 | yes (gpt-4o) |
| Zep (gpt-4o) | graph + LLM | accuracy | 0.7120 | [arxiv 2501.13956](https://arxiv.org/abs/2501.13956) | yes |
| GPT-4o long-context (no memory system) | LLM in-context | accuracy | 0.6060 | arxiv 2410.10813 Figure 3 | yes |
| ChatGPT memory feature (production) | proprietary | accuracy | 0.5773 | arxiv 2410.10813 Figure 3 | yes |
| Llama 3.1 70B long-context | LLM in-context | accuracy | 0.3340 | arxiv 2410.10813 Figure 3 | yes |
| **LongMemEval paper Stella V5** | **dense retriever** | **R@5** | **0.7320** | arxiv 2410.10813 Table 3 | no |

**Two metric families** are mixed in this table — **read carefully**:
- **R@5 / R@10** = retrieval recall at session granularity. What `forge-bench` reports.
- **accuracy / LLM-judge** = end-to-end answer accuracy graded by an LLM. What the LongMemEval paper, Mastra, Hindsight, and Zep report.

The two are NOT directly comparable. A system can score 100% R@5 (perfect retrieval) and still fail at end-to-end accuracy if its reader is bad; conversely, a system can score well at LLM-judge with weak retrieval if the reader is good at filling gaps.

The two **directly comparable** rows in this table are MemPalace raw R@5 (0.9660) and Stella V5 R@5 (0.7320) from the original paper. Forge raw at 0.9520 sits between them — closer to MemPalace's tuned baseline than to the academic dense-retriever baseline.

## Sanity check — Stella V5 reference retriever

Per [plan.md](../plan.md) §7.3, every benchmark run should also report the LongMemEval paper's Stella V5 reference retriever score on the same hardware as a harness-correctness check. **This run does not include Stella V5 because we have not yet built a Stella V5 wrapper for `forge-bench`.** The closest cross-check we have is that our `single-session-assistant` score (0.9286) reproduces MemPalace's published 0.9290 within 0.04 pp on the same data, which is strong evidence the harness is computing R@5 correctly.

A separate Stella V5 sanity-check pass is on the follow-up list — see [plan.md](../plan.md) §10 (Risks).

## Honest limitations

1. **Forge raw is intentionally MemPalace's recipe.** We made no attempt to beat their score in this commit. Adding hybrid scoring (keyword overlap + temporal boost), preference pattern extraction, or an LLM rerank tier are explicit follow-ups that should each move the number measurably. The point of this run is to prove parity on the recipe, not to claim a win on the benchmark.
2. **One-run number.** This is a single execution on a single machine. The variance across runs is small (the embedder is deterministic given identical chunks), but reproducibility on different hardware will need its own validation.
3. **Top-50 search depth.** Each LongMemEval question has 38–62 haystack sessions. With chunking, the corpus is ~250–500 chunks per question. Top-50 covers ~10–20% of the corpus before the dedupe collapses to session granularity. This is the same width MemPalace uses, but a tighter top-K (e.g. top-10) would produce a different number — we'll publish that variant if it's useful.
4. **No reader stage.** This is a retrieval-only number. We don't run an LLM reader to generate answers from the retrieved chunks, so we can't report end-to-end LLM-judge accuracy. Adding the reader and the LongMemEval official judge prompt is on the roadmap.
5. **No `extract` / `consolidate` / `hybrid` mode comparison.** The whole point of the [benchmark plan](../plan.md) is to publish the four-mode comparison so we know whether our extraction pipeline adds retrieval value on top of raw. This commit ships only the raw column; the other three are next.
6. **No CI integration yet.** This run was triggered by hand and timestamped manually. The CI workflow that re-runs the bench on every PR (plan §11.7) is not yet wired.

## What's next

Per [plan.md](../plan.md) §7.1 phase cadence, the next steps are:

1. **Add `extract` mode** — runs the existing 8-layer Manas extraction pipeline against each haystack session and queries via `recall` instead of `raw_search`. Produces the first raw-vs-extract delta.
2. **Add `consolidate` mode** — extract + run the consolidation phases before querying. Tests whether consolidation recovers any of the extraction loss.
3. **Add `hybrid` mode** — RRF-merge raw chunks + extracted memories. Tests whether extraction adds value on top of raw.
4. **Re-run** all four modes on the same 500 questions and publish the comparison table. **This is the actual diagnostic the plan exists to produce.**
5. **Add ConvoMem and MemBench harnesses.** LoCoMo is already shipped — see `locomo-2026-04-13.md`.

If the hybrid number ≤ raw, we have empirical proof our extraction pipeline is purely architectural cost — and we anchor its value to non-retrieval axes (tools, identity, domain DNA, behavioral learning) rather than retrieval recall. That's a useful diagnostic, even if it's painful to publish.

---

# Four-mode comparison (LongMemEval, 50-question `single-session-user` subset)

**Update:** the original "raw mode baseline" section above is the headline number on the full 500-question benchmark. This section adds the empirical answer to the question the benchmark initiative exists to produce: **does Forge's extraction pipeline add retrieval value on top of raw storage?**

The comparison runs on the **first 50 questions** of `longmemeval_s_cleaned.json` — all of which happen to be `single-session-user` category. All four modes are scored against the same 50 questions on the same hardware with the same seeds. The differences are pure methodology.

## Results

| Mode | R@5 | R@10 | R_all@10 | NDCG@10 | Wall time |
|---|---:|---:|---:|---:|---:|
| **Raw** | **0.9400** | **0.9600** | **0.9600** | **0.8780** | 60 s |
| Extract (Forge 8-layer pipeline via Gemini 2.5 Flash) | 0.7600 | 0.8200 | 0.8200 | 0.6992 | 2307 s |
| Consolidate (Extract + `consolidator::run_all_phases`) | 0.7600 | 0.8200 | 0.8200 | 0.7106 | 2288 s |
| Hybrid (Raw chunks + Extract memories, RRF k=60) | 0.8600 | 0.9400 | 0.9400 | 0.7765 | 2367 s |

## Per-mode deltas vs Raw

| Mode | Δ R@5 | Δ R@10 | Δ NDCG@10 | Interpretation |
|---|---:|---:|---:|---|
| Extract | **−18.0 pp** | **−14.0 pp** | −17.9 pp | Extraction loses ~18 points of literal recall |
| Consolidate | −18.0 pp | −14.0 pp | −16.7 pp | Consolidation phases recover **zero** R@K; NDCG gains +1.1 pp over Extract (minor reranking) |
| Hybrid | **−8.0 pp** | **−2.0 pp** | −10.2 pp | Fusing Raw with Extract recovers most of the gap but still underperforms pure Raw on R@5 |

## Headline finding

**Raw verbatim storage beats every other mode on this sample.** The result is consistent with MemPalace's published "extraction throws away information" thesis, and we now have our own lab measurement of exactly how much is lost:

- Extract mode alone loses ~18 pp R@5 to raw. The Forge extraction prompt is **selective** by design (decisions / lessons / patterns / preferences / skills / identity) but LongMemEval asks literal factual questions ("where did I take yoga classes?") that don't match the summarized structured output.
- Consolidation does not recover it. The 9-phase consolidator (exact dedup, semantic dedup, linking, decay, promotion, reconsolidation, valence contradictions, entity extraction, portability scoring, protocol extraction) runs to completion on the extracted memories and produces a 1.1 pp NDCG bump over pure extract — not enough to move R@K at all.
- Hybrid (raw chunks + extracted memories, RRF-merged at query time) **recovers 10 pp** over extract but injects enough noise into the top-5 that it underperforms pure raw. The correct session still lands in the top-10 consistently (hybrid 0.94 vs raw 0.96 at R@10), so the noise is a rank-5-vs-rank-10 issue, not a missed-retrieval issue.

**This is the diagnostic the benchmark plan exists to produce, and the answer is clear.** For retrieval recall at session granularity on LongMemEval, Forge's current extraction pipeline adds zero retrieval value on top of raw storage — and in the R@5 metric, it actively hurts. Extraction's value must come from non-retrieval axes (tool/skill recall, identity persistence, behavioral learning) rather than the primary memory retrieval path.

## Reproducing each mode

```bash
# Raw (baseline, LLM-free)
forge-bench longmemeval $LME --limit 50 --mode raw

# Extract (requires GEMINI_API_KEY)
forge-bench longmemeval $LME --limit 50 --mode extract \
  --extract-model gemini-2.5-flash --extract-concurrency 8

# Consolidate (extract + full consolidation phases)
forge-bench longmemeval $LME --limit 50 --mode consolidate \
  --extract-model gemini-2.5-flash --extract-concurrency 8

# Hybrid (raw + extract RRF-merged, k=60)
forge-bench longmemeval $LME --limit 50 --mode hybrid \
  --extract-model gemini-2.5-flash --extract-concurrency 8
```

Where `LME=/tmp/longmemeval-data/longmemeval_s_cleaned.json`.

## Honest limitations (follow-ups to this result)

1. **Single question category.** The 50-question subset is all `single-session-user`. The delta may behave differently on `multi-session`, `temporal-reasoning`, `knowledge-update`, or `single-session-preference` — especially the knowledge-update category, where consolidation's update-handling logic should in theory help. Full 500-question four-mode comparison is a follow-up (cost ~$50 in Gemini Flash calls, ~4 hours wall time).
2. **Single extraction backend.** We ran with Gemini 2.5 Flash. Different backends (Claude Sonnet, GPT-5, fine-tuned extractors) would produce differently-shaped memories and could recover some of the gap. Our finding is specifically about **the Forge 8-layer extraction prompt running on Gemini 2.5 Flash**.
3. **BM25-only retrieval on the extraction layer.** We did not add a 768-dim embedder for `memory_vec` (that would require Ollama or a second fastembed model). Adding vector search on extracted memories would likely improve extract mode by several points but is a separate experiment.
4. **Extraction prompt is the prime suspect.** The prompt deliberately filters out casual factual content. A "pure recall" variant that captures every user statement verbatim (similar to MemPalace's raw storage) would likely close most of the gap — at which point the distinction between extract and raw dissolves, which is itself an interesting finding.
5. **LoCoMo shows the same pattern.** See `locomo-2026-04-13.md`. The 18 pp R@5 gap is consistent across both benchmarks — not a LongMemEval quirk.
6. **Sample size.** 50 questions has ±7 pp confidence intervals at 95%. The ~18 pp delta is well outside that, so the qualitative finding is robust, but the exact number should be taken as "about 15–20 pp" rather than "exactly 18."

## What this means for Forge's extraction architecture

The plan explicitly called out that a finding like this would land us at one of two positions:

> If hybrid > raw: we have proven extraction adds retrieval value. Lead with this number.
> If hybrid ≈ raw: extraction is justified for other reasons (tools, identity, behavior). Say so clearly.

The data puts us in the second camp — and in the R@5 metric, hybrid is actually **below** raw, not tied. **Forge's extraction pipeline does not pay for itself on retrieval recall metrics.** It must justify its existence on:

- **Tool and skill recall** — extracting reusable procedures from transcripts. Not tested by LongMemEval; captured in the upcoming Forge-Tool custom benchmark.
- **Identity persistence** — tracking user preferences and role over time, correctly updating when preferences change. Forge-Identity custom benchmark.
- **Multi-agent coordination** — agents sharing structured memories across sessions via FISP. Forge-Multi custom benchmark.
- **Behavioral pattern extraction** — capturing *how* a user works (debugging heuristic, architecture style) rather than *what* they said. Not a recall metric.

The next publishing phase must include those custom benchmarks so we can point at the dimensions where extraction DOES pay off, while being honest that it does not pay off on standard memory-recall benchmarks. Shipping raw-vs-extract without the custom benchmarks would be half the story.

## Published JSONL data

- Raw (50-Q): `bench_results/longmemeval_raw_1776097509/`
- Raw (full 500): `bench_results/longmemeval_raw_1776097632/`
- Extract: `bench_results/longmemeval_extract_1776103099/`
- Consolidate: `bench_results/longmemeval_consolidate_1776103408/`
- Hybrid: `bench_results/longmemeval_hybrid_1776107736/`

Each directory contains the full `results.jsonl`, `summary.json`, and `repro.sh` for the run.
