// bench/ — benchmark harnesses for memory benchmarks.
//
// One module per benchmark format; one shared `scoring` module for the
// retrieval metrics (Recall@K, NDCG@K). The actual binary that drives a run
// lives at `src/bin/forge-bench.rs` and depends on this module.
//
// Design notes (see docs/benchmarks/plan.md):
//   - Every question gets a fresh in-memory SQLite — no cross-question state.
//   - Embedder is loaded once and shared across all questions in a run.
//   - Output: per-question JSONL + summary stats.

pub mod longmemeval;
pub mod scoring;
