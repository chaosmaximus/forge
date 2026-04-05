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
    /// Cluster this file belongs to (from community detection), if any.
    pub cluster_name: Option<String>,
    /// Other files in the same cluster.
    pub cluster_files: Vec<String>,
    /// Files that call symbols in this file (from edge table).
    pub calling_files: Vec<String>,
}

/// Main entry point: analyse the blast radius of changing `file`.
pub fn analyze_blast_radius(conn: &Connection, file: &str) -> BlastRadius {
    let file_target = format!("file:{file}");

    let decisions = find_decisions(conn, &file_target);
    let importers = find_importers(conn, &file_target);
    let (callers, calling_files) = find_callers(conn, file);
    let (cluster_name, cluster_files) = find_cluster(conn, file);

    let decision_ids: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();
    let files_affected = find_co_affected_files(conn, &decision_ids, &file_target);

    BlastRadius {
        decisions,
        callers,
        importers,
        files_affected,
        cluster_name,
        cluster_files,
        calling_files,
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

/// Find files that call/import symbols in the given file.
/// Searches both 'calls' and 'imports' edge types.
/// For imports: converts file path to module path pattern (e.g., src/server/handler.rs → server::handler).
/// Returns (caller_count, calling_file_paths).
fn find_callers(conn: &Connection, file: &str) -> (usize, Vec<String>) {
    // Escape LIKE wildcards in file path to prevent pattern injection
    let escaped = file.replace('%', "\\%").replace('_', "\\_");
    let file_pattern = format!("%{escaped}%");

    // Convert file path to Rust module pattern for import edge matching
    // e.g., "crates/daemon/src/server/handler.rs" → "%server::handler%"
    let module_pattern = {
        let stem = file
            .trim_end_matches(".rs")
            .trim_end_matches(".py")
            .trim_end_matches(".ts")
            .trim_end_matches(".tsx")
            .trim_end_matches(".js")
            .trim_end_matches(".go");
        // Take last 2 path segments as module path
        let parts: Vec<&str> = stem.split('/').collect();
        let module = if parts.len() >= 2 {
            format!("%{}::{}%", parts[parts.len()-2], parts[parts.len()-1])
        } else if !parts.is_empty() {
            format!("%{}%", parts[parts.len()-1])
        } else {
            file_pattern.clone()
        };
        module.replace('%', "\\%").replace('_', "\\_");
        format!("%{}%", parts.last().map(|s| s.replace('%', "\\%").replace('_', "\\_")).unwrap_or_default())
    };

    let sql = "
        SELECT DISTINCT from_id
        FROM edge
        WHERE (edge_type = 'calls' AND to_id LIKE ?1)
           OR (edge_type = 'imports' AND to_id LIKE ?2)
        LIMIT 100
    ";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return (0, Vec::new()),
    };
    let files: Vec<String> = match stmt.query_map(rusqlite::params![file_pattern, module_pattern], |row| {
        row.get::<_, String>(0)
    }) {
        Ok(rows) => rows
            .filter_map(|r| r.ok())
            .map(|s| s.strip_prefix("file:").unwrap_or(&s).to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    let count = files.len();
    (count, files)
}

/// Find the cluster this file belongs to, and all other files in that cluster.
/// Returns (cluster_name, cluster_files) — cluster_files excludes the target file.
fn find_cluster(conn: &Connection, file: &str) -> (Option<String>, Vec<String>) {
    // Step 1: Find which cluster this file belongs to
    let cluster_sql = "
        SELECT to_id
        FROM edge
        WHERE edge_type = 'belongs_to_cluster'
          AND from_id = ?1
        LIMIT 1
    ";
    let file_id = format!("file:{file}");
    let cluster_id: Option<String> = conn
        .prepare(cluster_sql)
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![file_id], |row| row.get::<_, String>(0))
                .ok()
                .and_then(|mut rows| rows.next().and_then(|r| r.ok()))
        });

    let cluster_id = match cluster_id {
        Some(id) => id,
        None => return (None, Vec::new()),
    };

    // Step 2: Find all other files in the same cluster
    let members_sql = "
        SELECT from_id
        FROM edge
        WHERE edge_type = 'belongs_to_cluster'
          AND to_id = ?1
    ";
    let mut stmt = match conn.prepare(members_sql) {
        Ok(s) => s,
        Err(_) => return (Some(cluster_id.clone()), Vec::new()),
    };
    let files: Vec<String> = match stmt.query_map(rusqlite::params![cluster_id], |row| {
        row.get::<_, String>(0)
    }) {
        Ok(rows) => rows
            .filter_map(|r| r.ok())
            .filter(|s| s != &file_id)
            .map(|s| s.strip_prefix("file:").unwrap_or(&s).to_string())
            .collect(),
        Err(_) => Vec::new(),
    };

    (Some(cluster_id), files)
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

    #[test]
    fn test_blast_radius_real_callers() {
        let conn = setup_db();

        // Create call edges: main.rs and routes.rs call symbols in auth.rs
        store_edge(&conn, "file:src/main.rs", "sym:src/auth.rs::authenticate", "calls", "{}").unwrap();
        store_edge(&conn, "file:src/routes.rs", "sym:src/auth.rs::verify_token", "calls", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert_eq!(br.callers, 2, "expected 2 callers, got {}", br.callers);
        assert!(br.calling_files.contains(&"src/main.rs".to_string()));
        assert!(br.calling_files.contains(&"src/routes.rs".to_string()));
    }

    #[test]
    fn test_blast_radius_cluster_info() {
        let conn = setup_db();

        // Create cluster edges
        store_edge(&conn, "file:src/auth.rs", "cluster:auth-cluster", "belongs_to_cluster", "{}").unwrap();
        store_edge(&conn, "file:src/session.rs", "cluster:auth-cluster", "belongs_to_cluster", "{}").unwrap();
        store_edge(&conn, "file:src/tokens.rs", "cluster:auth-cluster", "belongs_to_cluster", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/auth.rs");
        assert_eq!(br.cluster_name.as_deref(), Some("cluster:auth-cluster"));
        assert_eq!(br.cluster_files.len(), 2, "expected 2 cluster files (excluding self), got {:?}", br.cluster_files);
        assert!(br.cluster_files.contains(&"src/session.rs".to_string()));
        assert!(br.cluster_files.contains(&"src/tokens.rs".to_string()));
        // The target file itself should not appear in cluster_files
        assert!(!br.cluster_files.contains(&"src/auth.rs".to_string()));
    }

    #[test]
    fn test_blast_radius_empty_backward_compat() {
        let conn = setup_db();

        let br = analyze_blast_radius(&conn, "src/nonexistent.rs");
        assert_eq!(br.callers, 0);
        assert!(br.calling_files.is_empty());
        assert!(br.cluster_name.is_none());
        assert!(br.cluster_files.is_empty());
        assert!(br.decisions.is_empty());
        assert!(br.importers.is_empty());
        assert!(br.files_affected.is_empty());
    }

    #[test]
    fn test_blast_radius_cross_file_callers() {
        let conn = setup_db();

        // Multiple callers from different files targeting different symbols in the same file
        store_edge(&conn, "file:src/api.rs", "sym:src/db.rs::query", "calls", "{}").unwrap();
        store_edge(&conn, "file:src/service.rs", "sym:src/db.rs::insert", "calls", "{}").unwrap();
        store_edge(&conn, "file:src/worker.rs", "sym:src/db.rs::delete", "calls", "{}").unwrap();
        // A call within the same file (should still be counted)
        store_edge(&conn, "file:src/db.rs", "sym:src/db.rs::connect", "calls", "{}").unwrap();

        let br = analyze_blast_radius(&conn, "src/db.rs");
        assert_eq!(br.callers, 4, "expected 4 distinct callers, got {}", br.callers);
        assert!(br.calling_files.contains(&"src/api.rs".to_string()));
        assert!(br.calling_files.contains(&"src/service.rs".to_string()));
        assert!(br.calling_files.contains(&"src/worker.rs".to_string()));
        assert!(br.calling_files.contains(&"src/db.rs".to_string()));
    }
}
