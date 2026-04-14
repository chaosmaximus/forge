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

pub mod locomo;
pub mod longmemeval;
pub mod scoring;

#[cfg(test)]
mod tests {
    #[test]
    fn reqwest_blocking_feature_is_enabled() {
        // Regression guard for Forge-Persist prerequisite (c).
        // The upcoming subprocess-based Forge-Persist harness talks to
        // a spawned forge-daemon via reqwest::blocking::Client. If the
        // "blocking" feature is ever removed from crates/daemon/Cargo.toml,
        // this test fails to compile — catching the regression before it
        // reaches the harness code.
        //
        // See docs/benchmarks/forge-persist-design.md §7.3 and §14 TDD
        // step (c).
        let _ = reqwest::blocking::Client::new();
    }
}
