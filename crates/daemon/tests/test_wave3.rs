use forge_core::protocol::*;
use forge_core::types::{CodeFile, CodeSymbol, MemoryType};
use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};

#[test]
fn test_confidence_decay_persists_and_does_not_refade() {
    let state = DaemonState::new(":memory:").unwrap();

    // Insert 90-day-old memory (effective = 0.9 * exp(-0.03*90) ~ 0.06 < 0.1 → should fade)
    state.conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
         VALUES ('d1', 'decision', 'Old decision', 'content', 0.9, 'active', '[]',
                 datetime('now', '-90 days'), datetime('now', '-90 days'))",
        [],
    ).unwrap();

    // Insert 30-day-old memory (effective = 0.9 * exp(-0.03*30) ~ 0.37 > 0.1 → stays active)
    state.conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
         VALUES ('d2', 'decision', 'Mid decision', 'content', 0.9, 'active', '[]',
                 datetime('now', '-30 days'), datetime('now', '-30 days'))",
        [],
    ).unwrap();

    let (checked, faded) = ops::decay_memories(&state.conn, 1000, 14.0).unwrap();
    assert_eq!(checked, 2);
    assert_eq!(faded, 1, "90-day memory should be faded");

    // d1 fades (status set to faded); confidence column is left at 0.9 for faded memories
    let conf: f64 = state
        .conn
        .query_row("SELECT confidence FROM memory WHERE id = 'd1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        (conf - 0.9).abs() < 0.001,
        "faded memory's stored confidence column is not modified, got {conf}"
    );

    // d2 is 30 days old — confidence is persisted as the decayed value (~0.3659)
    let conf2: f64 = state
        .conn
        .query_row("SELECT confidence FROM memory WHERE id = 'd2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let expected_d2 = 0.9_f64 * (-0.03_f64 * 30.0_f64).exp();
    assert!(
        (conf2 - expected_d2).abs() < 0.02,
        "d2 confidence should be persisted as decayed value ~{expected_d2:.4}, got {conf2}"
    );

    // Status checks
    let s1: String = state
        .conn
        .query_row("SELECT status FROM memory WHERE id = 'd1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(s1, "faded");
    let s2: String = state
        .conn
        .query_row("SELECT status FROM memory WHERE id = 'd2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(s2, "active");

    // Running decay again: d1 is already faded (excluded from query), d2 stays active (conf > 0.1)
    let (checked2, faded2) = ops::decay_memories(&state.conn, 1000, 14.0).unwrap();
    assert_eq!(checked2, 1, "only d2 is still active after first run");
    assert_eq!(faded2, 0, "d2 should not fade on second run");
}

#[test]
fn test_code_storage_and_doctor() {
    let mut state = DaemonState::new(":memory:").unwrap();

    // Store code files + symbols
    let file = CodeFile {
        id: "f1".into(),
        path: "src/main.rs".into(),
        language: "rust".into(),
        project: "test".into(),
        hash: "abc".into(),
        indexed_at: "now".into(),
    };
    ops::store_file(&state.conn, &file).unwrap();

    let sym = CodeSymbol {
        id: "s1".into(),
        name: "main".into(),
        kind: "function".into(),
        file_path: "src/main.rs".into(),
        line_start: 1,
        line_end: Some(10),
        signature: Some("fn main()".into()),
    };
    ops::store_symbol(&state.conn, &sym).unwrap();

    // Doctor should report the counts
    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok {
            data:
                ResponseData::Doctor {
                    daemon_up,
                    file_count,
                    symbol_count,
                    ..
                },
        } => {
            assert!(daemon_up);
            assert_eq!(file_count, 1);
            assert_eq!(symbol_count, 1);
        }
        other => panic!("expected Doctor, got: {other:?}"),
    }
}

#[test]
fn test_doctor_via_handler() {
    let mut state = DaemonState::new(":memory:").unwrap();
    // Remember some memories first
    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Test".into(),
            content: "Content".into(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: None,
        },
    );
    assert!(matches!(resp, Response::Ok { .. }));

    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok {
            data:
                ResponseData::Doctor {
                    memory_count,
                    daemon_up,
                    workers,
                    ..
                },
        } => {
            assert!(daemon_up);
            assert_eq!(memory_count, 1);
            assert!(!workers.is_empty());
        }
        other => panic!("expected Doctor, got: {other:?}"),
    }
}
