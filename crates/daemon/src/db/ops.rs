use rusqlite::{Connection, params};
use forge_core::types::{Memory, MemoryType, CodeFile, CodeSymbol};

/// BM25 search result
#[derive(Debug, Clone)]
pub struct BM25Result {
    pub id: String,
    pub title: String,
    pub content: String,
    pub score: f64,
    pub memory_type: String,
    pub confidence: f64,
}

/// Health counts per memory type + edges
#[derive(Debug, Clone, Default)]
pub struct HealthCounts {
    pub decisions: usize,
    pub lessons: usize,
    pub patterns: usize,
    pub preferences: usize,
    pub edges: usize,
}

fn type_str(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::Decision => "decision",
        MemoryType::Lesson => "lesson",
        MemoryType::Pattern => "pattern",
        MemoryType::Preference => "preference",
    }
}

/// Insert or replace a memory record into the database.
pub fn remember(conn: &Connection, memory: &Memory) -> rusqlite::Result<()> {
    let mt = type_str(&memory.memory_type);
    let status = serde_json::to_value(&memory.status)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "active".to_string());
    let tags_json = serde_json::to_string(&memory.tags).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT OR REPLACE INTO memory
            (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            memory.id,
            mt,
            memory.title,
            memory.content,
            memory.confidence,
            status,
            memory.project,
            tags_json,
            memory.created_at,
            memory.accessed_at,
        ],
    )?;
    Ok(())
}

/// NEW-2: Sanitize user input for FTS5 MATCH by stripping non-alphanumeric chars
/// and wrapping each surviving word in double-quotes. This prevents FTS5 operator
/// injection (AND, OR, NOT, NEAR, *, ^, etc.) and avoids parse errors from bare
/// punctuation tokens that FTS5 rejects even inside quotes.
///
/// Terms are joined with OR so that a query like "JWT AND bad" matches documents
/// containing any of the words, not requiring all of them to be present.
fn sanitize_fts5_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter_map(|word| {
            // Strip characters that are not alphanumeric or underscore
            let cleaned: String = word.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
            if cleaned.is_empty() {
                return None; // drop pure-punctuation tokens like "*"
            }
            // FTS5 escape: double any internal double-quotes (shouldn't exist after cleaning, but defensive)
            let escaped = cleaned.replace('"', "\"\"");
            Some(format!("\"{}\"", escaped))
        })
        .collect();

    if terms.is_empty() {
        return String::new();
    }

    terms.join(" OR ")
}

/// Full-text search using FTS5 BM25 scoring. Returns active memories ranked by relevance.
pub fn recall_bm25(conn: &Connection, query: &str, limit: usize) -> rusqlite::Result<Vec<BM25Result>> {
    // NEW-2: Sanitize the query to prevent FTS5 operator injection
    let safe_query = sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new()); // No valid search terms after sanitization
    }

    let sql = "
        SELECT m.id, m.title, m.content, bm25(memory_fts) AS score, m.memory_type, m.confidence
        FROM memory_fts
        JOIN memory m ON memory_fts.rowid = m.rowid
        WHERE memory_fts MATCH ?1
          AND m.status = 'active'
        ORDER BY score
        LIMIT ?2
    ";

    let mut stmt = conn.prepare(sql)?;
    let results = stmt.query_map(params![safe_query, limit as i64], |row| {
        Ok(BM25Result {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            score: {
                let raw: f64 = row.get(3)?;
                raw.abs()
            },
            memory_type: row.get(4)?,
            confidence: row.get(5)?,
        })
    })?;

    results.collect()
}

/// Soft-delete a memory by setting status to 'superseded'.
/// Returns true if a row was updated (was active before).
pub fn forget(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let rows_changed = conn.execute(
        "UPDATE memory SET status = 'superseded' WHERE id = ?1 AND status = 'active'",
        params![id],
    )?;
    Ok(rows_changed > 0)
}

/// Count active memories per type and total edges.
pub fn health(conn: &Connection) -> rusqlite::Result<HealthCounts> {
    let count_type = |type_name: &str| -> rusqlite::Result<usize> {
        conn.query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = ?1 AND status = 'active'",
            params![type_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
    };

    let decisions = count_type("decision")?;
    let lessons = count_type("lesson")?;
    let patterns = count_type("pattern")?;
    let preferences = count_type("preference")?;

    let edges: usize = conn
        .query_row("SELECT COUNT(*) FROM edge", [], |row| row.get::<_, i64>(0))
        .map(|n| n as usize)?;

    Ok(HealthCounts {
        decisions,
        lessons,
        patterns,
        preferences,
        edges,
    })
}

/// Mark memories as "faded" when their effective confidence drops below 0.1.
///
/// Effective confidence is computed as: stored_confidence * exp(-0.03 * days_since_accessed).
/// The stored `confidence` field is NEVER modified by decay — it represents the base
/// confidence set at creation/update time. This avoids the over-decay bug where repeated
/// consolidation runs would multiply already-decayed values by the full time factor again
/// (exponential-over-exponential decay).
///
/// Returns (checked_count, faded_count).
pub fn decay_memories(conn: &Connection) -> rusqlite::Result<(usize, usize)> {
    let mut stmt = conn.prepare(
        "SELECT id, confidence, accessed_at FROM memory WHERE status = 'active'"
    )?;

    let rows: Vec<(String, f64, String)> = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
        ))
    })?.filter_map(|r| r.ok()).collect();

    let checked = rows.len();
    let mut faded_count = 0usize;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as f64;

    for (id, confidence, accessed_at) in &rows {
        let accessed_secs = parse_timestamp_to_epoch(accessed_at).unwrap_or(now_secs);
        let days_since = ((now_secs - accessed_secs) / 86400.0).max(0.0);
        let effective = confidence * (-0.03 * days_since).exp();

        if effective < 0.1 {
            conn.execute(
                "UPDATE memory SET status = 'faded' WHERE id = ?1",
                params![id],
            )?;
            faded_count += 1;
        }
    }

    Ok((checked, faded_count))
}

/// Parse a timestamp string to epoch seconds.
///
/// Handles two formats produced by SQLite and Rust code:
/// - Pure epoch seconds: "1743548000"
/// - SQLite datetime: "2026-04-02 12:00:00" or ISO 8601 "2026-04-02T12:00:00Z"
fn parse_timestamp_to_epoch(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    // Try epoch seconds first
    let trimmed = s.trim().trim_end_matches('Z');
    if let Ok(secs) = trimmed.parse::<f64>() {
        if secs > 1_000_000_000.0 {
            return Some(secs);
        }
    }
    // Try SQLite datetime format: "YYYY-MM-DD HH:MM:SS" or ISO 8601 with T
    let parts: Vec<&str> = s.split(&['-', ' ', ':', 'T'][..]).collect();
    if parts.len() >= 6 {
        let y: f64 = parts[0].parse().ok()?;
        let m: f64 = parts[1].parse().ok()?;
        let d: f64 = parts[2].parse().ok()?;
        let h: f64 = parts[3].parse().ok()?;
        let min: f64 = parts[4].parse().ok()?;
        let sec: f64 = parts[5].trim_end_matches('Z').parse().ok()?;
        // Approximate conversion (good enough for decay calculation — off by at most ~1 day)
        let days = (y - 1970.0) * 365.25 + (m - 1.0) * 30.44 + d;
        return Some(days * 86400.0 + h * 3600.0 + min * 60.0 + sec);
    }
    None
}

/// Update accessed_at for each given id (best-effort — errors are ignored).
pub fn touch(conn: &Connection, ids: &[&str]) {
    for id in ids {
        let _ = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now') WHERE id = ?1",
            params![id],
        );
    }
}

/// Insert or replace a code file record.
pub fn store_file(conn: &Connection, file: &CodeFile) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO code_file (id, path, language, project, hash, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![file.id, file.path, file.language, file.project, file.hash, file.indexed_at],
    )?;
    Ok(())
}

/// Insert or replace a code symbol record.
pub fn store_symbol(conn: &Connection, symbol: &CodeSymbol) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO code_symbol (id, name, kind, file_path, line_start, line_end, signature) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![symbol.id, symbol.name, symbol.kind, symbol.file_path, symbol.line_start, symbol.line_end, symbol.signature],
    )?;
    Ok(())
}

/// Delete code_file and code_symbol rows whose paths are not in `current_paths`.
/// Called after indexing to remove stale entries for files that have been deleted or renamed.
/// Returns the total number of rows deleted (files + symbols).
pub fn cleanup_stale_files(conn: &Connection, current_paths: &[&str]) -> rusqlite::Result<usize> {
    if current_paths.is_empty() {
        // No files indexed — don't wipe the whole table (could be an indexer failure)
        return Ok(0);
    }

    conn.execute("CREATE TEMP TABLE IF NOT EXISTS _current_paths (path TEXT PRIMARY KEY)", [])?;
    conn.execute("DELETE FROM _current_paths", [])?;

    for path in current_paths {
        conn.execute(
            "INSERT OR IGNORE INTO _current_paths (path) VALUES (?1)",
            params![path],
        )?;
    }

    let deleted_symbols = conn.execute(
        "DELETE FROM code_symbol WHERE file_path NOT IN (SELECT path FROM _current_paths)",
        [],
    )?;
    let deleted_files = conn.execute(
        "DELETE FROM code_file WHERE path NOT IN (SELECT path FROM _current_paths)",
        [],
    )?;

    conn.execute("DROP TABLE IF EXISTS _current_paths", [])?;

    Ok(deleted_files + deleted_symbols)
}

/// Count total code files in the database.
pub fn count_files(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT count(*) FROM code_file", [], |r| r.get(0))
}

/// Count total code symbols in the database.
pub fn count_symbols(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT count(*) FROM code_symbol", [], |r| r.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use forge_core::types::{Memory, MemoryType, CodeFile, CodeSymbol};

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_remember_and_recall() {
        let conn = open_db();

        let m = Memory::new(MemoryType::Decision, "Use SQLite for storage", "SQLite FTS5 gives fast BM25 recall");
        remember(&conn, &m).unwrap();

        let results = recall_bm25(&conn, "SQLite", 10).unwrap();
        assert!(!results.is_empty(), "should find at least one result");
        assert_eq!(results[0].id, m.id);
        assert!(results[0].score > 0.0, "BM25 score should be positive");
    }

    #[test]
    fn test_forget() {
        let conn = open_db();

        let m = Memory::new(MemoryType::Lesson, "TDD always", "Write tests first");
        remember(&conn, &m).unwrap();

        // Should recall before forgetting
        let before = recall_bm25(&conn, "TDD", 10).unwrap();
        assert!(!before.is_empty());

        let deleted = forget(&conn, &m.id).unwrap();
        assert!(deleted, "forget should return true for active memory");

        // After forgetting, recall should return 0
        let after = recall_bm25(&conn, "TDD", 10).unwrap();
        assert_eq!(after.len(), 0, "superseded memory should not appear in recall");

        // Second forget on same id should return false
        let again = forget(&conn, &m.id).unwrap();
        assert!(!again, "second forget should return false");
    }

    #[test]
    fn test_recall_bm25_special_characters() {
        let conn = open_db();

        let m = Memory::new(MemoryType::Decision, "Use JWT", "For auth");
        remember(&conn, &m).unwrap();

        // Should not crash or error on FTS5 operators
        let results = recall_bm25(&conn, "JWT AND OR NOT *", 10).unwrap();
        // Should return results (JWT matches) without FTS5 parse error
        assert!(!results.is_empty(), "should find JWT despite FTS5 operator chars in query");
    }

    #[test]
    fn test_sanitize_fts5_query() {
        let sanitized = sanitize_fts5_query("JWT AND authentication NOT bad");
        assert_eq!(sanitized, r#""JWT" OR "AND" OR "authentication" OR "NOT" OR "bad""#);

        // Punctuation-only tokens are dropped
        let sanitized2 = sanitize_fts5_query("hello * world");
        assert_eq!(sanitized2, r#""hello" OR "world""#);

        // Mixed punctuation stripped, alphanumeric kept
        let sanitized3 = sanitize_fts5_query("^prefix$ foo-bar");
        assert_eq!(sanitized3, r#""prefix" OR "foobar""#);

        // Empty input
        let sanitized4 = sanitize_fts5_query("* ^ !");
        assert_eq!(sanitized4, "");
    }

    #[test]
    fn test_decay_memories_does_not_modify_confidence() {
        let conn = open_db();
        // Insert a 30-day-old memory (effective conf = 0.9 * exp(-0.03*30) ~ 0.37 — still above 0.1)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('mid1', 'decision', 'Mid decision', 'content', 0.9, 'active', '[]',
                     datetime('now', '-30 days'), datetime('now', '-30 days'))",
            [],
        ).unwrap();
        // Insert recent memory
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('new1', 'decision', 'New decision', 'content', 0.9, 'active', '[]',
                     datetime('now'), datetime('now'))",
            [],
        ).unwrap();

        let (checked, faded) = decay_memories(&conn).unwrap();
        assert_eq!(checked, 2, "should check both memories");
        assert_eq!(faded, 0, "30-day memory at 0.9 base should not be faded yet");

        // Crucially: stored confidence is NEVER modified
        let mid_conf: f64 = conn.query_row("SELECT confidence FROM memory WHERE id = 'mid1'", [], |r| r.get(0)).unwrap();
        assert!((mid_conf - 0.9).abs() < 0.001, "stored confidence must remain 0.9, got {}", mid_conf);

        let new_conf: f64 = conn.query_row("SELECT confidence FROM memory WHERE id = 'new1'", [], |r| r.get(0)).unwrap();
        assert!((new_conf - 0.9).abs() < 0.001, "stored confidence must remain 0.9, got {}", new_conf);
    }

    #[test]
    fn test_decay_memories_fades_old_memory() {
        let conn = open_db();
        // Insert 90-day-old memory (effective conf = 0.9 * exp(-0.03*90) ~ 0.06 — below 0.1)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('old1', 'decision', 'Old decision', 'content', 0.9, 'active', '[]',
                     datetime('now', '-90 days'), datetime('now', '-90 days'))",
            [],
        ).unwrap();
        // Insert recent memory (should NOT fade)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('new1', 'decision', 'New decision', 'content', 0.9, 'active', '[]',
                     datetime('now'), datetime('now'))",
            [],
        ).unwrap();

        let (checked, faded) = decay_memories(&conn).unwrap();
        assert_eq!(checked, 2);
        assert_eq!(faded, 1, "90-day-old memory should be faded");

        let old_status: String = conn.query_row("SELECT status FROM memory WHERE id = 'old1'", [], |r| r.get(0)).unwrap();
        assert_eq!(old_status, "faded");

        let new_status: String = conn.query_row("SELECT status FROM memory WHERE id = 'new1'", [], |r| r.get(0)).unwrap();
        assert_eq!(new_status, "active");

        // Stored confidence is STILL not modified
        let old_conf: f64 = conn.query_row("SELECT confidence FROM memory WHERE id = 'old1'", [], |r| r.get(0)).unwrap();
        assert!((old_conf - 0.9).abs() < 0.001, "stored confidence must remain 0.9 even after fading, got {}", old_conf);
    }

    #[test]
    fn test_decay_idempotent_across_runs() {
        let conn = open_db();
        // Insert a 30-day-old memory (effective conf ~ 0.37 — above threshold)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('m1', 'decision', 'D1', 'c', 0.9, 'active', '[]',
                     datetime('now', '-30 days'), datetime('now', '-30 days'))",
            [],
        ).unwrap();

        // Run decay multiple times — result should be identical each time
        let (_, faded1) = decay_memories(&conn).unwrap();
        let (_, faded2) = decay_memories(&conn).unwrap();
        let (_, faded3) = decay_memories(&conn).unwrap();

        assert_eq!(faded1, faded2, "repeated decay runs must produce same result");
        assert_eq!(faded2, faded3, "repeated decay runs must produce same result");

        // Confidence is still untouched
        let conf: f64 = conn.query_row("SELECT confidence FROM memory WHERE id = 'm1'", [], |r| r.get(0)).unwrap();
        assert!((conf - 0.9).abs() < 0.001, "confidence must not change across multiple decay runs, got {}", conf);
    }

    #[test]
    fn test_parse_timestamp_to_epoch() {
        // Epoch seconds
        let epoch = parse_timestamp_to_epoch("1743548000");
        assert!(epoch.is_some());
        assert!((epoch.unwrap() - 1743548000.0).abs() < 1.0);

        // Empty string
        assert!(parse_timestamp_to_epoch("").is_none());

        // SQLite datetime format — just verify it parses to something reasonable
        let dt = parse_timestamp_to_epoch("2026-04-02 12:00:00");
        assert!(dt.is_some());
        assert!(dt.unwrap() > 1_700_000_000.0, "parsed datetime should be a reasonable epoch");

        // ISO 8601 with T
        let iso = parse_timestamp_to_epoch("2026-04-02T12:00:00Z");
        assert!(iso.is_some());
    }

    #[test]
    fn test_health_counts() {
        let conn = open_db();

        let d1 = Memory::new(MemoryType::Decision, "Decision one", "content one");
        let d2 = Memory::new(MemoryType::Decision, "Decision two", "content two");
        let l1 = Memory::new(MemoryType::Lesson, "Lesson one", "lesson content");

        remember(&conn, &d1).unwrap();
        remember(&conn, &d2).unwrap();
        remember(&conn, &l1).unwrap();

        let counts = health(&conn).unwrap();
        assert_eq!(counts.decisions, 2);
        assert_eq!(counts.lessons, 1);
        assert_eq!(counts.patterns, 0);
        assert_eq!(counts.preferences, 0);
        assert_eq!(counts.edges, 0);
    }

    #[test]
    fn test_store_file_and_symbol() {
        let conn = open_db();

        let file = CodeFile {
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "abc".into(),
            indexed_at: "2026-04-02".into(),
        };
        store_file(&conn, &file).unwrap();
        assert_eq!(count_files(&conn).unwrap(), 1);

        let sym = CodeSymbol {
            id: "s1".into(),
            name: "main".into(),
            kind: "function".into(),
            file_path: "src/main.rs".into(),
            line_start: 1,
            line_end: Some(10),
            signature: Some("fn main()".into()),
        };
        store_symbol(&conn, &sym).unwrap();
        assert_eq!(count_symbols(&conn).unwrap(), 1);
    }

    #[test]
    fn test_cleanup_stale_files() {
        let conn = open_db();

        // Insert two files and symbols
        let f1 = CodeFile {
            id: "f1".into(), path: "src/main.rs".into(), language: "rust".into(),
            project: "forge".into(), hash: "a".into(), indexed_at: "1".into(),
        };
        let f2 = CodeFile {
            id: "f2".into(), path: "src/old.rs".into(), language: "rust".into(),
            project: "forge".into(), hash: "b".into(), indexed_at: "1".into(),
        };
        store_file(&conn, &f1).unwrap();
        store_file(&conn, &f2).unwrap();

        let s1 = CodeSymbol {
            id: "s1".into(), name: "main".into(), kind: "function".into(),
            file_path: "src/main.rs".into(), line_start: 1, line_end: Some(10),
            signature: Some("fn main()".into()),
        };
        let s2 = CodeSymbol {
            id: "s2".into(), name: "old_fn".into(), kind: "function".into(),
            file_path: "src/old.rs".into(), line_start: 1, line_end: Some(5),
            signature: Some("fn old_fn()".into()),
        };
        store_symbol(&conn, &s1).unwrap();
        store_symbol(&conn, &s2).unwrap();

        assert_eq!(count_files(&conn).unwrap(), 2);
        assert_eq!(count_symbols(&conn).unwrap(), 2);

        // After re-index, only src/main.rs exists — old.rs was deleted
        let cleaned = cleanup_stale_files(&conn, &["src/main.rs"]).unwrap();
        assert_eq!(cleaned, 2, "should delete 1 file + 1 symbol for old.rs");

        assert_eq!(count_files(&conn).unwrap(), 1);
        assert_eq!(count_symbols(&conn).unwrap(), 1);
    }

    #[test]
    fn test_cleanup_stale_files_empty_preserves() {
        let conn = open_db();

        let f1 = CodeFile {
            id: "f1".into(), path: "src/main.rs".into(), language: "rust".into(),
            project: "forge".into(), hash: "a".into(), indexed_at: "1".into(),
        };
        store_file(&conn, &f1).unwrap();
        assert_eq!(count_files(&conn).unwrap(), 1);

        // Empty current_paths should NOT wipe existing data (safety)
        let cleaned = cleanup_stale_files(&conn, &[]).unwrap();
        assert_eq!(cleaned, 0);
        assert_eq!(count_files(&conn).unwrap(), 1);
    }

    #[test]
    fn test_store_file_upsert() {
        let conn = open_db();

        let file = CodeFile {
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "abc".into(),
            indexed_at: "2026-04-02".into(),
        };
        store_file(&conn, &file).unwrap();

        // Upsert same id with new hash
        let file2 = CodeFile {
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "def".into(),
            indexed_at: "2026-04-03".into(),
        };
        store_file(&conn, &file2).unwrap();
        assert_eq!(count_files(&conn).unwrap(), 1, "upsert should not duplicate");

        let stored_hash: String = conn.query_row(
            "SELECT hash FROM code_file WHERE id = 'f1'", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(stored_hash, "def", "upsert should update hash");
    }
}
