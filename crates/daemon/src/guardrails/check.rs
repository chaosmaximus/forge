use rusqlite::{Connection, params};

/// Result of a guardrail check for a file action.
#[derive(Debug, Clone)]
pub struct GuardrailResult {
    pub safe: bool,
    pub warnings: Vec<String>,
    pub decisions_affected: Vec<String>,
    pub callers_count: usize,
}

/// Check whether an action on a file is safe by looking up linked decisions
/// and counting callers of symbols in that file.
pub fn check_action(conn: &Connection, file: &str, action: &str) -> GuardrailResult {
    let decisions = find_decisions_for_file(conn, file);
    let callers_count = count_callers(conn, file);

    let warnings: Vec<String> = decisions
        .iter()
        .map(|(id, title, confidence)| {
            format!(
                "[{}] Decision \"{}\" (confidence: {:.2}) linked to {} — id: {}",
                action, title, confidence, file, id
            )
        })
        .collect();

    let decisions_affected: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();

    let safe = decisions_affected.is_empty();

    GuardrailResult {
        safe,
        warnings,
        decisions_affected,
        callers_count,
    }
}

/// Find active decisions linked to a file via "affects" edges.
/// Returns (id, title, confidence) tuples ordered by confidence descending.
fn find_decisions_for_file(conn: &Connection, file: &str) -> Vec<(String, String, f64)> {
    let file_target = format!("file:{}", file);
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.title, m.confidence FROM memory m
             JOIN edge e ON e.from_id = m.id
             WHERE e.to_id = ?1 AND e.edge_type = 'affects' AND m.status = 'active'
             ORDER BY m.confidence DESC",
        )
        .expect("failed to prepare find_decisions query");

    stmt.query_map(params![file_target], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })
    .expect("failed to execute find_decisions query")
    .filter_map(|r| r.ok())
    .collect()
}

/// Count distinct callers of symbols defined in the given file.
fn count_callers(conn: &Connection, file: &str) -> usize {
    conn.query_row(
        "SELECT COUNT(DISTINCT e.from_id) FROM code_symbol cs
         JOIN edge e ON e.to_id = cs.id
         WHERE cs.file_path = ?1 AND e.edge_type = 'calls'",
        params![file],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ops::{remember, store_edge, forget};
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

        // Memory exists but no edge linking it to the file
        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert!(result.decisions_affected.is_empty());
    }

    #[test]
    fn test_guardrail_with_decisions() {
        let conn = setup();

        // Two decisions linked to the same file
        let mem1 = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem1).unwrap();
        store_edge(&conn, &mem1.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let mem2 = Memory::new(MemoryType::Decision, "Rate limit endpoints", "Apply rate limiting");
        remember(&conn, &mem2).unwrap();
        store_edge(&conn, &mem2.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "delete");
        assert!(!result.safe);
        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.decisions_affected.len(), 2);

        // Verify warning format
        assert!(result.warnings[0].contains("[delete]"));
        assert!(result.warnings[0].contains("src/auth.rs"));
    }

    #[test]
    fn test_guardrail_superseded_decision_excluded() {
        let conn = setup();

        // Decision linked then forgotten (superseded)
        let mem = Memory::new(MemoryType::Decision, "Old auth approach", "Deprecated approach");
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();
        forget(&conn, &mem.id).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert!(result.decisions_affected.is_empty());
    }

    #[test]
    fn test_guardrail_different_files_independent() {
        let conn = setup();

        // Decision linked to file A, check file B
        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "We chose JWT tokens");
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/server.rs", "modify");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert!(result.decisions_affected.is_empty());
    }
}
