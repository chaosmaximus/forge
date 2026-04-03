use rusqlite::{Connection, params};
use forge_v2_core::types::{Memory, MemoryType};

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

/// Update accessed_at for each given id (best-effort — errors are ignored).
pub fn touch(conn: &Connection, ids: &[&str]) {
    for id in ids {
        let _ = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now') WHERE id = ?1",
            params![id],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use forge_v2_core::types::{Memory, MemoryType};

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
}
