# LongMemEval — Forge raw layer baseline (2026-04-13)

**Bench:** `forge-bench longmemeval`, mode `raw`
**Dataset:** [`xiaowu0162/longmemeval-cleaned`](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned) → `longmemeval_s_cleaned.json` (500 questions, 265 MB)
**Commit:** [`05f62a9`](https://github.com/) (head before this run)
**Hardware:** Apple M1 Pro, macOS Darwin 25.4.0 (arm64)
**Embedder:** `all-MiniLM-L6-v2` (384-dim) via `fastembed-rs 5.13`
**Runtime:** 701.57 s wall (~11.7 min) on a single CPU core, no GPU

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
