// workers/perception.rs — Environment perception worker
//
// Monitors the project environment (git status, file changes) and creates
// ephemeral Perception entries (Manas Layer 4). Perceptions auto-expire
// after 5 minutes — they represent transient sensory data, not long-term memory.

use crate::db::manas;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.perception_interval_secs
// (default: 30 seconds)

/// Perception expiry: 5 minutes from creation.
const PERCEPTION_TTL_SECS: i64 = 5 * 60;

pub async fn run_perception(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    eprintln!("[perception] started, interval = {interval:?}");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
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
                    eprintln!("[perception] expired {expired} old perceptions");
                }
            }
            Err(e) => eprintln!("[perception] expire error: {e}"),
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
                    eprintln!("[perception] store error: {e}");
                }
            }
            eprintln!("[perception] stored {} git perceptions", perceptions.len());
        }
    }

    // Phase 3: Anti-pattern detection (v8.2)
    // Compare recent agent actions against stored anti-patterns via keyword overlap.
    // Uses FTS5 + keyword matching (not embedding — avoids LLM call on every tick).
    {
        let locked = state.lock().await;
        let config = crate::config::load_config();
        let threshold = config.proactive.anti_pattern_threshold;
        let detections = detect_anti_patterns(&locked.conn, threshold);
        for (ref ap_title, ref action_summary, confidence) in &detections {
            let data = format!(
                "{{\"type\":\"anti_pattern_detected\",\"anti_pattern\":\"{}\",\"matched_action\":\"{}\",\"confidence\":{}}}",
                ap_title.replace('"', "\\\""),
                action_summary.replace('"', "\\\""),
                confidence,
            );
            let perception = forge_core::types::Perception {
                id: ulid::Ulid::new().to_string(),
                kind: forge_core::types::PerceptionKind::Error, // closest to "warning" — anti-pattern detection
                data,
                severity: forge_core::types::Severity::Warning,
                project: None,
                created_at: manas::now_offset(0),
                expires_at: Some(manas::now_offset(PERCEPTION_TTL_SECS)),
                consumed: false,
            };
            if let Err(e) = manas::store_perception(&locked.conn, &perception) {
                eprintln!("[perception] anti-pattern store error: {e}");
            }
            // Emit event for real-time UI notification
            crate::events::emit(&locked.events, "anti_pattern_detected", serde_json::json!({
                "anti_pattern": ap_title,
                "action": action_summary,
                "confidence": confidence,
            }));
            eprintln!("[perception] anti-pattern detected: {ap_title} (confidence: {confidence:.2})");
        }
    } // lock released
}

/// Detect behavioral anti-patterns by comparing recent agent actions against stored anti-patterns.
/// Uses keyword overlap similarity (Jaccard-like). Returns (title, action_summary, confidence).
///
/// v8.2 spec: learns anti-pattern classes from annotated mistakes, matches current behavior,
/// raises perceptions when confidence exceeds threshold.
fn detect_anti_patterns(conn: &rusqlite::Connection, threshold: f64) -> Vec<(String, String, f64)> {
    use std::collections::HashSet;

    // 1. Fetch stored anti-patterns (tagged 'anti-pattern', active)
    let anti_patterns: Vec<(String, String)> = (|| -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(
            "SELECT title, content FROM memory
             WHERE tags LIKE '%anti-pattern%' AND status = 'active'
             ORDER BY quality_score DESC LIMIT 10"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })().unwrap_or_default();

    if anti_patterns.is_empty() {
        return vec![];
    }

    // 2. Fetch recent audit log entries (last 5 minutes)
    // audit_log schema: id, actor_type, actor_id, action, resource_type, resource_id, scope_path, details, timestamp
    let recent_actions: Vec<(String, String)> = (|| -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(
            "SELECT action, COALESCE(resource_id, '') || ' ' || COALESCE(details, '') FROM audit_log
             WHERE timestamp > datetime('now', '-5 minutes')
             AND action NOT IN ('health', 'manas_health', 'doctor', 'sessions', 'context_stats')
             ORDER BY timestamp DESC LIMIT 20"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })().unwrap_or_default();

    if recent_actions.is_empty() {
        return vec![];
    }

    // 3. Keyword overlap similarity (Jaccard index on significant words)
    let mut detections = Vec::new();
    let stop_words: HashSet<String> = crate::common::STOP_WORDS.iter().map(|s| s.to_string()).collect();

    for (ap_title, ap_content) in &anti_patterns {
        let ap_text = format!("{ap_title} {ap_content}").to_lowercase();
        let ap_words: HashSet<String> = ap_text.split_whitespace()
            .filter(|w| w.len() > 3 && !stop_words.contains(*w))
            .map(|s| s.to_string())
            .collect();
        if ap_words.len() < 3 { continue; }

        for (action_type, action_summary) in &recent_actions {
            let action_text = format!("{action_type} {action_summary}").to_lowercase();
            let action_words: HashSet<String> = action_text.split_whitespace()
                .filter(|w| w.len() > 3 && !stop_words.contains(*w))
                .map(|s| s.to_string())
                .collect();
            if action_words.len() < 2 { continue; }

            let intersection = ap_words.intersection(&action_words).count();
            let union = ap_words.union(&action_words).count();
            let similarity = if union > 0 { intersection as f64 / union as f64 } else { 0.0 };

            if similarity >= threshold {
                detections.push((
                    ap_title.clone(),
                    format!("{action_type}: {action_summary}"),
                    similarity,
                ));
            }
        }
    }

    // Deduplicate: only keep the highest-confidence detection per anti-pattern
    detections.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen_titles = HashSet::new();
    detections.retain(|(title, _, _)| seen_titles.insert(title.clone()));

    detections
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
        let remaining = db_manas::list_unconsumed_perceptions(&conn, None, None).unwrap();
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

    // ── Anti-pattern detection tests (v8.2) ──

    #[test]
    fn test_detect_anti_patterns_no_patterns() {
        let conn = open_db();
        let results = detect_anti_patterns(&conn, 0.85);
        assert!(results.is_empty(), "no anti-patterns stored → no detections");
    }

    #[test]
    fn test_detect_anti_patterns_no_recent_actions() {
        let conn = open_db();
        // Store an anti-pattern but no audit log entries
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, tags, status, confidence, created_at, accessed_at)
             VALUES ('ap1', 'lesson', 'Never test browser when asked for native',
                     'Playwright tests browser fallback not Tauri native app desktop testing webview',
                     'anti-pattern', 'active', 0.9, datetime('now'), datetime('now'))",
            [],
        ).unwrap();
        let results = detect_anti_patterns(&conn, 0.85);
        assert!(results.is_empty(), "no recent actions → no detections");
    }

    #[test]
    fn test_detect_anti_patterns_matching_action() {
        let conn = open_db();
        // Store anti-pattern about browser testing
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, tags, status, confidence, created_at, accessed_at, quality_score)
             VALUES ('ap1', 'lesson', 'Browser testing fallback',
                     'playwright browser testing localhost fallback desktop native webview tauri',
                     'anti-pattern', 'active', 0.9, datetime('now'), datetime('now'), 1.0)",
            [],
        ).unwrap();

        // Store matching audit log entry with heavily overlapping words
        conn.execute(
            "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, details, timestamp)
             VALUES ('a1', 'agent', 'test', 'post_edit', 'file', 'test.ts',
                     'playwright browser testing localhost fallback desktop native webview tauri', datetime('now'))",
            [],
        ).unwrap();

        // Low threshold so keyword overlap matches
        let results = detect_anti_patterns(&conn, 0.3);
        assert!(!results.is_empty(), "should detect matching anti-pattern, got empty");
        assert!(results[0].0.contains("Browser"), "should be the browser testing anti-pattern");
        assert!(results[0].2 >= 0.3, "confidence should be above threshold");
    }

    #[test]
    fn test_detect_anti_patterns_no_false_positive() {
        let conn = open_db();
        // Store anti-pattern about browser testing
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, tags, status, confidence, created_at, accessed_at, quality_score)
             VALUES ('ap1', 'lesson', 'Browser testing fallback',
                     'playwright browser testing localhost fallback desktop native webview tauri',
                     'anti-pattern', 'active', 0.9, datetime('now'), datetime('now'), 1.0)",
            [],
        ).unwrap();

        // Store completely unrelated audit log entry
        conn.execute(
            "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, details, timestamp)
             VALUES ('a1', 'agent', 'test', 'remember', 'memory', 'mem1',
                     'Added a decision about database schema migration strategy', datetime('now'))",
            [],
        ).unwrap();

        let results = detect_anti_patterns(&conn, 0.3);
        assert!(results.is_empty(), "unrelated action should NOT match anti-pattern");
    }
}
