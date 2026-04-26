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

pub mod common;
pub mod forge_consolidation;
pub mod forge_context;
// forge_identity requires `Request::ComputeRecencyFactor` + `ResponseData::RecencyFactor`
// + `consolidator::PHASE_ORDER`, all of which live in forge-core / forge-daemon behind
// `#[cfg(any(test, feature = "bench"))]`. Since `cfg(test)` does not propagate across
// crate boundaries, the module must be feature-gated to match — `cargo test` on
// forge-daemon sees forge-core as a non-test dependency, so only `feature = "bench"`
// makes the bench-only enum variants visible here.
/// 2A-6 multi-agent coordination bench. Gated on `feature = "bench"`. Uses
/// production `sessions::send_message` + `respond_to_message` + `list_messages`
/// + `ack_messages` directly; emits via `bench::telemetry`.
#[cfg(feature = "bench")]
pub mod forge_coordination;
#[cfg(feature = "bench")]
pub mod forge_identity;
/// 2A-5 domain-transfer isolation bench. Gated on `feature = "bench"` for
/// consistency with forge_identity + telemetry; uses production
/// `Request::Recall` + `compile_dynamic_suffix_with_inj` (no bench-only
/// Request variants), but emits via `bench::telemetry` which IS bench-gated.
#[cfg(feature = "bench")]
pub mod forge_isolation;
pub mod forge_persist;
pub mod locomo;
pub mod longmemeval;
pub mod scoring;
#[cfg(feature = "bench")]
pub mod telemetry;

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
