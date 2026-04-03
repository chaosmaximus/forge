use rusqlite::Connection;

/// Full blast radius analysis for a file: which decisions reference it,
/// how many callers it has, who imports it, and what other files share
/// the same decisions.
pub struct BlastRadius {
    /// (id, title, confidence) for each decision linked to this file
    pub decisions: Vec<(String, String, f64)>,
    /// Number of edges where this file is a call target
    pub callers: usize,
    /// Files that import this file (from_id values with edge_type='imports')
    pub importers: Vec<String>,
    /// Other files affected by the same decisions (excluding the target file)
    pub files_affected: Vec<String>,
}

/// Main entry point: analyse the blast radius of changing `file`.
pub fn analyze_blast_radius(conn: &Connection, file: &str) -> BlastRadius {
    let file_target = format!("file:{file}");

    let decisions = find_decisions(conn, &file_target);
    let callers = count_callers(conn, &file_target);
    let importers = find_importers(conn, &file_target);

    let decision_ids: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();
    let files_affected = find_co_affected_files(conn, &decision_ids, &file_target);

    BlastRadius {
        decisions,
        callers,
        importers,
        files_affected,
    }
}

/// Find all decisions that affect the given file target.
/// Returns (id, title, confidence) triples.
fn find_decisions(conn: &Connection, file_target: &str) -> Vec<(String, String, f64)> {
    let sql = "
        SELECT m.id, m.title, m.confidence
        FROM edge e
        JOIN memory m ON e.from_id = m.id
        WHERE e.to_id = ?1
          AND e.edge_type = 'affects'
          AND m.memory_type = 'decision'
          AND m.status = 'active'
        ORDER BY m.confidence DESC
        LIMIT 50
    ";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let result = match stmt.query_map([file_target], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    };
    result
}

/// Count edges where this file is a call target (edge_type = 'calls').
fn count_callers(conn: &Connection, file: &str) -> usize {
    let sql = "SELECT COUNT(*) FROM edge WHERE to_id = ?1 AND edge_type = 'calls'";
    conn.query_row(sql, [file], |row| row.get::<_, i64>(0))
        .unwrap_or(0) as usize
}

/// Find files that import the given file target.
/// Returns from_id values stripped of the "file:" prefix.
fn find_importers(conn: &Connection, file_target: &str) -> Vec<String> {
    let sql = "
        SELECT DISTINCT e.from_id
        FROM edge e
        WHERE e.to_id = ?1
          AND e.edge_type = 'imports'
        LIMIT 50
    ";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let result = match stmt.query_map([file_target], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows
            .filter_map(|r| r.ok())
            .map(|s| s.strip_prefix("file:").unwrap_or(&s).to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    result
}

/// Find other files affected by the same decisions, excluding the target file itself.
/// Returns file paths stripped of the "file:" prefix.
fn find_co_affected_files(
    conn: &Connection,
    decision_ids: &[String],
    exclude_target: &str,
) -> Vec<String> {
    if decision_ids.is_empty() {
        return Vec::new();
    }

    let placeholders: Vec<String> = (1..=decision_ids.len()).map(|i| format!("?{i}")).collect();
    let in_clause = placeholders.join(", ");
    let exclude_idx = decision_ids.len() + 1;
    let sql = format!(
        "SELECT DISTINCT e.to_id FROM edge e
         WHERE e.from_id IN ({in_clause})
         AND e.edge_type = 'affects'
         AND e.to_id != ?{exclude_idx}
         AND e.to_id LIKE 'file:%'
         LIMIT 50"
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = decision_ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    param_values.push(Box::new(exclude_target.to_string()));
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let result = match stmt.query_map(params_ref.as_slice(), |row| row.get::<_, String>(0)) {
        Ok(rows) => rows
            .filter_map(|r| r.ok())
            .map(|s| s.strip_prefix("file:").unwrap_or(&s).to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ops::store_edge, schema::create_schema};
    use forge_core::types::{Memory, MemoryType};

    fn setup_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_blast_radius_empty() {
        let conn = setup_db();
        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert!(br.decisions.is_empty());
        assert_eq!(br.callers, 0);
        assert!(br.importers.is_empty());
        assert!(br.files_affected.is_empty());
    }

    #[test]
    fn test_blast_radius_with_decisions() {
        let conn = setup_db();

        let d1 = Memory::new(MemoryType::Decision, "Use JWT auth", "JWT for all APIs");
        let d2 = Memory::new(MemoryType::Decision, "Rate limit endpoints", "Global rate limiter");
        crate::db::ops::remember(&conn, &d1).unwrap();
        crate::db::ops::remember(&conn, &d2).unwrap();

        store_edge(&conn, &d1.id, "file:src/auth.rs", "affects", "{}").unwrap();
        store_edge(&conn, &d2.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert_eq!(br.decisions.len(), 2);
    }

    #[test]
    fn test_blast_radius_co_affected_files() {
        let conn = setup_db();

        let d1 = Memory::new(MemoryType::Decision, "Auth middleware", "Shared auth layer");
        crate::db::ops::remember(&conn, &d1).unwrap();

        // Decision affects both auth.rs and middleware.rs
        store_edge(&conn, &d1.id, "file:src/auth.rs", "affects", "{}").unwrap();
        store_edge(&conn, &d1.id, "file:src/middleware.rs", "affects", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert_eq!(br.decisions.len(), 1);
        assert!(
            br.files_affected.contains(&"src/middleware.rs".to_string()),
            "files_affected should contain src/middleware.rs, got: {:?}",
            br.files_affected
        );
        // The target file itself should NOT appear in files_affected
        assert!(
            !br.files_affected.contains(&"src/auth.rs".to_string()),
            "files_affected should NOT contain the target file"
        );
    }

    #[test]
    fn test_blast_radius_importers() {
        let conn = setup_db();

        store_edge(&conn, "file:src/main.rs", "file:src/auth.rs", "imports", "{}").unwrap();
        store_edge(&conn, "file:src/routes.rs", "file:src/auth.rs", "imports", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert_eq!(br.importers.len(), 2);
        assert!(br.importers.contains(&"src/main.rs".to_string()));
        assert!(br.importers.contains(&"src/routes.rs".to_string()));
    }
}
