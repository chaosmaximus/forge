//! End-to-end integration test for the Forge-Persist harness.
//!
//! Uses `env!("CARGO_BIN_EXE_forge-daemon")` to spawn the real daemon
//! binary as a child process, exercise the subprocess lifecycle
//! (spawn → kill), and verify that the daemon actually binds its
//! HTTP port and stops accepting connections after SIGKILL.
//!
//! Tests here run via `cargo test -p forge-daemon --test forge_persist_harness`.
//! They are NOT included in `cargo test --lib` which only runs unit tests.

use forge_daemon::bench::forge_persist::{
    generate_workload, PersistConfig, PersistHarness, WorkloadConfig,
};
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn test_persist_harness_spawns_and_kills_daemon() {
    // Phase 2A-1 Forge-Persist cycle (f1): minimum subprocess
    // lifecycle validation. Spawns a real forge-daemon subprocess
    // isolated in a TempDir via FORGE_DIR, verifies the HTTP port
    // binds, kills the subprocess, and verifies the port is no
    // longer accepting connections.
    //
    // Does NOT exercise HTTP request issuance — that comes in cycle
    // (f2) once the HttpClient wrapper lands. This test is a tight
    // smoke check for the spawn/kill primitives only.
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    assert!(
        daemon_bin.exists(),
        "CARGO_BIN_EXE_forge-daemon should point at a built binary: {daemon_bin:?}"
    );

    let config = PersistConfig {
        daemon_bin,
        memories: 0,
        chunks: 0,
        fisp_messages: 0,
        seed: 42,
        kill_after: 0.5,
        recovery_timeout: Duration::from_secs(15),
        worker_catchup: Duration::from_secs(0),
        output_dir: None,
    };

    let mut harness = PersistHarness::new(config).expect("PersistHarness::new should succeed");

    harness
        .spawn()
        .expect("spawn should succeed within timeout");
    assert!(
        harness.is_daemon_alive(),
        "daemon should be accepting TCP connections after spawn"
    );

    harness.kill().expect("kill should succeed");
    assert!(
        !harness.is_daemon_alive(),
        "daemon should reject TCP connections after kill"
    );
}

#[test]
fn test_persist_harness_executes_op_against_real_daemon() {
    // Phase 2A-1 Forge-Persist cycle (f2): end-to-end validation that
    // the HttpClient wrapper can marshal an Operation into a real HTTP
    // request against a spawned daemon and extract a non-empty ack id.
    //
    // Drives the existence of HttpClient, execute_op, AckedOp, and the
    // new HarnessError network/json/status/daemon variants. Covers the
    // "spawn daemon → generate workload → POST /api → parse ack" path
    // end-to-end with a single Remember op (the smallest non-trivial
    // workload that still exercises Response parsing).
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    let config = PersistConfig {
        daemon_bin,
        memories: 1,
        chunks: 0,
        fisp_messages: 0,
        seed: 42,
        kill_after: 1.0,
        recovery_timeout: Duration::from_secs(15),
        worker_catchup: Duration::from_secs(0),
        output_dir: None,
    };

    let mut harness = PersistHarness::new(config).expect("PersistHarness::new should succeed");
    harness
        .spawn()
        .expect("spawn should succeed within timeout");

    let ops = generate_workload(&WorkloadConfig {
        seed: 42,
        memories: 1,
        chunks: 0,
        fisp_messages: 0,
    });
    assert_eq!(ops.len(), 1, "workload should produce exactly 1 op");

    let ack = harness
        .client()
        .execute_op(&ops[0])
        .expect("execute_op should succeed against real daemon");
    assert!(
        !ack.id.is_empty(),
        "Remember ack should carry a non-empty id"
    );

    harness.kill().expect("kill should succeed");
}
