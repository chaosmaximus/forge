//! Integration test: Forge-Identity skeleton compiles + runs (Phase 2A-4d.3 T2).
//!
//! This exercises the fail-fast path — the T2 stub returns 14 failing
//! infrastructure checks, so `run_bench` short-circuits with `pass: false`
//! and zeroed dimension scores. T6 flips the checks to real assertions
//! and unblocks the happy path.

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

    assert!(!score.pass, "T2 stub cannot pass overall");
    assert_eq!(
        score.infrastructure_checks.len(),
        14,
        "master v6 §6 mandates 14 infrastructure assertions"
    );
}
