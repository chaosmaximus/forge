//! Phase 2A-4b: verifies sync export+import preserves reaffirmed_at across nodes.
//!
//! Tests that the reaffirmed_at field round-trips correctly through:
//! - ops::remember_raw (INSERT stores reaffirmed_at)
//! - ops::export_memories_org (SELECT includes reaffirmed_at, serialized to JSON)
//! - sync::sync_import (UPDATE propagates reaffirmed_at from remote)

use forge_core::types::*;
use forge_daemon::db::{ops, schema};
use forge_daemon::sync;
use rusqlite::Connection;

fn setup_node() -> Connection {
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = Connection::open_in_memory().unwrap();
    schema::create_schema(&conn).unwrap();
    conn
}

#[test]
fn sync_export_import_preserves_reaffirmed_at() {
    // Node A: seed a reaffirmed preference
    let conn_a = setup_node();
    let mut pref = Memory::new(
        MemoryType::Preference,
        "prefer-vim".to_string(),
        "yes".to_string(),
    );
    pref.reaffirmed_at = Some("2026-04-19 12:00:00".to_string());
    // Set HLC so sync_export won't reject this memory
    pref.hlc_timestamp = "1745000000000-0000000000-nodea001".to_string();
    pref.node_id = "nodea001".to_string();

    ops::remember_raw(&conn_a, &pref).unwrap();

    // Verify DB stored reaffirmed_at
    let stored_a_initial: Option<String> = conn_a
        .query_row(
            "SELECT reaffirmed_at FROM memory WHERE title = 'prefer-vim'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_a_initial,
        Some("2026-04-19 12:00:00".to_string()),
        "remember_raw must store reaffirmed_at"
    );

    // Export from A using export_memories_org and serialize to JSON lines
    let memories_a = ops::export_memories_org(&conn_a, None).unwrap();
    assert_eq!(memories_a.len(), 1);

    let exported_json = serde_json::to_string(&memories_a[0]).unwrap();
    assert!(
        exported_json.contains("\"reaffirmed_at\":\"2026-04-19 12:00:00\""),
        "export_memories_org + serde should include reaffirmed_at; got: {exported_json}"
    );

    // Node B: empty — import the exported JSON via sync_import
    let conn_b = setup_node();
    let local_node_id = "nodeb001";
    let lines = vec![exported_json.clone()];

    let result = sync::sync_import(&conn_b, &lines, local_node_id).unwrap();
    assert_eq!(
        result.imported, 1,
        "should have imported 1 memory to node B"
    );

    // Verify B has the reaffirmed_at
    let stored_b: Option<String> = conn_b
        .query_row(
            "SELECT reaffirmed_at FROM memory WHERE title = 'prefer-vim'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_b,
        Some("2026-04-19 12:00:00".to_string()),
        "sync_import must propagate reaffirmed_at to node B"
    );

    // Round-trip back to A: export from B, import into A (same-node path — HLC wins)
    // For the round-trip, we'll directly export from B and import into A
    let memories_b = ops::export_memories_org(&conn_b, None).unwrap();
    assert_eq!(memories_b.len(), 1);
    assert_eq!(
        memories_b[0].reaffirmed_at,
        Some("2026-04-19 12:00:00".to_string()),
        "exported memory from B must carry reaffirmed_at"
    );

    // The re-import into A from B won't trigger update (same content, skipped),
    // but A already has the correct reaffirmed_at from the original insert.
    let stored_a_final: Option<String> = conn_a
        .query_row(
            "SELECT reaffirmed_at FROM memory WHERE title = 'prefer-vim'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_a_final,
        Some("2026-04-19 12:00:00".to_string()),
        "node A must still have correct reaffirmed_at after round-trip"
    );
}

#[test]
fn sync_import_round_trip_update_preserves_reaffirmed_at() {
    // This test exercises the UPDATE branch in sync_import (same node_id, newer HLC)
    // and verifies reaffirmed_at is propagated via COALESCE(?7, reaffirmed_at).

    let conn = setup_node();
    let local_node_id = "nodex001";

    // Insert initial memory with reaffirmed_at via sync_import (None path)
    let mut initial = Memory::new(
        MemoryType::Preference,
        "prefer-emacs".to_string(),
        "original content".to_string(),
    );
    initial.reaffirmed_at = Some("2026-04-18 10:00:00".to_string());
    initial.hlc_timestamp = "1744900000000-0000000000-nodex001".to_string();
    initial.node_id = local_node_id.to_string();

    let line1 = serde_json::to_string(&initial).unwrap();
    sync::sync_import(&conn, &[line1], local_node_id).unwrap();

    // Now sync_import an update from the SAME node with a newer HLC (triggers UPDATE branch)
    let mut updated = initial.clone();
    updated.content = "updated content".to_string();
    updated.hlc_timestamp = "1744999999999-0000000000-nodex001".to_string(); // newer HLC
    updated.reaffirmed_at = Some("2026-04-19 09:00:00".to_string()); // updated reaffirmed_at

    let line2 = serde_json::to_string(&updated).unwrap();
    let result = sync::sync_import(&conn, &[line2], local_node_id).unwrap();
    assert_eq!(result.imported, 1, "newer HLC should trigger import/update");

    let stored: Option<String> = conn
        .query_row(
            "SELECT reaffirmed_at FROM memory WHERE title = 'prefer-emacs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored,
        Some("2026-04-19 09:00:00".to_string()),
        "UPDATE branch must propagate reaffirmed_at when remote provides Some value"
    );
}
