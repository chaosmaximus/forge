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

/// Generate all plausible edge-table ID formats for a given file path.
/// The edge table may contain:
///   - `file:relative/path` (canonical format used by decisions, clusters, calls)
///   - bare absolute paths (legacy import edges stored by the indexer)
///   - `file:absolute/path` (new import edge format after the indexer fix)
///   - bare relative paths
fn resolve_file_targets(file: &str) -> Vec<String> {
    let mut targets = Vec::with_capacity(6);

    // 1. Canonical: file:{relative_path}
    targets.push(format!("file:{file}"));

    // 2. Bare relative path
    targets.push(file.to_string());

    // 3. Try to compute the absolute path via project dir for matching legacy edges
    if let Some(raw_project_dir) = crate::workers::indexer::find_project_dir() {
        // Strip trailing slashes to prevent double-slash paths like /mnt/project//src/file.rs
        let project_dir = raw_project_dir.trim_end_matches('/');
        let abs = if file.starts_with('/') {
            file.to_string()
        } else {
            format!("{project_dir}/{file}")
        };
        // 4. file:{absolute_path}
        targets.push(format!("file:{abs}"));
        // 5. bare absolute path (legacy indexer format)
        targets.push(abs);
    } else if file.starts_with('/') {
        // File is already absolute
        targets.push(format!("file:{file}"));
        // bare absolute already covered by #2
    }

    // 6. Rust module path variants: convert file path to crate::module::path format
    // so that import edges stored as "crate::server::handler" can match against
    // a file like "crates/daemon/src/server/handler.rs".
    if file.ends_with(".rs") {
        if let Some(module_path) = file_path_to_rust_module(file) {
            targets.push(module_path);
        }
    }

    targets.sort();
    targets.dedup();
    targets
}

/// Convert a Rust file path to a `crate::module::path` format.
/// e.g., "crates/daemon/src/server/handler.rs" → "crate::server::handler"
/// e.g., "src/db/ops.rs" → "crate::db::ops"
/// Returns None if the path doesn't contain a `src/` segment.
fn file_path_to_rust_module(file: &str) -> Option<String> {
    let stem = file.trim_end_matches(".rs");
    // Find the "src/" segment — everything after it is the module path
    let after_src = if let Some(idx) = stem.find("/src/") {
        &stem[idx + 5..] // skip "/src/"
    } else if let Some(rest) = stem.strip_prefix("src/") {
        rest
    } else {
        return None;
    };
    // Skip lib and main — they are crate roots, not importable modules
    if after_src == "lib" || after_src == "main" {
        return None;
    }
    // Strip trailing /mod (e.g., server/mod → server)
    let module = after_src.trim_end_matches("/mod");
    // Convert path separators to Rust module separators
    let module_path = format!("crate::{}", module.replace('/', "::"));
    Some(module_path)
}

/// Main entry point: analyse the blast radius of changing `file`.
pub fn analyze_blast_radius(conn: &Connection, file: &str) -> BlastRadius {
    let targets = resolve_file_targets(file);

    let decisions = find_decisions(conn, &targets);
    let importers = find_importers(conn, &targets);
    let (callers, calling_files) = find_callers(conn, file);
    let (cluster_name, cluster_files) = find_cluster(conn, &targets);

    let decision_ids: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();
    let files_affected = find_co_affected_files(conn, &decision_ids, &targets);

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

/// Find all decisions that affect the given file target (trying all path formats).
/// Returns (id, title, confidence) triples.
fn find_decisions(conn: &Connection, targets: &[String]) -> Vec<(String, String, f64)> {
    if targets.is_empty() {
        return Vec::new();
    }
    let placeholders: Vec<String> = (1..=targets.len()).map(|i| format!("?{i}")).collect();
    let in_clause = placeholders.join(", ");
    let sql = format!(
        "SELECT DISTINCT m.id, m.title, m.confidence
         FROM edge e
         JOIN memory m ON e.from_id = m.id
         WHERE e.to_id IN ({in_clause})
           AND e.edge_type = 'affects'
           AND m.memory_type = 'decision'
           AND m.status = 'active'
         ORDER BY m.confidence DESC
         LIMIT 50"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let params: Vec<&dyn rusqlite::types::ToSql> =
        targets.iter().map(|t| t as &dyn rusqlite::types::ToSql).collect();
    let result = match stmt.query_map(params.as_slice(), |row| {
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

/// Find files that import the given file target (trying all path formats).
/// Returns from_id values stripped of the "file:" prefix.
fn find_importers(conn: &Connection, targets: &[String]) -> Vec<String> {
    if targets.is_empty() {
        return Vec::new();
    }
    let placeholders: Vec<String> = (1..=targets.len()).map(|i| format!("?{i}")).collect();
    let in_clause = placeholders.join(", ");
    let sql = format!(
        "SELECT DISTINCT e.from_id
         FROM edge e
         WHERE e.to_id IN ({in_clause})
           AND e.edge_type = 'imports'
         LIMIT 50"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let params: Vec<&dyn rusqlite::types::ToSql> =
        targets.iter().map(|t| t as &dyn rusqlite::types::ToSql).collect();
    let result = match stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0)) {
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

    // Convert file path to module pattern for import edge matching
    // e.g., "crates/daemon/src/server/handler.rs" → "%server::handler%"
    let module_pattern = {
        let stem = file
            .trim_end_matches(".rs")
            .trim_end_matches(".py")
            .trim_end_matches(".ts")
            .trim_end_matches(".tsx")
            .trim_end_matches(".js")
            .trim_end_matches(".go");
        let parts: Vec<&str> = stem.split('/').collect();
        let raw = if parts.len() >= 2 {
            format!("{}::{}", parts[parts.len()-2], parts[parts.len()-1])
        } else if !parts.is_empty() {
            parts[parts.len()-1].to_string()
        } else {
            return (0, Vec::new());
        };
        let escaped_mod = raw.replace('%', "\\%").replace('_', "\\_");
        format!("%{escaped_mod}%")
    };

    let sql = "
        SELECT DISTINCT from_id
        FROM edge
        WHERE (edge_type = 'calls' AND to_id LIKE ?1 ESCAPE '\\')
           OR (edge_type = 'imports' AND to_id LIKE ?2 ESCAPE '\\')
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
fn find_cluster(conn: &Connection, targets: &[String]) -> (Option<String>, Vec<String>) {
    if targets.is_empty() {
        return (None, Vec::new());
    }
    // Step 1: Find which cluster this file belongs to (trying all path formats)
    let placeholders: Vec<String> = (1..=targets.len()).map(|i| format!("?{i}")).collect();
    let in_clause = placeholders.join(", ");
    let cluster_sql = format!(
        "SELECT to_id
         FROM edge
         WHERE edge_type = 'belongs_to_cluster'
           AND from_id IN ({in_clause})
         LIMIT 1"
    );
    let params: Vec<&dyn rusqlite::types::ToSql> =
        targets.iter().map(|t| t as &dyn rusqlite::types::ToSql).collect();
    let cluster_id: Option<String> = conn
        .prepare(&cluster_sql)
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))
                .ok()
                .and_then(|mut rows| rows.next().and_then(|r| r.ok()))
        });

    let cluster_id = match cluster_id {
        Some(id) => id,
        None => return (None, Vec::new()),
    };

    // Step 2: Find all other files in the same cluster, excluding any of our target formats
    let exclude_placeholders: Vec<String> =
        (1..=targets.len()).map(|i| format!("?{}", i + 1)).collect();
    let exclude_clause = exclude_placeholders.join(", ");
    let members_sql = format!(
        "SELECT from_id
         FROM edge
         WHERE edge_type = 'belongs_to_cluster'
           AND to_id = ?1
           AND from_id NOT IN ({exclude_clause})"
    );
    let mut stmt = match conn.prepare(&members_sql) {
        Ok(s) => s,
        Err(_) => return (Some(cluster_id.clone()), Vec::new()),
    };
    let mut member_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    member_params.push(Box::new(cluster_id.clone()));
    for t in targets {
        member_params.push(Box::new(t.clone()));
    }
    let member_params_ref: Vec<&dyn rusqlite::types::ToSql> =
        member_params.iter().map(|p| p.as_ref()).collect();
    let files: Vec<String> = match stmt.query_map(member_params_ref.as_slice(), |row| {
        row.get::<_, String>(0)
    }) {
        Ok(rows) => rows
            .filter_map(|r| r.ok())
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
    exclude_targets: &[String],
) -> Vec<String> {
    if decision_ids.is_empty() {
        return Vec::new();
    }

    let id_placeholders: Vec<String> = (1..=decision_ids.len()).map(|i| format!("?{i}")).collect();
    let in_clause = id_placeholders.join(", ");
    // Exclude all path variants of the target file
    let exclude_start = decision_ids.len() + 1;
    let exclude_placeholders: Vec<String> = (0..exclude_targets.len())
        .map(|i| format!("?{}", exclude_start + i))
        .collect();
    let exclude_clause = exclude_placeholders.join(", ");
    let sql = format!(
        "SELECT DISTINCT e.to_id FROM edge e
         WHERE e.from_id IN ({in_clause})
         AND e.edge_type = 'affects'
         AND e.to_id NOT IN ({exclude_clause})
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
    for t in exclude_targets {
        param_values.push(Box::new(t.clone()));
    }
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

    #[test]
    fn test_blast_radius_bare_absolute_path_importers() {
        // Simulates the real DB format where the indexer stores bare absolute paths
        // as from_id (no "file:" prefix), matching the legacy edge format.
        // The to_id is a Rust module path like "crate::server::handler".
        let conn = setup_db();

        // Legacy indexer format: bare absolute paths as from_id, module as to_id
        // For file "crates/daemon/src/server/handler.rs", the module pattern is
        // "%server::handler%" which matches "crate::server::handler".
        store_edge(
            &conn,
            "/mnt/project/crates/daemon/src/main.rs",
            "crate::server::handler",
            "imports",
            "{}",
        )
        .unwrap();
        store_edge(
            &conn,
            "/mnt/project/crates/daemon/src/routes.rs",
            "crate::server::handler",
            "imports",
            "{}",
        )
        .unwrap();

        // find_callers uses LIKE "%server::handler%" on to_id
        let br = analyze_blast_radius(&conn, "crates/daemon/src/server/handler.rs");
        assert!(
            br.callers >= 2,
            "expected at least 2 callers from bare-path import edges, got {}",
            br.callers,
        );
        // The from_id values (bare abs paths) should appear in calling_files
        assert!(
            br.calling_files.contains(&"/mnt/project/crates/daemon/src/main.rs".to_string()),
            "calling_files should contain main.rs, got: {:?}",
            br.calling_files,
        );
    }

    #[test]
    fn test_blast_radius_mixed_edge_formats() {
        // Tests that blast-radius works with edges in mixed formats:
        // some with "file:" prefix and some with bare absolute paths.
        let conn = setup_db();

        // New format: file:-prefixed import edges (to_id also prefixed)
        store_edge(
            &conn,
            "file:/mnt/project/src/main.rs",
            "file:src/server/handler.rs",
            "imports",
            "{}",
        )
        .unwrap();

        // Legacy format: bare absolute path import edges (module as to_id)
        store_edge(
            &conn,
            "/mnt/project/src/routes.rs",
            "crate::server::handler",
            "imports",
            "{}",
        )
        .unwrap();

        let br = analyze_blast_radius(&conn, "src/server/handler.rs");
        // The file:-prefixed edge should match via find_importers
        assert!(
            br.importers.contains(&"/mnt/project/src/main.rs".to_string()),
            "importers should contain main.rs from file:-prefixed edge, got: {:?}",
            br.importers,
        );
        // The bare-path edge should match via find_callers (LIKE on module pattern)
        assert!(
            br.callers >= 1,
            "expected at least 1 caller from legacy bare-path import edge, got {}",
            br.callers,
        );
    }

    #[test]
    fn test_file_path_to_rust_module() {
        // Standard crate path
        assert_eq!(
            file_path_to_rust_module("crates/daemon/src/server/handler.rs"),
            Some("crate::server::handler".to_string()),
        );
        // Direct src/ path
        assert_eq!(
            file_path_to_rust_module("src/db/ops.rs"),
            Some("crate::db::ops".to_string()),
        );
        // mod.rs should strip trailing /mod
        assert_eq!(
            file_path_to_rust_module("src/server/mod.rs"),
            Some("crate::server".to_string()),
        );
        // lib.rs and main.rs are crate roots — not importable
        assert_eq!(file_path_to_rust_module("src/lib.rs"), None);
        assert_eq!(file_path_to_rust_module("src/main.rs"), None);
        // No src/ segment
        assert_eq!(file_path_to_rust_module("build.rs"), None);
        // Deeply nested
        assert_eq!(
            file_path_to_rust_module("crates/daemon/src/db/manas/layer.rs"),
            Some("crate::db::manas::layer".to_string()),
        );
    }

    #[test]
    fn test_importers_match_rust_module_path() {
        // Tests that find_importers returns results when edges use crate:: module paths
        // and we query by file path (which resolve_file_targets converts to module path).
        let conn = setup_db();

        // Store import edge with module-path to_id (as extract_imports produces)
        store_edge(
            &conn,
            "file:crates/daemon/src/main.rs",
            "crate::server::handler",
            "imports",
            "{}",
        )
        .unwrap();

        // resolve_file_targets should include "crate::server::handler" in targets
        let targets = resolve_file_targets("crates/daemon/src/server/handler.rs");
        assert!(
            targets.contains(&"crate::server::handler".to_string()),
            "targets should contain crate::server::handler, got: {targets:?}",
        );

        // find_importers should now match
        let importers = find_importers(&conn, &targets);
        assert!(
            !importers.is_empty(),
            "importers should find main.rs via module-path matching, got: {importers:?}",
        );
        assert!(
            importers.iter().any(|i| i.contains("main.rs")),
            "importers should contain main.rs, got: {importers:?}",
        );
    }
}
