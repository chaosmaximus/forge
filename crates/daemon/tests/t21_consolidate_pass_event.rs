//! Phase 2A-4d.2 T5 integration test — `consolidate_pass_completed` event.
//!
//! Subscribes to the daemon broadcast bus, runs `run_all_phases` on a seeded
//! DB, and asserts that exactly one `consolidate_pass_completed` event is
//! received within a short timeout with the expected v1 payload shape.

use forge_daemon::db::schema::create_schema;
use forge_daemon::db::vec::init_sqlite_vec;
use forge_daemon::events;
use forge_daemon::workers::consolidator;
use rusqlite::Connection;
use std::time::Duration;

fn make_conn() -> Connection {
    init_sqlite_vec();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn).unwrap();
    conn
}

#[tokio::test]
async fn consolidate_pass_completed_emits_one_event_per_pass() {
    let conn = make_conn();
    let tx = events::create_event_bus();
    let mut rx = tx.subscribe();

    let cfg = forge_daemon::config::ConsolidationConfig::default();
    let _stats = consolidator::run_all_phases(&conn, &cfg, None, Some(&tx));

    // Drain events until we find our target or time out.
    let mut saw_consolidate_pass_completed = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let ev = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        match ev {
            Ok(Ok(event)) => {
                if event.event == "consolidate_pass_completed" {
                    saw_consolidate_pass_completed += 1;

                    // Validate v1 payload contract.
                    let d = &event.data;
                    assert_eq!(d["event_schema_version"], 1);
                    assert!(d["run_id"].is_string(), "run_id missing or wrong type");
                    assert!(
                        d["correlation_id"].is_string(),
                        "correlation_id missing or wrong type"
                    );
                    assert!(
                        d["trace_id"].is_null() || d["trace_id"].is_string(),
                        "trace_id should be null or string"
                    );
                    assert_eq!(
                        d["phase_count"], 23,
                        "phase_count should match PHASE_SPAN_NAMES.len()"
                    );
                    assert!(
                        d["pass_wall_duration_ms"].is_u64(),
                        "pass_wall_duration_ms missing"
                    );
                    assert!(
                        d["stats"].is_object(),
                        "stats should be serialized ConsolidationStats"
                    );

                    // ConsolidationStats has 20 usize fields — spot-check one.
                    assert!(d["stats"]["exact_dedup"].is_u64());
                    assert!(d["stats"]["healed_quality_adjusted"].is_u64());
                }
                // Other events (extraction, consolidation summary, etc.) are ignored.
            }
            Ok(Err(_)) => break, // channel closed
            Err(_) => break,     // per-recv timeout; let the outer loop decide
        }
        if saw_consolidate_pass_completed > 0 {
            // Give the bus a brief settle window to catch any duplicates.
            tokio::time::sleep(Duration::from_millis(100)).await;
            while let Ok(Ok(event)) =
                tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
            {
                if event.event == "consolidate_pass_completed" {
                    saw_consolidate_pass_completed += 1;
                }
            }
            break;
        }
    }

    assert_eq!(
        saw_consolidate_pass_completed, 1,
        "expected exactly one consolidate_pass_completed event per run_all_phases invocation"
    );
}

#[tokio::test]
async fn consolidate_pass_completed_not_emitted_when_events_is_none() {
    let conn = make_conn();
    let tx = events::create_event_bus();
    let mut rx = tx.subscribe();

    let cfg = forge_daemon::config::ConsolidationConfig::default();
    let _stats = consolidator::run_all_phases(&conn, &cfg, None, None);

    // Short probe: if we don't see the event in 300ms we call it good.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            assert_ne!(
                event.event, "consolidate_pass_completed",
                "should not receive event when events param is None"
            );
        }
    }
}
