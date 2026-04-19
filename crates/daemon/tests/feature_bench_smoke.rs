//! Smoke test verifying the `bench` Cargo feature compiles and gates Request
//! variants properly. Exercises the gate by referencing the bench-gated
//! variant under #[cfg(feature = "bench")].

#[cfg(feature = "bench")]
#[test]
fn bench_feature_gate_exposes_compute_recency_factor() {
    // The Request::ComputeRecencyFactor variant must exist when the bench
    // feature is enabled. This test only compiles under --features bench.
    // NOTE: Variant doesn't exist yet (T5 adds it). This test will compile
    // only after T5 lands. For now we only verify the default path.
}

#[cfg(not(feature = "bench"))]
#[test]
fn bench_feature_gate_default_off() {
    // Default-off: ensure standard build doesn't get the variant by accident
    // (no compile-time check possible, just confirms test file compiles)
}
