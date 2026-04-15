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
    canonical_hash, generate_workload, verify_matches, Operation, PersistConfig, PersistHarness,
    WorkloadConfig, HARNESS_SOURCE,
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

    // Cycle (g2): execute_op must now populate content_hash with the
    // canonical SHA-256 of the op's payload per design doc §6.2. An
    // empty content_hash (the f2 placeholder) fails this assertion
    // loudly — this is the RED test that drives g2.
    assert_eq!(
        ack.content_hash,
        canonical_hash(&ops[0]),
        "content_hash must equal canonical_hash(op) — cycle g2 wiring"
    );
    assert_eq!(
        ack.content_hash.len(),
        64,
        "SHA-256 hex digest is exactly 64 chars"
    );

    harness.kill().expect("kill should succeed");
}

#[test]
fn test_persist_harness_list_raw_documents_returns_acked_doc() {
    // Phase 2A-1 Forge-Persist cycle (j1.1): cycle (j) verify_matches
    // composer needs to enumerate raw documents the harness ingested.
    // This drives a new HttpClient::list_raw_documents helper that
    // wraps Request::RawDocumentsList from cycle (j0).
    //
    // The test seeds one IngestRaw op via execute_op, then queries the
    // listing endpoint and asserts the doc is recovered with the same
    // id and a verbatim text round-trip — the latter is load-bearing
    // for verify_matches because the post-restart consistency_rate
    // computation re-hashes the recovered text and compares against
    // the pre-kill canonical_hash.
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    let config = PersistConfig {
        daemon_bin,
        memories: 0,
        chunks: 1,
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

    // The raw layer embedder loads asynchronously in a background task
    // (see daemon/src/main.rs:265). Wait for it before the first
    // RawIngest call, otherwise the daemon returns
    // "embedder not initialized" until the MiniLM model is ready.
    harness
        .client()
        .wait_for_raw_layer(Duration::from_secs(30))
        .expect("raw layer should become ready within timeout");

    let ops = generate_workload(&WorkloadConfig {
        seed: 42,
        memories: 0,
        chunks: 1,
        fisp_messages: 0,
    });
    assert_eq!(ops.len(), 1, "workload should produce exactly 1 raw op");
    let ack = harness
        .client()
        .execute_op(&ops[0])
        .expect("execute_op should succeed against real daemon");

    // New helper: list raw documents tagged with the harness source.
    let docs = harness
        .client()
        .list_raw_documents(HARNESS_SOURCE)
        .expect("list_raw_documents should succeed");
    assert_eq!(
        docs.len(),
        1,
        "expected exactly one acked doc, got {} ({docs:?})",
        docs.len()
    );
    let doc = &docs[0];
    assert_eq!(doc.id, ack.id, "listed doc id should match ack id");
    assert_eq!(doc.source, HARNESS_SOURCE);

    // Verbatim text round-trip — re-hashing the listed text via
    // canonical_hash on a synthetic Operation::IngestRaw must match
    // the pre-kill ack hash. This is the consistency_rate invariant
    // the cycle (j2) orchestrator depends on.
    let rehashed = canonical_hash(&Operation::IngestRaw {
        index: 0,
        content: doc.text.clone(),
    });
    assert_eq!(
        rehashed, ack.content_hash,
        "listed doc text should re-hash to the same content_hash as the original op"
    );

    harness.kill().expect("kill should succeed");
}

#[test]
fn test_persist_harness_export_memories_returns_acked_memory() {
    // Phase 2A-1 Forge-Persist cycle (j1.2): memories arm of
    // verify_matches. The daemon's `Export` endpoint returns all
    // memories globally; the harness runs in a fresh TempDir so the
    // returned set is exactly the harness-ingested memories. This
    // drives a new HttpClient::export_memories helper.
    //
    // The verbatim content round-trip is the load-bearing assertion:
    // re-hashing the recovered content via canonical_hash on a synthetic
    // Operation::Remember must reproduce the pre-kill ack hash. If the
    // daemon ever rewrites memory.content (normalization, trimming,
    // markdown processing), this test catches it loudly.
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
    assert_eq!(ops.len(), 1, "workload should produce exactly 1 memory op");
    let ack = harness
        .client()
        .execute_op(&ops[0])
        .expect("execute_op should succeed against real daemon");

    // New helper: export all memories from the daemon.
    let memories = harness
        .client()
        .export_memories()
        .expect("export_memories should succeed");
    assert_eq!(
        memories.len(),
        1,
        "fresh TempDir daemon should hold exactly the one harness memory, got {memories:?}"
    );
    let m = &memories[0];
    assert_eq!(
        m.memory.id, ack.id,
        "exported memory id should match ack id"
    );

    // Reconstruct the Remember op from the recovered content and
    // confirm the canonical hash round-trips. The other Operation
    // fields (index, memory_type, title, tags) do not feed
    // canonical_hash, so any placeholder values are safe.
    let rehashed = canonical_hash(&Operation::Remember {
        index: 0,
        memory_type: "decision".to_string(),
        title: String::new(),
        content: m.memory.content.clone(),
        tags: vec![],
    });
    assert_eq!(
        rehashed, ack.content_hash,
        "exported memory content should re-hash to the same content_hash as the original op"
    );

    harness.kill().expect("kill should succeed");
}

#[test]
fn test_persist_harness_list_session_messages_returns_acked_fisp_message() {
    // Phase 2A-1 Forge-Persist cycle (j1.3a): FISP arm of
    // verify_matches. The daemon's `SessionMessages` endpoint filters
    // by `to_session`, so the harness must query each pool session
    // separately. This drives a new HttpClient::list_session_messages
    // helper.
    //
    // The parts round-trip is the load-bearing assertion — re-hashing
    // serde_json::to_string(&parts) of the recovered MessagePart vec
    // must reproduce the pre-kill canonical_hash. Catches any future
    // serde-derive change that perturbs field ordering or adds new
    // optional fields with a serializer side-effect.
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    let config = PersistConfig {
        daemon_bin,
        memories: 0,
        chunks: 0,
        fisp_messages: 1,
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
        memories: 0,
        chunks: 0,
        fisp_messages: 1,
    });
    assert_eq!(ops.len(), 1, "workload should produce exactly 1 FISP op");
    let op = &ops[0];
    let to_session = match op {
        Operation::FispSend { to_session, .. } => to_session.clone(),
        other => panic!("expected FispSend op, got {other:?}"),
    };
    let ack = harness
        .client()
        .execute_op(op)
        .expect("execute_op should succeed against real daemon");

    // New helper: list session messages for a single pool session.
    let messages = harness
        .client()
        .list_session_messages(&to_session)
        .expect("list_session_messages should succeed");
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one acked FISP message, got {messages:?}"
    );
    let m = &messages[0];
    assert_eq!(m.id, ack.id, "listed message id should match ack id");
    assert_eq!(m.to_session, to_session);

    // Reconstruct the FispSend op from the recovered parts and confirm
    // the canonical hash round-trips. canonical_hash for FispSend hashes
    // serde_json::to_string(&fisp_parts(content)). We pass the original
    // op's content so canonical_hash recomputes the same bytes, then
    // compare against ack.content_hash. The actual round-trip property
    // we care about: does the daemon preserve the parts bytes?
    let parts_json = serde_json::to_string(&m.parts).expect("parts must serialize");
    let original_parts_json = match op {
        Operation::FispSend { content, .. } => {
            let parts = vec![forge_core::protocol::MessagePart {
                kind: "text".to_string(),
                text: Some(content.clone()),
                path: None,
                data: None,
                memory_id: None,
            }];
            serde_json::to_string(&parts).expect("parts must serialize")
        }
        _ => unreachable!(),
    };
    assert_eq!(
        parts_json, original_parts_json,
        "daemon-stored parts should round-trip byte-identically with the harness-side serialization"
    );

    harness.kill().expect("kill should succeed");
}

#[test]
fn test_persist_harness_verify_matches_unions_all_three_op_types() {
    // Phase 2A-1 Forge-Persist cycle (j1.3b): the verify_matches
    // composer drives the consistency_rate / recovery_rate inputs by
    // unioning the three list helpers (raw / memories / FISP). This
    // is the function the cycle (j2) orchestrator calls after the
    // daemon restart to compute the durability metrics.
    //
    // Test seeds 1 of each op type (3 ops total), runs verify_matches,
    // and asserts the returned (visible, content) tuple holds all 3
    // ids and their reconstructed hashes match the pre-kill acks.
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    let config = PersistConfig {
        daemon_bin,
        memories: 1,
        chunks: 1,
        fisp_messages: 1,
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
    harness
        .client()
        .wait_for_raw_layer(Duration::from_secs(30))
        .expect("raw layer should become ready within timeout");

    let ops = generate_workload(&WorkloadConfig {
        seed: 42,
        memories: 1,
        chunks: 1,
        fisp_messages: 1,
    });
    assert_eq!(ops.len(), 3, "workload should produce exactly 3 ops");

    let mut acks = Vec::with_capacity(ops.len());
    for op in &ops {
        let ack = harness
            .client()
            .execute_op(op)
            .expect("execute_op should succeed");
        acks.push(ack);
    }

    let (visible, content) = verify_matches(harness.client())
        .expect("verify_matches should succeed against real daemon");
    assert_eq!(
        visible.len(),
        3,
        "verify_matches should see all 3 acked ids, got {visible:?}"
    );
    for ack in &acks {
        assert!(
            visible.contains(&ack.id),
            "visible set missing acked id {} ({visible:?})",
            ack.id
        );
        assert_eq!(
            content.get(&ack.id),
            Some(&ack.content_hash),
            "content map mismatch for id {}: expected {}, got {:?}",
            ack.id,
            ack.content_hash,
            content.get(&ack.id)
        );
    }

    harness.kill().expect("kill should succeed");
}

#[test]
fn test_persist_harness_full_run_passes_on_clean_workload() {
    // Phase 2A-1 Forge-Persist cycle (j2.1): the canonical end-to-end
    // integration test from design doc §9. Spawns a real daemon, runs
    // a small mixed workload (3 memories + 2 raw + 2 FISP), SIGKILLs
    // at 50% of total ops, restarts, runs verify_matches, computes
    // recovery + consistency + recovery_time, and asserts the run
    // produces correct metrics per design §6.4.
    //
    // Uses the §9-recommended CI-loose recovery_time threshold of
    // 10 seconds (vs the 5s production threshold on PersistScore)
    // because GitHub Actions cold-start runners need headroom for
    // double daemon spawn + embedder load. The test does NOT assert
    // `summary.passed` — that uses the strict production score_run
    // thresholds. The CLI is responsible for enforcing those.
    let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
    let config = PersistConfig {
        daemon_bin,
        memories: 3,
        chunks: 2,
        fisp_messages: 2,
        seed: 1,
        kill_after: 0.5,
        recovery_timeout: Duration::from_secs(30),
        worker_catchup: Duration::from_secs(5),
        output_dir: None,
    };

    let summary = forge_daemon::bench::forge_persist::run(config)
        .expect("full run should complete without crashing");

    assert_eq!(summary.total_ops, 7, "workload should be 3+2+2=7 ops");
    assert!(summary.wall_time_ms > 0, "wall time must be positive");
    assert!(
        summary.recovery_rate >= 0.99,
        "recovery_rate {} < 0.99: {summary:?}",
        summary.recovery_rate
    );
    assert!(
        (summary.consistency_rate - 1.0).abs() < f64::EPSILON,
        "consistency_rate must be exactly 1.0 (no orphans), got {}: {summary:?}",
        summary.consistency_rate
    );
    assert!(
        summary.recovery_time_ms < 10_000,
        "recovery_time_ms {} exceeds CI-loose 10s threshold (§9): {summary:?}",
        summary.recovery_time_ms
    );
}
