use forge_daemon::bench::forge_context::{run, ContextConfig};

#[test]
fn test_context_harness_passes_on_clean_workload() {
    let config = ContextConfig {
        seed: 42,
        output_dir: None,
    };
    let score = run(config).expect("harness should not error");
    assert!(score.composite > 0.0, "composite must be positive on a seeded corpus");
    assert_eq!(score.tool_filter_accuracy, 1.0, "tool filtering must be exact");
    assert!(score.total_queries > 0, "must have queries");
}
