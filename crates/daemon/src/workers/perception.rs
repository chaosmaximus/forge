// workers/perception.rs — Environment perception worker
//
// Monitors the project environment (git status, file changes) and creates
// ephemeral Perception entries (Manas Layer 4). Perceptions auto-expire
// after 5 minutes — they represent transient sensory data, not long-term memory.

use crate::db::manas;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const PERCEPTION_INTERVAL: Duration = Duration::from_secs(30);

/// Perception expiry: 5 minutes from creation.
const PERCEPTION_TTL_SECS: i64 = 5 * 60;

pub async fn run_perception(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[perception] started, interval = {:?}", PERCEPTION_INTERVAL);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(PERCEPTION_INTERVAL) => {
                tick(&state).await;
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[perception] shutting down");
                return;
            }
        }
    }
}

async fn tick(state: &Arc<Mutex<crate::server::handler::DaemonState>>) {
    // Phase 1: Expire old perceptions (fast, keep lock briefly)
    {
        let locked = state.lock().await;
        match manas::expire_perceptions(&locked.conn) {
            Ok(expired) => {
                if expired > 0 {
                    eprintln!("[perception] expired {} old perceptions", expired);
                }
            }
            Err(e) => eprintln!("[perception] expire error: {}", e),
        }
    } // lock released

    // Phase 2: Check git status if project dir available
    let project_dir = crate::workers::indexer::find_project_dir();
    if let Some(dir) = project_dir {
        let perceptions = collect_git_perceptions(&dir);
        if !perceptions.is_empty() {
            let locked = state.lock().await;
            for p in &perceptions {
                if let Err(e) = manas::store_perception(&locked.conn, p) {
                    eprintln!("[perception] store error: {}", e);
                }
            }
            eprintln!("[perception] stored {} git perceptions", perceptions.len());
        }
    }
}

/// Run git commands and build Perception entries from the results.
/// Uses std::process::Command (blocking but fast).
fn collect_git_perceptions(project_dir: &str) -> Vec<forge_core::types::Perception> {
    use forge_core::types::{Perception, PerceptionKind, Severity};

    let mut perceptions = Vec::new();
    let expires_at = Some(manas::now_offset(PERCEPTION_TTL_SECS));

    // 1. git status --porcelain
    if let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_dir)
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let dirty_files: Vec<&str> = stdout.lines().collect();
            if !dirty_files.is_empty() {
                // Truncate to first 20 lines to avoid huge perceptions
                let summary: String = dirty_files
                    .iter()
                    .take(20)
                    .copied()
                    .collect::<Vec<&str>>()
                    .join("\n");
                let data = format!(
                    "{{\"dirty_count\":{},\"files\":\"{}\"}}",
                    dirty_files.len(),
                    summary.replace('\\', "\\\\").replace('"', "\\\"")
                );
                perceptions.push(Perception {
                    id: ulid::Ulid::new().to_string(),
                    kind: PerceptionKind::FileChange,
                    data,
                    severity: Severity::Info,
                    project: Some(project_dir.to_string()),
                    created_at: manas::now_offset(0),
                    expires_at: expires_at.clone(),
                    consumed: false,
                });
            }
        }
    }

    // 2. git log --oneline -1
    if let Ok(output) = std::process::Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(project_dir)
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stdout.is_empty() {
                let data = format!(
                    "{{\"latest_commit\":\"{}\"}}",
                    stdout.replace('\\', "\\\\").replace('"', "\\\"")
                );
                perceptions.push(Perception {
                    id: ulid::Ulid::new().to_string(),
                    kind: PerceptionKind::BuildResult,
                    data,
                    severity: Severity::Debug,
                    project: Some(project_dir.to_string()),
                    created_at: manas::now_offset(0),
                    expires_at,
                    consumed: false,
                });
            }
        }
    }

    perceptions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{manas as db_manas, schema};
    use forge_core::types::{Perception, PerceptionKind, Severity};

    fn open_db() -> rusqlite::Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_expire_perceptions() {
        let conn = open_db();

        // Store a perception with expires_at in the past
        let p = Perception {
            id: "p-expired".into(),
            kind: PerceptionKind::FileChange,
            data: "old data".into(),
            severity: Severity::Info,
            project: None,
            created_at: "2020-01-01 00:00:00".into(),
            expires_at: Some("2020-01-01 00:05:00".into()),
            consumed: false,
        };
        db_manas::store_perception(&conn, &p).unwrap();

        // Store a perception with expires_at in the future
        let p2 = Perception {
            id: "p-fresh".into(),
            kind: PerceptionKind::FileChange,
            data: "new data".into(),
            severity: Severity::Info,
            project: None,
            created_at: manas::now_offset(0),
            expires_at: Some(manas::now_offset(300)),
            consumed: false,
        };
        db_manas::store_perception(&conn, &p2).unwrap();

        // Expire old perceptions
        let expired = db_manas::expire_perceptions(&conn).unwrap();
        assert_eq!(expired, 1, "should expire the past perception");

        // Verify only the fresh one remains
        let remaining = db_manas::list_unconsumed_perceptions(&conn, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "p-fresh");
    }

    #[test]
    fn test_perception_git_status_no_project() {
        // When no project dir, collect_git_perceptions should return empty
        let perceptions = collect_git_perceptions("/nonexistent/path/that/does/not/exist");
        assert!(
            perceptions.is_empty(),
            "should return empty for nonexistent dir"
        );
    }
}
