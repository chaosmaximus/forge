//! Integration test: Forge-Identity skeleton compiles + runs (Phase 2A-4d.3 T2).
//!
//! The forge-identity bench module is gated behind `cfg(any(test, feature =
//! "bench"))` because it references bench-only Request variants
//! (`ComputeRecencyFactor`, `ProbePhase`, `PHASE_ORDER`). The library crate
//! is compiled WITHOUT `cfg(test)` active when producing integration-test
//! binaries, so this file is only compiled under `--features bench`.
//!
//! Run with: `cargo test -p forge-daemon --test forge_identity_harness --features bench`

#![cfg(feature = "bench")]

use forge_daemon::bench::forge_identity::{run_bench, BenchConfig};

#[tokio::test]
async fn forge_identity_skeleton_runs_without_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = BenchConfig {
        seed: 42,
        output_dir: tmp.path().to_path_buf(),
        expected_composite: None,
    };
    let score = run_bench(cfg).await.expect("run_bench returns Ok");

    assert_eq!(
        score.infrastructure_checks.len(),
        14,
        "master v6 §6 mandates 14 infrastructure assertions"
    );
}
