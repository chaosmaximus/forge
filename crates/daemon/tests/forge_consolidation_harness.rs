//! Integration test: Forge-Consolidation harness runs end-to-end.

use forge_daemon::bench::forge_consolidation::{run, ConsolidationBenchConfig};

#[test]
fn test_forge_consolidation_completes_on_seed_42() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = ConsolidationBenchConfig {
        seed: 42,
        output_dir: tmp.path().to_path_buf(),
        expected_recall_delta: None,
    };
    let score = run(cfg).expect("bench should complete without panicking");

    // Artifacts written
    for f in &["summary.json", "baseline.json", "post.json", "repro.sh"] {
        assert!(tmp.path().join(f).exists(), "missing artifact: {f}");
    }

    // Score is a sane float in [0, 1]
    assert!(score.composite.is_finite());
    assert!(
        score.composite >= 0.0 && score.composite <= 1.0,
        "composite={} out of [0,1]",
        score.composite
    );

    // All 5 dimensions present
    for name in &[
        "dedup_quality",
        "contradiction_handling",
        "reweave_enrichment",
        "quality_lifecycle",
        "recall_improvement",
    ] {
        assert!(
            score.dimensions.contains_key(*name),
            "missing dimension: {name}"
        );
    }

    // For calibration runs, we do NOT assert score.pass — first run is expected to score below 1.0
}
