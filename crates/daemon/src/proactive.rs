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
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='context_effectiveness'")?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)?;

    if !table_exists {
        return Ok(None);
    }

    // Query for acknowledged and total injections to compute effectiveness rate.
    // With project scoping: try project-specific first, fall back to global.
    // context_effectiveness table uses column name `context_type` (not knowledge_type)
    // Project scoping: try project-specific first via session join, fall back to global
    let (ack_count, total_count) = if let Some(proj) = project {
        let result: (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN ce.acknowledged = 1 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM context_effectiveness ce
             JOIN session s ON ce.session_id = s.id
             WHERE ce.hook_event = ?1 AND ce.context_type = ?2 AND s.project = ?3",
            rusqlite::params![hook_event, knowledge_type, proj],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if result.1 >= 5 {
            result
        } else {
            conn.query_row(
                "SELECT COALESCE(SUM(CASE WHEN acknowledged = 1 THEN 1 ELSE 0 END), 0),
                        COUNT(*)
                 FROM context_effectiveness
                 WHERE hook_event = ?1 AND context_type = ?2",
                rusqlite::params![hook_event, knowledge_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        }
    } else {
        conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN acknowledged = 1 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM context_effectiveness
             WHERE hook_event = ?1 AND context_type = ?2",
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

/// Build proactive context injections for a given hook event.
/// Queries the database for each knowledge type that passes the relevance threshold,
/// fetching the most relevant content for each type.
pub fn build_proactive_context(
    conn: &Connection,
    hook_event: &str,
    project: Option<&str>,
) -> Vec<forge_core::protocol::response::ProactiveInjection> {
    build_proactive_context_with_org(conn, hook_event, project, None)
}

/// Build proactive context with optional organization_id filtering (multi-tenant isolation).
///
/// Phase 2A-4d.3.1 #3: knowledge types are filtered by `context_injection`
/// feature toggles loaded inside the fn:
///
/// * `anti_patterns = false` → skip KT_ANTI_PATTERN (the loudest channel).
/// * `skills = false` → skip KT_SKILL.
/// * `blast_radius = false` → skip KT_BLAST_RADIUS (already a no-op
///   content-wise, but suppress relevance scoring for symmetry).
/// * `active_state = false` → skip KT_NOTIFICATION (active state).
/// * `session_context = false` → skip KT_DECISION + KT_UAT_LESSON.
///
/// Other knowledge types (KT_TEST_REMINDER) are unaffected.
///
/// When a request already has a loaded `ContextInjectionConfig` in
/// scope, prefer [`build_proactive_context_with_inj`] to avoid the
/// extra config-load round-trip.
pub fn build_proactive_context_with_org(
    conn: &Connection,
    hook_event: &str,
    project: Option<&str>,
    org_id: Option<&str>,
) -> Vec<forge_core::protocol::response::ProactiveInjection> {
    let inj = crate::config::load_config().context_injection;
    build_proactive_context_with_inj(conn, hook_event, project, org_id, &inj)
}

/// Phase 2A-4d.3.1 #3 H6: variant of [`build_proactive_context_with_org`]
/// that accepts a pre-loaded [`crate::config::ContextInjectionConfig`].
/// Saves one disk read per hook call when the caller already shares a
/// config across this and other context-compile fns in the same arm.
pub fn build_proactive_context_with_inj(
    conn: &Connection,
    hook_event: &str,
    project: Option<&str>,
    org_id: Option<&str>,
    inj: &crate::config::ContextInjectionConfig,
) -> Vec<forge_core::protocol::response::ProactiveInjection> {
    use forge_core::protocol::response::ProactiveInjection;

    let knowledge_types = [
        (KT_BLAST_RADIUS, "blast_radius context"),
        (KT_ANTI_PATTERN, "anti_pattern"),
        (KT_UAT_LESSON, "uat_lesson"),
        (KT_DECISION, "decision"),
        (KT_TEST_REMINDER, "test_reminder"),
        (KT_SKILL, "skill"),
        (KT_NOTIFICATION, "notification"),
    ];

    let mut injections = Vec::new();

    for (kt, memory_type) in &knowledge_types {
        // Feature-toggle gating — silently skip suppressed knowledge types.
        let suppressed = match *kt {
            KT_ANTI_PATTERN => !inj.anti_patterns,
            KT_SKILL => !inj.skills,
            KT_BLAST_RADIUS => !inj.blast_radius,
            KT_NOTIFICATION => !inj.active_state,
            KT_DECISION | KT_UAT_LESSON => !inj.session_context,
            _ => false,
        };
        if suppressed {
            continue;
        }

        let relevance = context_relevance(conn, hook_event, kt, project);
        if relevance < RELEVANCE_THRESHOLD {
            continue;
        }

        // Fetch the most relevant content for this knowledge type
        let content = match *kt {
            KT_ANTI_PATTERN => fetch_recent_by_tag(conn, "anti-pattern", project, org_id, 1),
            KT_UAT_LESSON => fetch_recent_by_type(conn, "lesson", project, org_id, 1),
            KT_DECISION => fetch_recent_by_type(conn, "decision", project, org_id, 1),
            KT_TEST_REMINDER => fetch_recent_by_tag(conn, "test", project, org_id, 1),
            KT_SKILL => fetch_recent_by_type(conn, "pattern", project, org_id, 1),
            KT_NOTIFICATION => fetch_pending_notifications(conn, project),
            KT_BLAST_RADIUS => continue, // blast_radius is computed per-file, not from memory
            _ => continue,
        };

        if !content.is_empty() {
            injections.push(ProactiveInjection {
                knowledge_type: memory_type.to_string(),
                relevance,
                content,
            });
        }
    }

    injections
}

/// Fetch recent memories by type, optionally scoped to project and organization.
fn fetch_recent_by_type(
    conn: &Connection,
    memory_type: &str,
    project: Option<&str>,
    org_id: Option<&str>,
    limit: usize,
) -> String {
    let result: Vec<String> = match (project, org_id) {
        (Some(proj), Some(org)) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE memory_type = ?1 AND status = 'active' AND project = ?2 AND organization_id = ?3 ORDER BY accessed_at DESC LIMIT ?4"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![memory_type, proj, org, limit], |r| {
                r.get(0)
            })
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
        }
        (Some(proj), None) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE memory_type = ?1 AND status = 'active' AND project = ?2 ORDER BY accessed_at DESC LIMIT ?3"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![memory_type, proj, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
        (None, Some(org)) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE memory_type = ?1 AND status = 'active' AND organization_id = ?2 ORDER BY accessed_at DESC LIMIT ?3"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![memory_type, org, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
        (None, None) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE memory_type = ?1 AND status = 'active' ORDER BY accessed_at DESC LIMIT ?2"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![memory_type, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
    };
    result.join("; ")
}

/// Fetch recent memories by tag, optionally scoped to project and organization.
fn fetch_recent_by_tag(
    conn: &Connection,
    tag: &str,
    project: Option<&str>,
    org_id: Option<&str>,
    limit: usize,
) -> String {
    let pattern = format!("%{tag}%");
    let result: Vec<String> = match (project, org_id) {
        (Some(proj), Some(org)) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE tags LIKE ?1 AND status = 'active' AND project = ?2 AND organization_id = ?3 ORDER BY accessed_at DESC LIMIT ?4"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![pattern, proj, org, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
        (Some(proj), None) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE tags LIKE ?1 AND status = 'active' AND project = ?2 ORDER BY accessed_at DESC LIMIT ?3"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![pattern, proj, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
        (None, Some(org)) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE tags LIKE ?1 AND status = 'active' AND organization_id = ?2 ORDER BY accessed_at DESC LIMIT ?3"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![pattern, org, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
        (None, None) => {
            let mut stmt = match conn.prepare(
                "SELECT title FROM memory WHERE tags LIKE ?1 AND status = 'active' ORDER BY accessed_at DESC LIMIT ?2"
            ) { Ok(s) => s, Err(_) => return String::new() };
            stmt.query_map(rusqlite::params![pattern, limit], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        }
    };
    result.join("; ")
}

/// Fetch pending notifications for the project.
fn fetch_pending_notifications(conn: &Connection, _project: Option<&str>) -> String {
    let mut stmt = match conn.prepare(
        "SELECT message FROM notification WHERE status = 'pending' AND dismissed = 0 ORDER BY created_at DESC LIMIT 3"
    ) { Ok(s) => s, Err(_) => return String::new() };
    let result: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    result.join("; ")
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
        // context_effectiveness table is created by create_schema() (Prajna v2.4)
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
        assert!(score <= 0.2, "unknown pair should be <= 0.2, got {score}");
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
    fn test_build_proactive_context_returns_relevant_types() {
        let conn = setup_effectiveness_db();

        // Store a decision memory
        let m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Decision,
            "Use JWT for auth",
            "JWT chosen for stateless authentication",
        );
        crate::db::ops::remember(&conn, &m).unwrap();

        // Build proactive context for PreEdit (should include decisions at 0.7 relevance)
        let ctx = build_proactive_context(&conn, HOOK_PRE_EDIT, None);
        let has_decision = ctx.iter().any(|c| c.knowledge_type == "decision");
        assert!(
            has_decision,
            "PreEdit should surface decisions (relevance 0.7 >= 0.3 threshold)"
        );
    }

    #[test]
    fn test_build_proactive_context_filters_low_relevance() {
        let conn = setup_effectiveness_db();

        // Store a lesson (uat_lesson)
        let m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Lesson,
            "Always verify test output",
            "UAT lesson from production incident",
        );
        crate::db::ops::remember(&conn, &m).unwrap();

        // Build proactive context for PostEdit — uat_lesson has no bootstrap entry (defaults to 0.1)
        let ctx = build_proactive_context(&conn, HOOK_POST_EDIT, None);
        let has_uat = ctx.iter().any(|c| c.knowledge_type == "uat_lesson");
        assert!(
            !has_uat,
            "PostEdit should NOT surface uat_lesson (relevance 0.1 < 0.3 threshold)"
        );
    }

    #[test]
    fn test_build_proactive_context_empty_db() {
        let conn = setup_effectiveness_db();
        let ctx = build_proactive_context(&conn, HOOK_PRE_BASH, None);
        assert!(
            ctx.is_empty(),
            "empty DB should produce no proactive context"
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

        // Insert 10 acknowledged records via the real API -> learned rate = 10/10 = 1.0
        for i in 0..10 {
            let id = crate::db::effectiveness::record_injection(
                &conn,
                "test-session",
                HOOK_POST_EDIT,
                KT_BLAST_RADIUS,
                &format!("blast {i}"),
            )
            .unwrap();
            crate::db::effectiveness::mark_acknowledged(&conn, &id).unwrap();
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

    // ── Phase 2A-4d.3.1 #3 M1: gating tests for context_injection ──
    //
    // Reviewer flagged the original commit's "structurally identical"
    // claim about per-knowledge-type gating as unsound. These tests
    // prove that each `ContextInjectionConfig` flag actually suppresses
    // its associated knowledge type from the injections list. Each test
    // pairs:
    //   (a) default-on baseline — knowledge type DOES appear
    //   (b) gated-off probe   — same fixture, knowledge type GONE
    //
    // NOTE: `blast_radius` is intentionally not covered by a fixture
    // pair here. KT_BLAST_RADIUS injections are computed per-file in the
    // BlastRadius handler arm, not surfaced from stored memory rows
    // (see line ~235 — `KT_BLAST_RADIUS => continue;`). The
    // suppress-check at line ~213 still gates the loop, but with the
    // unconditional `continue` no fixture in this module can produce
    // a baseline injection to compare against. Coverage for the
    // `blast_radius` flag lives in the handler::BlastRadius arm short-
    // circuit (see handler.rs ~2469).

    fn inj_off(field: &str) -> crate::config::ContextInjectionConfig {
        let mut inj = crate::config::ContextInjectionConfig::default();
        match field {
            "session_context" => inj.session_context = false,
            "active_state" => inj.active_state = false,
            "skills" => inj.skills = false,
            "anti_patterns" => inj.anti_patterns = false,
            "blast_radius" => inj.blast_radius = false,
            "preferences" => inj.preferences = false,
            other => panic!("unknown ContextInjectionConfig field: {other}"),
        }
        inj
    }

    #[test]
    fn gating_proactive_session_context_off_skips_kt_decision() {
        let conn = setup_effectiveness_db();

        let m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Decision,
            "JWT for auth",
            "JWT chosen for stateless authentication",
        );
        crate::db::ops::remember(&conn, &m).unwrap();

        // Baseline: default config surfaces decision at PreEdit (relevance 0.7).
        let default_inj = crate::config::ContextInjectionConfig::default();
        let baseline =
            build_proactive_context_with_inj(&conn, HOOK_PRE_EDIT, None, None, &default_inj);
        let baseline_has_decision = baseline.iter().any(|c| c.knowledge_type == "decision");
        assert!(
            baseline_has_decision,
            "baseline (default config) must surface decision at PreEdit"
        );

        // Gated off: session_context=false suppresses KT_DECISION.
        let inj = inj_off("session_context");
        let gated = build_proactive_context_with_inj(&conn, HOOK_PRE_EDIT, None, None, &inj);
        assert!(
            !gated.iter().any(|c| c.knowledge_type == "decision"),
            "session_context=false must suppress KT_DECISION; got: {gated:?}"
        );
    }

    #[test]
    fn gating_proactive_anti_patterns_off_skips_kt_anti_pattern() {
        let conn = setup_effectiveness_db();

        // Anti-pattern is fetched by tag containing "anti-pattern".
        let mut m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Lesson,
            "ap-1",
            "do not do X",
        );
        m.tags = vec!["anti-pattern".to_string()];
        crate::db::ops::remember(&conn, &m).unwrap();

        // Baseline: PreEdit + anti_pattern → 0.8 relevance, surfaces.
        let default_inj = crate::config::ContextInjectionConfig::default();
        let baseline =
            build_proactive_context_with_inj(&conn, HOOK_PRE_EDIT, None, None, &default_inj);
        assert!(
            baseline.iter().any(|c| c.knowledge_type == "anti_pattern"),
            "baseline (default config) must surface anti_pattern at PreEdit"
        );

        let inj = inj_off("anti_patterns");
        let gated = build_proactive_context_with_inj(&conn, HOOK_PRE_EDIT, None, None, &inj);
        assert!(
            !gated.iter().any(|c| c.knowledge_type == "anti_pattern"),
            "anti_patterns=false must suppress KT_ANTI_PATTERN; got: {gated:?}"
        );
    }

    #[test]
    fn gating_proactive_skills_off_skips_kt_skill() {
        let conn = setup_effectiveness_db();

        // KT_SKILL is fetched as memory_type='pattern'.
        let m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Pattern,
            "skill-1",
            "use Edit not sed for in-place edits",
        );
        crate::db::ops::remember(&conn, &m).unwrap();

        // Baseline: PostEdit + skill → 0.6 relevance, surfaces.
        let default_inj = crate::config::ContextInjectionConfig::default();
        let baseline =
            build_proactive_context_with_inj(&conn, HOOK_POST_EDIT, None, None, &default_inj);
        assert!(
            baseline.iter().any(|c| c.knowledge_type == "skill"),
            "baseline (default config) must surface skill at PostEdit"
        );

        let inj = inj_off("skills");
        let gated = build_proactive_context_with_inj(&conn, HOOK_POST_EDIT, None, None, &inj);
        assert!(
            !gated.iter().any(|c| c.knowledge_type == "skill"),
            "skills=false must suppress KT_SKILL; got: {gated:?}"
        );
    }

    #[test]
    fn gating_proactive_session_context_off_skips_kt_uat_lesson() {
        let conn = setup_effectiveness_db();

        let m = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Lesson,
            "lesson-1",
            "Always verify test output",
        );
        crate::db::ops::remember(&conn, &m).unwrap();

        // Baseline: PreBash + uat_lesson → 0.7 relevance, surfaces.
        let default_inj = crate::config::ContextInjectionConfig::default();
        let baseline =
            build_proactive_context_with_inj(&conn, HOOK_PRE_BASH, None, None, &default_inj);
        assert!(
            baseline.iter().any(|c| c.knowledge_type == "uat_lesson"),
            "baseline (default config) must surface uat_lesson at PreBash"
        );

        let inj = inj_off("session_context");
        let gated = build_proactive_context_with_inj(&conn, HOOK_PRE_BASH, None, None, &inj);
        assert!(
            !gated.iter().any(|c| c.knowledge_type == "uat_lesson"),
            "session_context=false must suppress KT_UAT_LESSON; got: {gated:?}"
        );
    }
}
