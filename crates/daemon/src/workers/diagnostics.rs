//! Diagnostics worker — debounced batch analysis after agent edit turns.
//!
//! Accumulates edited file paths via an mpsc channel. After 3 seconds of
//! silence (no new files), runs batch analysis on ALL accumulated files:
//! 1. Cross-file consistency check (callers of changed symbols)
//! 2. Memory-informed repeat-bug check
//! 3. Store results in diagnostic table
//! 4. Emit diagnostics_ready event

use crate::db::diagnostics::{self, Diagnostic};
use crate::server::handler::DaemonState;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, Mutex};

const DEBOUNCE_SECS: u64 = 3;

/// Run the diagnostics worker loop.
///
/// Receives file paths from the PostEdit handler via `file_rx`, debounces
/// for `DEBOUNCE_SECS` of silence, then runs batch analysis.
pub async fn run_diagnostics_worker(
    state: Arc<Mutex<DaemonState>>,
    mut file_rx: mpsc::Receiver<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    db_path: String,
) {
    let mut pending_files: HashSet<String> = HashSet::new();
    eprintln!(
        "[diagnostics] ready, waiting for files ({}s debounce)...",
        DEBOUNCE_SECS
    );

    loop {
        // Wait for the first file event or shutdown
        tokio::select! {
            Some(file) = file_rx.recv() => {
                pending_files.insert(file);
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    // Process any pending files before shutdown
                    if !pending_files.is_empty() {
                        let files: Vec<String> = pending_files.drain().collect();
                        run_batch_analysis(&state, &files, &db_path).await;
                    }
                    eprintln!("[diagnostics] shutdown received");
                    return;
                }
            }
        }

        // Activity gap debounce: keep collecting events for DEBOUNCE_SECS of silence.
        // Max wait of 30s prevents starvation under continuous activity (Codex fix).
        let max_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let debounce_timeout = tokio::time::sleep(Duration::from_secs(DEBOUNCE_SECS));
            let max_timeout = tokio::time::sleep_until(max_deadline);
            tokio::select! {
                Some(file) = file_rx.recv() => {
                    pending_files.insert(file);
                    // Reset the debounce timer (keep waiting for silence)
                }
                _ = debounce_timeout => {
                    // Silence period reached — process all pending files
                    break;
                }
                _ = max_timeout => {
                    // Max wait reached — process even if still receiving files
                    eprintln!("[diagnostics] max wait reached, processing {} files", pending_files.len());
                    break;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        if !pending_files.is_empty() {
                            let files: Vec<String> = pending_files.drain().collect();
                            run_batch_analysis(&state, &files, &db_path).await;
                        }
                        eprintln!("[diagnostics] shutdown during debounce");
                        return;
                    }
                }
            }
        }

        // Process the batch
        let files: Vec<String> = pending_files.drain().collect();
        run_batch_analysis(&state, &files, &db_path).await;
    }
}

async fn run_batch_analysis(state: &Arc<Mutex<DaemonState>>, files: &[String], db_path: &str) {
    eprintln!("[diagnostics] batch analysis on {} files", files.len());

    // For :memory: databases (tests), fall back to state lock for all operations.
    // Read-only connections can't share data with in-memory databases.
    let use_read_conn = db_path != ":memory:";

    for file in files {
        if use_read_conn {
            // Brief lock for the DELETE (clear old diagnostics)
            {
                let locked = state.lock().await;
                if let Err(e) = diagnostics::clear_diagnostics(&locked.conn, file) {
                    eprintln!("[diagnostics] failed to clear diagnostics for {}: {e}", file);
                }
            } // lock released

            // Read-only queries + write lock per diagnostic stored
            {
                let read_conn = super::open_read_conn(db_path);
                let diags = collect_consistency_diagnostics(&read_conn, file);
                let bug_diags = collect_repeat_bug_diagnostics(&read_conn, file);
                drop(read_conn);

                // Brief lock for storing diagnostics
                if !diags.is_empty() || !bug_diags.is_empty() {
                    let locked = state.lock().await;
                    for d in &diags {
                        if let Err(e) = diagnostics::store_diagnostic(&locked.conn, d) {
                            eprintln!("[diagnostics] failed to store consistency diagnostic: {e}");
                        }
                    }
                    for d in &bug_diags {
                        if let Err(e) = diagnostics::store_diagnostic(&locked.conn, d) {
                            eprintln!("[diagnostics] failed to store repeat-bug diagnostic: {e}");
                        }
                    }
                }
            }
        } else {
            // Fallback: use state lock for everything (tests with :memory: databases)
            let locked = state.lock().await;
            if let Err(e) = diagnostics::clear_diagnostics(&locked.conn, file) {
                eprintln!("[diagnostics] failed to clear diagnostics for {}: {e}", file);
            }
            let diags = collect_consistency_diagnostics(&locked.conn, file);
            let bug_diags = collect_repeat_bug_diagnostics(&locked.conn, file);
            for d in &diags {
                if let Err(e) = diagnostics::store_diagnostic(&locked.conn, d) {
                    eprintln!("[diagnostics] failed to store consistency diagnostic: {e}");
                }
            }
            for d in &bug_diags {
                if let Err(e) = diagnostics::store_diagnostic(&locked.conn, d) {
                    eprintln!("[diagnostics] failed to store repeat-bug diagnostic: {e}");
                }
            }
            drop(locked);
        }
    }

    // Emit event (brief lock to get event sender)
    let event_tx = {
        let locked = state.lock().await;
        locked.events.clone()
    };
    crate::events::emit(
        &event_tx,
        "diagnostics_ready",
        serde_json::json!({
            "files": files,
            "count": files.len(),
        }),
    );
}

/// Cross-file consistency check: finds symbols defined in the edited file,
/// looks for callers from OTHER files via "calls" edges, and stores a
/// diagnostic warning when callers exist.
#[cfg(test)]
pub(crate) fn run_consistency_check(conn: &Connection, file: &str) {
    let diags = collect_consistency_diagnostics(conn, file);
    for d in &diags {
        if let Err(e) = diagnostics::store_diagnostic(conn, d) {
            eprintln!("[diagnostics] failed to store consistency diagnostic: {e}");
        }
    }
}

/// Collect consistency diagnostics (read-only). Returns diagnostics without storing.
fn collect_consistency_diagnostics(conn: &Connection, file: &str) -> Vec<Diagnostic> {
    let like_pattern = format!("%{}", file);
    let mut result = Vec::new();

    // Find symbols defined in this file
    let symbols: Vec<(String, String)> = conn
        .prepare(
            "SELECT id, name FROM code_symbol WHERE file_path LIKE ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![like_pattern], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect()
        })
        .unwrap_or_default();

    // For each symbol, find callers from OTHER files
    for (sym_id, sym_name) in &symbols {
        let caller_count: i64 = conn
            .prepare(
                "SELECT COUNT(DISTINCT cs2.file_path)
                 FROM edge e
                 JOIN code_symbol cs2 ON cs2.id = e.from_id
                 WHERE e.to_id = ?1 AND e.edge_type = 'calls'
                 AND cs2.file_path NOT LIKE ?2",
            )
            .and_then(|mut stmt| {
                stmt.query_row(params![sym_id, like_pattern], |row| row.get(0))
            })
            .unwrap_or(0);

        if caller_count > 0 {
            result.push(Diagnostic {
                id: format!("consistency-{}", sym_id),
                file_path: file.to_string(),
                severity: "warning".into(),
                message: format!(
                    "{} files call {}() -- verify callers are updated",
                    caller_count, sym_name
                ),
                source: "forge-consistency".into(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(300), // 5 min TTL
            });
        }
    }

    result
}

/// Memory-informed repeat-bug detector: matches the edited file against
/// high-intensity negative-valence lessons in memory.
#[cfg(test)]
pub(crate) fn run_repeat_bug_check(conn: &Connection, file: &str) {
    let diags = collect_repeat_bug_diagnostics(conn, file);
    for d in &diags {
        if let Err(e) = diagnostics::store_diagnostic(conn, d) {
            eprintln!("[diagnostics] failed to store repeat-bug diagnostic: {e}");
        }
    }
}

/// Collect repeat-bug diagnostics (read-only). Returns diagnostics without storing.
fn collect_repeat_bug_diagnostics(conn: &Connection, file: &str) -> Vec<Diagnostic> {
    let file_stem = file.rsplit('/').next().unwrap_or(file);
    let dir = file.rsplit('/').nth(1).unwrap_or("");
    let mut result = Vec::new();

    let search_terms = [format!("%{}%", file_stem), format!("%{}%", dir)];

    for search in &search_terms {
        if search == "%%" {
            continue; // skip empty directory search
        }

        let lessons: Vec<(String, f64)> = conn
            .prepare(
                "SELECT title, intensity FROM memory
                 WHERE status = 'active' AND valence = 'negative' AND intensity > 0.5
                 AND (title LIKE ?1 OR content LIKE ?1 OR tags LIKE ?1)
                 ORDER BY intensity DESC LIMIT 2",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![search], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect()
            })
            .unwrap_or_default();

        for (title, intensity) in &lessons {
            result.push(Diagnostic {
                id: format!("repeat-bug-{}-{}", file_stem, title.len()),
                file_path: file.to_string(),
                severity: if *intensity > 0.8 {
                    "error"
                } else {
                    "warning"
                }
                .into(),
                message: format!("Past issue: {} (intensity: {:.1})", title, intensity),
                source: "forge-memory".into(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(300),
            });
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ops::{remember, store_edge, store_symbol};
    use crate::db::schema::create_schema;
    use forge_core::types::code::CodeSymbol;
    use forge_core::types::{Memory, MemoryType};

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_consistency_check_detects_callers() {
        let conn = setup();

        // Create a symbol in the target file
        let sym = CodeSymbol {
            id: "sym-validate".into(),
            name: "validate_token".into(),
            kind: "function".into(),
            file_path: "/project/src/auth.rs".into(),
            line_start: 10,
            line_end: Some(20),
            signature: Some("fn validate_token(token: &str) -> bool".into()),
        };
        store_symbol(&conn, &sym).unwrap();

        // Create a caller symbol in a DIFFERENT file
        let caller = CodeSymbol {
            id: "sym-handler".into(),
            name: "handle_request".into(),
            kind: "function".into(),
            file_path: "/project/src/handler.rs".into(),
            line_start: 5,
            line_end: Some(15),
            signature: None,
        };
        store_symbol(&conn, &caller).unwrap();

        // Create a "calls" edge from handler to validate_token
        store_edge(&conn, "sym-handler", "sym-validate", "calls", "{}").unwrap();

        // Run the consistency check
        run_consistency_check(&conn, "src/auth.rs");

        // Should have stored a diagnostic
        let diags = diagnostics::get_diagnostics(&conn, "src/auth.rs").unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, "warning");
        assert!(diags[0].message.contains("validate_token"));
        assert!(diags[0].message.contains("1 files call"));
        assert_eq!(diags[0].source, "forge-consistency");
    }

    #[test]
    fn test_consistency_check_no_callers() {
        let conn = setup();

        // Create a symbol with no callers
        let sym = CodeSymbol {
            id: "sym-lonely".into(),
            name: "lonely_func".into(),
            kind: "function".into(),
            file_path: "/project/src/utils.rs".into(),
            line_start: 1,
            line_end: Some(5),
            signature: None,
        };
        store_symbol(&conn, &sym).unwrap();

        run_consistency_check(&conn, "src/utils.rs");

        let diags = diagnostics::get_diagnostics(&conn, "src/utils.rs").unwrap();
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn test_repeat_bug_check_finds_matching_lesson() {
        let conn = setup();

        // Store a negative-valence lesson about auth
        let mut mem = Memory::new(
            MemoryType::Lesson,
            "auth.rs token validation bug",
            "The validate_token function had a timing attack vulnerability",
        );
        mem = mem.with_tags(vec!["auth".into(), "security".into()]);
        remember(&conn, &mem).unwrap();
        // Set negative valence and high intensity
        conn.execute(
            "UPDATE memory SET valence = 'negative', intensity = 0.9 WHERE id = ?1",
            params![mem.id],
        )
        .unwrap();

        // Run repeat-bug check on auth.rs
        run_repeat_bug_check(&conn, "src/auth.rs");

        let diags = diagnostics::get_diagnostics(&conn, "src/auth.rs").unwrap();
        assert!(!diags.is_empty(), "should have found matching lesson");
        assert_eq!(diags[0].source, "forge-memory");
        assert!(diags[0].message.contains("auth.rs token validation bug"));
        // Intensity 0.9 > 0.8 => severity should be "error"
        assert_eq!(diags[0].severity, "error");
    }

    #[test]
    fn test_repeat_bug_check_ignores_low_intensity() {
        let conn = setup();

        // Store a low-intensity negative lesson
        let mem = Memory::new(
            MemoryType::Lesson,
            "minor utils.rs issue",
            "Small formatting problem",
        );
        remember(&conn, &mem).unwrap();
        conn.execute(
            "UPDATE memory SET valence = 'negative', intensity = 0.3 WHERE id = ?1",
            params![mem.id],
        )
        .unwrap();

        run_repeat_bug_check(&conn, "src/utils.rs");

        let diags = diagnostics::get_diagnostics(&conn, "src/utils.rs").unwrap();
        assert_eq!(diags.len(), 0, "low intensity lessons should be ignored");
    }

    #[test]
    fn test_repeat_bug_check_warning_severity() {
        let conn = setup();

        // Intensity between 0.5 and 0.8 => warning
        let mem = Memory::new(
            MemoryType::Lesson,
            "handler.rs edge case",
            "Off-by-one in handler.rs loop",
        );
        remember(&conn, &mem).unwrap();
        conn.execute(
            "UPDATE memory SET valence = 'negative', intensity = 0.7 WHERE id = ?1",
            params![mem.id],
        )
        .unwrap();

        run_repeat_bug_check(&conn, "src/handler.rs");

        let diags = diagnostics::get_diagnostics(&conn, "src/handler.rs").unwrap();
        assert!(!diags.is_empty());
        assert_eq!(diags[0].severity, "warning");
    }

    #[tokio::test]
    async fn test_debounced_worker_processes_files() {
        let state = Arc::new(Mutex::new(DaemonState::new(":memory:").unwrap()));
        let (file_tx, file_rx) = mpsc::channel::<String>(100);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn the worker
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            run_diagnostics_worker(worker_state, file_rx, shutdown_rx, ":memory:".to_string()).await;
        });

        // Send some file paths
        file_tx.send("src/main.rs".into()).await.unwrap();
        file_tx.send("src/lib.rs".into()).await.unwrap();

        // Wait for debounce (3s) + processing time
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Signal shutdown
        let _ = shutdown_tx.send(true);
        let _ = handle.await;

        // The worker should have run batch analysis (even if no diagnostics were produced,
        // the worker should have completed without error)
    }

    #[tokio::test]
    async fn test_debounced_worker_accumulates_files() {
        let conn_setup = {
            crate::db::vec::init_sqlite_vec();
            let c = Connection::open_in_memory().unwrap();
            create_schema(&c).unwrap();
            c
        };
        // Store a symbol + caller to generate a diagnostic
        let sym = CodeSymbol {
            id: "sym-target".into(),
            name: "process_data".into(),
            kind: "function".into(),
            file_path: "/project/src/data.rs".into(),
            line_start: 1,
            line_end: Some(10),
            signature: None,
        };
        store_symbol(&conn_setup, &sym).unwrap();
        let caller = CodeSymbol {
            id: "sym-caller".into(),
            name: "main_handler".into(),
            kind: "function".into(),
            file_path: "/project/src/main.rs".into(),
            line_start: 1,
            line_end: Some(5),
            signature: None,
        };
        store_symbol(&conn_setup, &caller).unwrap();
        store_edge(&conn_setup, "sym-caller", "sym-target", "calls", "{}").unwrap();
        drop(conn_setup);

        let state = Arc::new(Mutex::new(DaemonState::new(":memory:").unwrap()));
        // Re-setup the test data in the state's connection
        {
            let locked = state.lock().await;
            let sym = CodeSymbol {
                id: "sym-target".into(),
                name: "process_data".into(),
                kind: "function".into(),
                file_path: "/project/src/data.rs".into(),
                line_start: 1,
                line_end: Some(10),
                signature: None,
            };
            store_symbol(&locked.conn, &sym).unwrap();
            let caller = CodeSymbol {
                id: "sym-caller".into(),
                name: "main_handler".into(),
                kind: "function".into(),
                file_path: "/project/src/main.rs".into(),
                line_start: 1,
                line_end: Some(5),
                signature: None,
            };
            store_symbol(&locked.conn, &caller).unwrap();
            store_edge(&locked.conn, "sym-caller", "sym-target", "calls", "{}").unwrap();
            drop(locked);
        }

        let (file_tx, file_rx) = mpsc::channel::<String>(100);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            run_diagnostics_worker(worker_state, file_rx, shutdown_rx, ":memory:".to_string()).await;
        });

        // Send the file that has callers
        file_tx.send("src/data.rs".into()).await.unwrap();

        // Wait for debounce + processing
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Check diagnostics were produced
        {
            let locked = state.lock().await;
            let diags = diagnostics::get_diagnostics(&locked.conn, "src/data.rs").unwrap();
            assert!(
                !diags.is_empty(),
                "should have created consistency diagnostic"
            );
            assert!(diags[0].message.contains("process_data"));
        }

        let _ = shutdown_tx.send(true);
        let _ = handle.await;
    }
}
