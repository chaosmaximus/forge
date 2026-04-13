//! Chitta diagnostic table CRUD operations.
//!
//! The `diagnostic` table caches analysis results from cross-file consistency
//! checks, memory-informed repeat-bug detection, and (future) LSP diagnostics.
//! Results are ephemeral — auto-expire after 5 minutes (configurable per entry).

use rusqlite::{params, Connection};

/// A cached diagnostic entry.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub id: String,
    pub file_path: String,
    pub severity: String, // "error", "warning", "info", "hint"
    pub message: String,
    pub source: String, // "rust-analyzer", "pyright", "forge-consistency", "forge-memory"
    pub line: Option<i64>,
    pub column: Option<i64>,
    pub created_at: String,
    pub expires_at: String,
}

/// Store a diagnostic (INSERT OR REPLACE — upsert by id).
pub fn store_diagnostic(conn: &Connection, d: &Diagnostic) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO diagnostic (id, file_path, severity, message, source, line, col, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            d.id,
            d.file_path,
            d.severity,
            d.message,
            d.source,
            d.line,
            d.column,
            d.created_at,
            d.expires_at,
        ],
    )?;
    Ok(())
}

/// Get active (non-expired) diagnostics for a file.
/// Uses exact match first, then escaped suffix match for absolute/relative path compatibility.
/// Wildcards in file_path are escaped to prevent LIKE injection (Codex fix).
pub fn get_diagnostics(conn: &Connection, file_path: &str) -> rusqlite::Result<Vec<Diagnostic>> {
    // Escape LIKE wildcards in the file path to prevent overmatch
    let escaped = file_path.replace('%', "\\%").replace('_', "\\_");
    let like_pattern = format!("%{escaped}");
    let mut stmt = conn.prepare(
        "SELECT id, file_path, severity, message, source, line, col, created_at, expires_at
         FROM diagnostic WHERE (file_path = ?1 OR file_path LIKE ?2 ESCAPE '\\') AND expires_at > datetime('now')
         ORDER BY severity, line",
    )?;
    let rows = stmt.query_map(params![file_path, like_pattern], |row| {
        Ok(Diagnostic {
            id: row.get(0)?,
            file_path: row.get(1)?,
            severity: row.get(2)?,
            message: row.get(3)?,
            source: row.get(4)?,
            line: row.get(5)?,
            column: row.get(6)?,
            created_at: row.get(7)?,
            expires_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

/// Clear all diagnostics for a file (before re-populating).
/// Escapes LIKE wildcards to prevent cross-file deletion (Codex fix).
pub fn clear_diagnostics(conn: &Connection, file_path: &str) -> rusqlite::Result<usize> {
    let escaped = file_path.replace('%', "\\%").replace('_', "\\_");
    let like_pattern = format!("%{escaped}");
    conn.execute(
        "DELETE FROM diagnostic WHERE file_path = ?1 OR file_path LIKE ?2 ESCAPE '\\'",
        params![file_path, like_pattern],
    )
}

/// Expire old diagnostics (remove entries past their expires_at).
pub fn expire_diagnostics(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM diagnostic WHERE expires_at < datetime('now')",
        [],
    )
}

/// Get all active (non-expired) diagnostics across all files.
pub fn get_all_active_diagnostics(conn: &Connection) -> rusqlite::Result<Vec<Diagnostic>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, severity, message, source, line, col, created_at, expires_at
         FROM diagnostic WHERE expires_at > datetime('now')
         ORDER BY severity, file_path, line",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Diagnostic {
            id: row.get(0)?,
            file_path: row.get(1)?,
            severity: row.get(2)?,
            message: row.get(3)?,
            source: row.get(4)?,
            line: row.get(5)?,
            column: row.get(6)?,
            created_at: row.get(7)?,
            expires_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

/// Count active diagnostics by severity.
/// Returns (errors, warnings, hints).
pub fn diagnostic_summary(conn: &Connection) -> rusqlite::Result<(usize, usize, usize)> {
    let errors: i64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostic WHERE severity = 'error' AND expires_at > datetime('now')",
        [],
        |row| row.get(0),
    )?;
    let warnings: i64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostic WHERE severity = 'warning' AND expires_at > datetime('now')",
        [],
        |row| row.get(0),
    )?;
    let hints: i64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostic WHERE severity IN ('info', 'hint') AND expires_at > datetime('now')",
        [],
        |row| row.get(0),
    )?;
    Ok((errors as usize, warnings as usize, hints as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_store_and_get_diagnostic() {
        let conn = setup();
        let d = Diagnostic {
            id: "test-1".into(),
            file_path: "src/main.rs".into(),
            severity: "error".into(),
            message: "undefined variable x".into(),
            source: "pyright".into(),
            line: Some(10),
            column: Some(5),
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        store_diagnostic(&conn, &d).unwrap();

        let results = get_diagnostics(&conn, "src/main.rs").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "test-1");
        assert_eq!(results[0].message, "undefined variable x");
        assert_eq!(results[0].source, "pyright");
        assert_eq!(results[0].line, Some(10));
        assert_eq!(results[0].column, Some(5));
    }

    #[test]
    fn test_get_diagnostics_filters_expired() {
        let conn = setup();
        // Store a diagnostic that already expired
        let d = Diagnostic {
            id: "expired-1".into(),
            file_path: "src/lib.rs".into(),
            severity: "warning".into(),
            message: "stale check".into(),
            source: "forge-consistency".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_offset(-600),
            expires_at: forge_core::time::now_offset(-300), // expired 5 min ago
        };
        store_diagnostic(&conn, &d).unwrap();

        let results = get_diagnostics(&conn, "src/lib.rs").unwrap();
        assert_eq!(
            results.len(),
            0,
            "expired diagnostics should not be returned"
        );
    }

    #[test]
    fn test_clear_diagnostics() {
        let conn = setup();
        let d1 = Diagnostic {
            id: "c1".into(),
            file_path: "src/auth.rs".into(),
            severity: "warning".into(),
            message: "callers exist".into(),
            source: "forge-consistency".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        let d2 = Diagnostic {
            id: "c2".into(),
            file_path: "src/other.rs".into(),
            severity: "error".into(),
            message: "something else".into(),
            source: "forge-memory".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        store_diagnostic(&conn, &d1).unwrap();
        store_diagnostic(&conn, &d2).unwrap();

        let cleared = clear_diagnostics(&conn, "src/auth.rs").unwrap();
        assert_eq!(cleared, 1);

        // auth.rs gone, other.rs still there
        let remaining = get_diagnostics(&conn, "src/other.rs").unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_expire_diagnostics() {
        let conn = setup();
        let fresh = Diagnostic {
            id: "fresh".into(),
            file_path: "src/a.rs".into(),
            severity: "error".into(),
            message: "fresh error".into(),
            source: "pyright".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        let stale = Diagnostic {
            id: "stale".into(),
            file_path: "src/b.rs".into(),
            severity: "warning".into(),
            message: "stale warning".into(),
            source: "forge-memory".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_offset(-600),
            expires_at: forge_core::time::now_offset(-100),
        };
        store_diagnostic(&conn, &fresh).unwrap();
        store_diagnostic(&conn, &stale).unwrap();

        let expired_count = expire_diagnostics(&conn).unwrap();
        assert_eq!(expired_count, 1);

        // Only fresh remains
        let all: i64 = conn
            .query_row("SELECT COUNT(*) FROM diagnostic", [], |r| r.get(0))
            .unwrap();
        assert_eq!(all, 1);
    }

    #[test]
    fn test_diagnostic_summary() {
        let conn = setup();
        for (id, sev) in &[
            ("e1", "error"),
            ("e2", "error"),
            ("w1", "warning"),
            ("h1", "hint"),
        ] {
            let d = Diagnostic {
                id: id.to_string(),
                file_path: "src/test.rs".into(),
                severity: sev.to_string(),
                message: format!("{sev} msg"),
                source: "test".into(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(300),
            };
            store_diagnostic(&conn, &d).unwrap();
        }

        let (errors, warnings, hints) = diagnostic_summary(&conn).unwrap();
        assert_eq!(errors, 2);
        assert_eq!(warnings, 1);
        assert_eq!(hints, 1);
    }

    #[test]
    fn test_upsert_diagnostic() {
        let conn = setup();
        let d = Diagnostic {
            id: "upsert-1".into(),
            file_path: "src/main.rs".into(),
            severity: "error".into(),
            message: "first message".into(),
            source: "test".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        store_diagnostic(&conn, &d).unwrap();

        // Update the same id with a new message
        let d2 = Diagnostic {
            id: "upsert-1".into(),
            file_path: "src/main.rs".into(),
            severity: "warning".into(),
            message: "updated message".into(),
            source: "test".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        store_diagnostic(&conn, &d2).unwrap();

        let results = get_diagnostics(&conn, "src/main.rs").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "updated message");
        assert_eq!(results[0].severity, "warning");
    }
}
