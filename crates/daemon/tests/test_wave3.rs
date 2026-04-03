use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::migrate::import_v1_cache;
use forge_v2_core::protocol::*;
use forge_v2_core::types::{MemoryType, CodeFile, CodeSymbol};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_confidence_decay() {
    let state = DaemonState::new(":memory:").unwrap();
    // Insert memory with old accessed_at (60 days ago)
    state.conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
         VALUES ('d1', 'decision', 'Old decision', 'content', 0.9, 'active', '[]',
                 datetime('now', '-60 days'), datetime('now', '-60 days'))",
        [],
    ).unwrap();

    let (decayed, _) = ops::decay_memories(&state.conn).unwrap();
    assert!(decayed >= 1);

    let conf: f64 = state.conn.query_row("SELECT confidence FROM memory WHERE id = 'd1'", [], |r| r.get(0)).unwrap();
    assert!(conf < 0.5, "60-day-old memory should be below 0.5: got {}", conf);
}

#[test]
fn test_migrate_and_recall() {
    let state = DaemonState::new(":memory:").unwrap();
    let cache = r#"{"entries":[
        {"type":"decision","title":"Use PostgreSQL","content":"ACID compliance","confidence":0.95,"status":"active"},
        {"type":"lesson","title":"Avoid MongoDB","content":"Schema issues","confidence":0.8,"status":"active"}
    ]}"#;
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "{}", cache).unwrap();

    let (imported, _) = import_v1_cache(&state.conn, tmp.path().to_str().unwrap()).unwrap();
    assert_eq!(imported, 2);

    let results = ops::recall_bm25(&state.conn, "PostgreSQL", 10).unwrap();
    assert!(!results.is_empty(), "should recall PostgreSQL after migration");
}

#[test]
fn test_code_storage_and_doctor() {
    let mut state = DaemonState::new(":memory:").unwrap();

    // Store code files + symbols
    let file = CodeFile {
        id: "f1".into(), path: "src/main.rs".into(), language: "rust".into(),
        project: "test".into(), hash: "abc".into(), indexed_at: "now".into(),
    };
    ops::store_file(&state.conn, &file).unwrap();

    let sym = CodeSymbol {
        id: "s1".into(), name: "main".into(), kind: "function".into(),
        file_path: "src/main.rs".into(), line_start: 1, line_end: Some(10),
        signature: Some("fn main()".into()),
    };
    ops::store_symbol(&state.conn, &sym).unwrap();

    // Doctor should report the counts
    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok { data: ResponseData::Doctor { daemon_up, file_count, symbol_count, .. } } => {
            assert!(daemon_up);
            assert_eq!(file_count, 1);
            assert_eq!(symbol_count, 1);
        }
        other => panic!("expected Doctor, got: {:?}", other),
    }
}

#[test]
fn test_doctor_via_handler() {
    let mut state = DaemonState::new(":memory:").unwrap();
    // Remember some memories first
    let resp = handle_request(&mut state, Request::Remember {
        memory_type: MemoryType::Decision,
        title: "Test".into(),
        content: "Content".into(),
        confidence: None,
        tags: None,
        project: None,
    });
    assert!(matches!(resp, Response::Ok { .. }));

    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok { data: ResponseData::Doctor { memory_count, daemon_up, workers, .. } } => {
            assert!(daemon_up);
            assert_eq!(memory_count, 1);
            assert!(!workers.is_empty());
        }
        other => panic!("expected Doctor, got: {:?}", other),
    }
}
