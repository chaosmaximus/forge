# Forge Benchmarks

Reproducible memory-benchmark results for the Forge raw storage layer.

This directory contains:
- **[plan.md](plan.md)** — full benchmark initiative plan (modes, custom benchmarks, observability, publishing rail)
- **results/** — per-run results, one Markdown file per benchmark run

## What's published here

Every result file under `results/` has the same shape:
1. Benchmark overview — what it measures, source, size
2. Setup — commit SHA, hardware, embedder, mode
3. Results table with per-category breakdown
4. Reproduction command (one-liner against this repo)
5. Comparison to other published systems
6. Honest limitations and known gaps

The full per-question JSONL and `summary.json` for each run live alongside the binary at the path printed by `forge-bench` (typically `bench_results/<benchmark>_<mode>_<unix_secs>/`). The results doc summarizes; the raw artifacts are the source of truth.

## Reproducing any result

Every result doc ends with a `repro.sh` block. To rerun a benchmark from a clean checkout:

```bash
git clone https://github.com/<your-fork>/forge
cd forge
cargo build --release --bin forge-bench
# Download the dataset (see the result doc for the exact URL)
# Then run forge-bench with the documented flags
```

The first run downloads the `all-MiniLM-L6-v2` ONNX weights (~90 MB) to `~/.cache/fastembed/`; subsequent runs are instant.

## Honesty rail

Per §7.3 of [plan.md](plan.md):

1. **Reproducible.** One `forge-bench` command from a clean checkout against the dataset's canonical hash.
2. **Sourced.** Own runs cite commit SHA + JSONL. Competitor numbers cite paper/blog/GitHub with permalink.
3. **Honest.** If extraction loses to raw, we say so. If a benchmark variant is designed to favor us, we say so explicitly.

We will not:
- Cherry-pick modes — every mode we run gets published, even bad ones.
- Cite competitor numbers without source (e.g., we cite the LongMemEval paper's Stella V5 baseline at 0.732 R@5, **not** any derivative table).
- Use judge prompts without publishing them.
- Run our system with rerank and compare against a competitor's raw number.

## Sanity check

Every benchmark run should also report the LongMemEval paper's Stella V5 reference retriever score on the same hardware (Table 3: R@5 = 0.732 at Value=Session, K=V+fact). If our harness reports Stella above 0.75 or below 0.70, the harness is broken — non-negotiable QA gate.

## Status

| Benchmark | Modes implemented | Last run |
|---|---|---|
| LongMemEval | raw | _see results/_ |
| LoCoMo | raw | _see results/_ |
| ConvoMem | (planned) | — |
| MemBench | (planned) | — |
| Forge-Persist | (planned) | — |
| Forge-Multi | (planned) | — |
| Forge-Transfer | (planned) | — |
| Forge-Tool | (planned) | — |
| Forge-Identity | (planned) | — |
