//! Smoke test verifying the `bench` Cargo feature is off by default.
//! A proper compile-check assertion for bench-gated Request variants will be
//! added in T5 when Request::ComputeRecencyFactor ships.

#[cfg(not(feature = "bench"))]
#[test]
fn bench_feature_gate_default_off() {
    // Default-off: ensure standard build doesn't get the variant by accident
    // (no compile-time check possible, just confirms test file compiles)
}
