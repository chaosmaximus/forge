use rusqlite::{params, Connection};

/// Result of a guardrail check for a file action.
#[derive(Debug, Clone)]
pub struct GuardrailResult {
    pub safe: bool,
    pub warnings: Vec<String>,
    pub decisions_affected: Vec<String>,
    /// Number of callers of symbols in this file. Currently always 0 —
    /// will be populated when LSP-based indexing creates "calls" edges (Phase 4).
    pub callers_count: usize,
}

/// Check whether an action on a file is safe by looking up linked decisions.
///
/// Queries the edge table for active decisions linked to the target file via
/// "affects" edges. The `action` parameter is included in warning messages
/// for context but does not affect the safety determination.
pub fn check_action(conn: &Connection, file: &str, action: &str) -> GuardrailResult {
    let file_target = format!("file:{}", file);
    let decisions = find_decisions_for_file(conn, &file_target);

    let warnings: Vec<String> = decisions
        .iter()
        .map(|(id, title, confidence)| {
            format!(
                "[{action}] Decision \"{title}\" (confidence: {confidence:.2}) linked to {file} — id: {id}"
            )
        })
        .collect();

    let decisions_affected: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();
    let safe = decisions_affected.is_empty();

    GuardrailResult {
        safe,
        warnings,
        decisions_affected,
        // No "calls" edges exist yet — LSP indexing (Phase 4) will populate these.
        callers_count: 0,
    }
}

/// Find active decisions linked to a file target via "affects" edges.
/// Returns (id, title, confidence) tuples ordered by confidence descending.
fn find_decisions_for_file(conn: &Connection, file_target: &str) -> Vec<(String, String, f64)> {
    let mut stmt = match conn.prepare(
        "SELECT m.id, m.title, m.confidence FROM memory m
         JOIN edge e ON e.from_id = m.id
         WHERE e.to_id = ?1 AND e.edge_type = 'affects'
         AND m.memory_type = 'decision' AND m.status = 'active'
         ORDER BY m.confidence DESC
         LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let rows = match stmt.query_map(params![file_target], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    }) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rows.filter_map(|r| r.ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ops::{forget, remember, store_edge};
    use crate::db::schema::create_schema;
    use forge_core::types::{Memory, MemoryType};

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_guardrail_no_decisions() {
        let conn = setup();

        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert!(result.decisions_affected.is_empty());
        assert_eq!(result.callers_count, 0);
    }

    #[test]
    fn test_guardrail_with_decisions() {
        let conn = setup();

        let mem1 = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem1).unwrap();
        store_edge(&conn, &mem1.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let mem2 = Memory::new(
            MemoryType::Decision,
            "Rate limit endpoints",
            "Apply rate limiting",
        );
        remember(&conn, &mem2).unwrap();
        store_edge(&conn, &mem2.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "delete");
        assert!(!result.safe);
        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.decisions_affected.len(), 2);
        assert!(result.warnings[0].contains("[delete]"));
        assert!(result.warnings[0].contains("src/auth.rs"));
    }

    #[test]
    fn test_guardrail_superseded_decision_excluded() {
        let conn = setup();

        let mem = Memory::new(MemoryType::Decision, "Old auth approach", "Deprecated approach");
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();
        forget(&conn, &mem.id).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
    }

    #[test]
    fn test_guardrail_different_files_independent() {
        let conn = setup();

        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/server.rs", "modify");
        assert!(result.safe);
    }

    #[test]
    fn test_guardrail_only_decisions_not_lessons() {
        let conn = setup();

        // A lesson linked to a file should NOT trigger guardrails
        let lesson = Memory::new(MemoryType::Lesson, "Learned about auth", "Auth is tricky");
        remember(&conn, &lesson).unwrap();
        store_edge(&conn, &lesson.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert!(result.safe, "lessons should not trigger guardrails, only decisions");
    }
}
