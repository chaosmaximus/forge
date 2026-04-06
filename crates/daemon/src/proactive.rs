//! Prajna Context Router — decides WHAT knowledge to surface at WHAT moment.
//!
//! Starts with a hardcoded bootstrap matrix mapping (hook_event, knowledge_type) -> relevance.
//! When effectiveness data is available (from the effectiveness tracking table), learned
//! rates override the bootstrap values, allowing the system to evolve.

use rusqlite::Connection;

// ── Hook Events ──

pub const HOOK_PRE_EDIT: &str = "PreEdit";
pub const HOOK_POST_EDIT: &str = "PostEdit";
pub const HOOK_PRE_BASH: &str = "PreBash";
pub const HOOK_POST_BASH: &str = "PostBash";
pub const HOOK_USER_PROMPT: &str = "UserPromptSubmit";
pub const HOOK_STOP: &str = "Stop";
pub const HOOK_SUBAGENT_START: &str = "SubagentStart";
pub const HOOK_TASK_COMPLETED: &str = "TaskCompleted";
pub const HOOK_POST_COMPACT: &str = "PostCompact";

// ── Knowledge Types ──

pub const KT_BLAST_RADIUS: &str = "blast_radius";
pub const KT_ANTI_PATTERN: &str = "anti_pattern";
pub const KT_UAT_LESSON: &str = "uat_lesson";
pub const KT_DECISION: &str = "decision";
pub const KT_TEST_REMINDER: &str = "test_reminder";
pub const KT_SKILL: &str = "skill";
pub const KT_NOTIFICATION: &str = "notification";

// ── Relevance Threshold ──

pub const RELEVANCE_THRESHOLD: f64 = 0.3;

// ── Bootstrap Matrix ──

/// Returns a bootstrap relevance score (0.0-1.0) for a given (hook_event, knowledge_type) pair.
/// This is the hardcoded prior — used when no learned effectiveness data is available.
fn bootstrap_relevance(hook_event: &str, knowledge_type: &str) -> f64 {
    match (hook_event, knowledge_type) {
        (HOOK_PRE_EDIT, KT_BLAST_RADIUS) => 0.9,
        (HOOK_PRE_EDIT, KT_ANTI_PATTERN) => 0.8,
        (HOOK_PRE_EDIT, KT_DECISION) => 0.7,
        (HOOK_POST_EDIT, KT_TEST_REMINDER) => 0.8,
        (HOOK_POST_EDIT, KT_BLAST_RADIUS) => 0.2, // Too late for prevention
        (HOOK_POST_EDIT, KT_SKILL) => 0.6,
        (HOOK_PRE_BASH, KT_ANTI_PATTERN) => 0.8,
        (HOOK_PRE_BASH, KT_UAT_LESSON) => 0.7,
        (HOOK_USER_PROMPT, KT_NOTIFICATION) => 0.9,
        (HOOK_USER_PROMPT, KT_ANTI_PATTERN) => 0.7,
        (HOOK_STOP, KT_UAT_LESSON) => 0.95,
        (HOOK_STOP, KT_TEST_REMINDER) => 0.9,
        (HOOK_STOP, KT_ANTI_PATTERN) => 0.8,
        (HOOK_SUBAGENT_START, KT_ANTI_PATTERN) => 0.9,
        (HOOK_SUBAGENT_START, KT_DECISION) => 0.8,
        (HOOK_TASK_COMPLETED, KT_UAT_LESSON) => 0.95,
        (HOOK_TASK_COMPLETED, KT_TEST_REMINDER) => 0.9,
        (HOOK_POST_COMPACT, KT_ANTI_PATTERN) => 0.8,
        (HOOK_POST_COMPACT, KT_DECISION) => 0.7,
        _ => 0.1,
    }
}

// ── Learned Effectiveness ──

/// Query the effectiveness table for a learned rate. Returns Ok(Some(rate)) if enough data
/// exists, Ok(None) if the table is missing or has insufficient data, Err on real DB errors.
fn learned_effectiveness_rate(
    conn: &Connection,
    hook_event: &str,
    knowledge_type: &str,
    project: Option<&str>,
) -> rusqlite::Result<Option<f64>> {
    // Check if the effectiveness table exists (it may not be created yet if Task 2 hasn't run)
    let table_exists: bool = conn
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='effectiveness'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)?;

    if !table_exists {
        return Ok(None);
    }

    // Query for acknowledged and total injections to compute effectiveness rate.
    // With project scoping: try project-specific first, fall back to global.
    let (ack_count, total_count) = if let Some(proj) = project {
        let result: (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN acknowledged = 1 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM effectiveness
             WHERE hook_event = ?1 AND knowledge_type = ?2 AND project = ?3",
            rusqlite::params![hook_event, knowledge_type, proj],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if result.1 >= 5 {
            result
        } else {
            // Fall back to global (all projects)
            conn.query_row(
                "SELECT COALESCE(SUM(CASE WHEN acknowledged = 1 THEN 1 ELSE 0 END), 0),
                        COUNT(*)
                 FROM effectiveness
                 WHERE hook_event = ?1 AND knowledge_type = ?2",
                rusqlite::params![hook_event, knowledge_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        }
    } else {
        conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN acknowledged = 1 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM effectiveness
             WHERE hook_event = ?1 AND knowledge_type = ?2",
            rusqlite::params![hook_event, knowledge_type],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };

    // Require a minimum of 5 samples before trusting learned data
    if total_count < 5 {
        return Ok(None);
    }

    Ok(Some(ack_count as f64 / total_count as f64))
}

// ── Public API ──

/// Returns the relevance score for surfacing `knowledge_type` at `hook_event`.
/// Prefers learned effectiveness data when available; falls back to bootstrap matrix.
pub fn context_relevance(
    conn: &Connection,
    hook_event: &str,
    knowledge_type: &str,
    project: Option<&str>,
) -> f64 {
    if let Ok(Some(rate)) = learned_effectiveness_rate(conn, hook_event, knowledge_type, project) {
        return rate;
    }
    bootstrap_relevance(hook_event, knowledge_type)
}

/// Returns true if the knowledge type should be surfaced at the given hook event.
pub fn should_surface(
    conn: &Connection,
    hook_event: &str,
    knowledge_type: &str,
    project: Option<&str>,
) -> bool {
    context_relevance(conn, hook_event, knowledge_type, project) >= RELEVANCE_THRESHOLD
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Helper: create an in-memory DB with the effectiveness table for testing learned overrides.
    fn setup_effectiveness_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // Create the effectiveness table inline (Task 2 may not be merged yet)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS effectiveness (
                id TEXT PRIMARY KEY,
                hook_event TEXT NOT NULL,
                knowledge_type TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                session_id TEXT NOT NULL DEFAULT '',
                project TEXT,
                acknowledged INTEGER NOT NULL DEFAULT 0,
                injected_at TEXT NOT NULL,
                acknowledged_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_eff_hook_kt ON effectiveness(hook_event, knowledge_type);
            CREATE INDEX IF NOT EXISTS idx_eff_project ON effectiveness(project);",
        )
        .unwrap();
        conn
    }

    /// Helper: create a minimal in-memory DB (no effectiveness table).
    fn setup_minimal_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_bootstrap_blast_radius_high_at_pre_edit() {
        let score = bootstrap_relevance(HOOK_PRE_EDIT, KT_BLAST_RADIUS);
        assert!(
            score >= 0.8,
            "blast_radius at PreEdit should be >= 0.8, got {score}"
        );
    }

    #[test]
    fn test_bootstrap_blast_radius_low_at_post_edit() {
        let score = bootstrap_relevance(HOOK_POST_EDIT, KT_BLAST_RADIUS);
        assert!(
            score <= 0.3,
            "blast_radius at PostEdit should be <= 0.3, got {score}"
        );
    }

    #[test]
    fn test_bootstrap_uat_high_at_stop() {
        let score = bootstrap_relevance(HOOK_STOP, KT_UAT_LESSON);
        assert!(
            score >= 0.9,
            "uat_lesson at Stop should be >= 0.9, got {score}"
        );
    }

    #[test]
    fn test_bootstrap_uat_low_at_user_prompt() {
        let score = bootstrap_relevance(HOOK_USER_PROMPT, KT_UAT_LESSON);
        assert!(
            score < 0.3,
            "uat_lesson at UserPromptSubmit should be < 0.3 (not in bootstrap), got {score}"
        );
    }

    #[test]
    fn test_bootstrap_unknown_pair_is_low() {
        let score = bootstrap_relevance("UnknownHook", "unknown_type");
        assert!(
            score <= 0.2,
            "unknown pair should be <= 0.2, got {score}"
        );
    }

    #[test]
    fn test_should_surface_threshold() {
        let conn = setup_minimal_db();

        // blast_radius at pre-edit (0.9) should surface
        assert!(
            should_surface(&conn, HOOK_PRE_EDIT, KT_BLAST_RADIUS, None),
            "blast_radius at PreEdit should surface (relevance 0.9 >= threshold 0.3)"
        );

        // blast_radius at post-edit (0.2) should NOT surface
        assert!(
            !should_surface(&conn, HOOK_POST_EDIT, KT_BLAST_RADIUS, None),
            "blast_radius at PostEdit should NOT surface (relevance 0.2 < threshold 0.3)"
        );
    }

    #[test]
    fn test_learned_overrides_bootstrap() {
        let conn = setup_effectiveness_db();

        // Bootstrap: PostEdit + blast_radius = 0.2 (below threshold)
        assert!(
            !should_surface(&conn, HOOK_POST_EDIT, KT_BLAST_RADIUS, None),
            "before learning, PostEdit+blast_radius should NOT surface"
        );

        // Insert 10 acknowledged records -> learned rate = 10/10 = 1.0
        let now = "2026-04-06T00:00:00Z";
        for i in 0..10 {
            conn.execute(
                "INSERT INTO effectiveness (id, hook_event, knowledge_type, memory_id, session_id, acknowledged, injected_at, acknowledged_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                rusqlite::params![
                    format!("eff-{i}"),
                    HOOK_POST_EDIT,
                    KT_BLAST_RADIUS,
                    format!("mem-{i}"),
                    "test-session",
                    now,
                ],
            )
            .unwrap();
        }

        // Now learned rate = 1.0, well above threshold
        let relevance = context_relevance(&conn, HOOK_POST_EDIT, KT_BLAST_RADIUS, None);
        assert!(
            relevance > RELEVANCE_THRESHOLD,
            "learned rate should override bootstrap: got {relevance}"
        );
        assert!(
            should_surface(&conn, HOOK_POST_EDIT, KT_BLAST_RADIUS, None),
            "after learning (10 acks), PostEdit+blast_radius should surface"
        );
    }
}
