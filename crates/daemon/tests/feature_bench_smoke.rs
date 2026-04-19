//! Smoke tests verifying the `bench` Cargo feature gate.

#[cfg(not(feature = "bench"))]
#[test]
fn bench_feature_gate_default_off() {
    // Default-off: ensure standard build doesn't get the variant by accident
    // (no compile-time check possible, just confirms test file compiles)
}

#[cfg(feature = "bench")]
#[test]
fn bench_feature_gate_exposes_compute_recency_factor() {
    // T5 (Phase 2A-4b): now that Request::ComputeRecencyFactor variant exists,
    // confirm the bench feature actually exposes it.
    let _check = matches!(
        forge_core::protocol::Request::ComputeRecencyFactor {
            memory_id: "test-id".to_string(),
        },
        forge_core::protocol::Request::ComputeRecencyFactor { .. }
    );
}
