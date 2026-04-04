use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use forge_core::types::{Memory, MemoryType, MemoryStatus, CodeFile, CodeSymbol};

/// BM25 search result
#[derive(Debug, Clone)]
pub struct BM25Result {
    pub id: String,
    pub title: String,
    pub content: String,
    pub score: f64,
    pub memory_type: String,
    pub confidence: f64,
    pub valence: String,
    pub intensity: f64,
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

fn status_str(ms: &MemoryStatus) -> &'static str {
    match ms {
        MemoryStatus::Active => "active",
        MemoryStatus::Superseded => "superseded",
        MemoryStatus::Reverted => "reverted",
        MemoryStatus::Faded => "faded",
        MemoryStatus::Conflict => "conflict",
    }
}

pub fn status_from_str(s: &str) -> MemoryStatus {
    match s {
        "active" => MemoryStatus::Active,
        "superseded" => MemoryStatus::Superseded,
        "reverted" => MemoryStatus::Reverted,
        "faded" => MemoryStatus::Faded,
        "conflict" => MemoryStatus::Conflict,
        _ => MemoryStatus::Active,
    }
}

/// Insert or update a memory record, deduplicating by title + type.
///
/// If an active memory with the same title and type already exists, its content
/// is updated and its confidence is bumped to the higher of the two values.
/// This prevents the extractor from creating 18 copies of the same decision
/// when it re-processes overlapping transcript chunks.
pub fn remember(conn: &Connection, memory: &Memory) -> rusqlite::Result<()> {
    let mt = type_str(&memory.memory_type);
    let status = status_str(&memory.status);
    let tags_json = serde_json::to_string(&memory.tags).unwrap_or_else(|_| "[]".to_string());

    // Check for existing memory with same title, type, AND project.
    // Including project in the dedup key prevents cross-project merging where a
    // decision from "proj-a" silently overwrites an identically-titled decision
    // from "proj-b".
    let existing_id: Option<String> = conn.query_row(
        "SELECT id FROM memory WHERE title = ?1 AND memory_type = ?2 AND COALESCE(project, '') = COALESCE(?3, '') AND status = 'active'",
        params![memory.title, mt, memory.project],
        |row| row.get(0),
    ).optional()?;

    if let Some(existing_id) = existing_id {
        // Update existing — bump confidence if higher, update content
        conn.execute(
            "UPDATE memory SET content = ?1, confidence = MAX(confidence, ?2), accessed_at = ?3,
             hlc_timestamp = ?4, node_id = ?5
             WHERE id = ?6",
            params![memory.content, memory.confidence, memory.accessed_at,
                    memory.hlc_timestamp, memory.node_id, existing_id],
        )?;
    } else {
        // Insert new
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                memory.id, mt, memory.title, memory.content,
                memory.confidence, status,
                memory.project, tags_json,
                memory.created_at, memory.accessed_at,
                memory.valence, memory.intensity,
                memory.hlc_timestamp, memory.node_id,
                memory.session_id, memory.access_count as i64,
            ],
        )?;
    }
    Ok(())
}

/// Insert a memory without dedup checking. Used for storing conflict versions
/// where we need both the local and remote copy preserved.
pub fn remember_raw(conn: &Connection, memory: &Memory) -> rusqlite::Result<()> {
    let mt = type_str(&memory.memory_type);
    let status = status_str(&memory.status);
    let tags_json = serde_json::to_string(&memory.tags).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            memory.id, mt, memory.title, memory.content,
            memory.confidence, status,
            memory.project, tags_json,
            memory.created_at, memory.accessed_at,
            memory.valence, memory.intensity,
            memory.hlc_timestamp, memory.node_id,
            memory.session_id, memory.access_count as i64,
        ],
    )?;
    Ok(())
}

/// Remove duplicate memories, keeping the one with highest confidence for each title+type.
/// Returns the number of rows deleted.
pub fn dedup_memories(conn: &Connection) -> rusqlite::Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM memory WHERE id NOT IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY title, memory_type
                    ORDER BY confidence DESC, created_at DESC
                ) as rn
                FROM memory WHERE status = 'active'
            ) WHERE rn = 1
        ) AND status = 'active'",
        [],
    )?;
    Ok(deleted)
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
        SELECT m.id, m.title, m.content, bm25(memory_fts) AS score, m.memory_type, m.confidence, m.valence, m.intensity
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
            valence: row.get(6)?,
            intensity: row.get(7)?,
        })
    })?;

    results.collect()
}

/// Full-text search using FTS5 BM25 scoring with optional project filter.
///
/// When `project` is `Some("X")`, returns only memories where `project = 'X'`
/// OR `project IS NULL` OR `project = ''` (global memories visible in every project).
/// When `project` is `None`, returns all active memories (existing behavior).
pub fn recall_bm25_project(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<BM25Result>> {
    let safe_query = sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new());
    }

    match project {
        Some(proj) => {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.title, m.content, bm25(memory_fts) AS score, m.memory_type, m.confidence, m.valence, m.intensity
                 FROM memory_fts
                 JOIN memory m ON memory_fts.rowid = m.rowid
                 WHERE memory_fts MATCH ?1
                   AND m.status = 'active'
                   AND (m.project = ?2 OR m.project IS NULL OR m.project = '')
                 ORDER BY score
                 LIMIT ?3"
            )?;
            let results = stmt.query_map(params![safe_query, proj, limit as i64], |row| {
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
                    valence: row.get(6)?,
                    intensity: row.get(7)?,
                })
            })?;
            results.collect()
        }
        None => recall_bm25(conn, query, limit),
    }
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

/// Health counts grouped by project.
pub fn health_by_project(conn: &Connection) -> rusqlite::Result<std::collections::HashMap<String, HealthCounts>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(project, ''), '_global') as proj, memory_type, count(*) as cnt
         FROM memory WHERE status = 'active' GROUP BY proj, memory_type"
    )?;

    let mut projects: std::collections::HashMap<String, HealthCounts> = std::collections::HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, usize>(2)?))
    })?;

    for row in rows.flatten() {
        let (proj, mtype, count) = row;
        let entry = projects.entry(proj).or_default();
        match mtype.as_str() {
            "decision" => entry.decisions = count,
            "lesson" => entry.lessons = count,
            "pattern" => entry.patterns = count,
            "preference" => entry.preferences = count,
            _ => {}
        }
    }

    // Add total edge count to each project (simplified — all projects see total edges)
    let total_edges: usize = conn.query_row("SELECT count(*) FROM edge", [], |r| r.get(0)).unwrap_or(0);
    for counts in projects.values_mut() {
        counts.edges = total_edges;
    }

    Ok(projects)
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
pub fn parse_timestamp_to_epoch(s: &str) -> Option<f64> {
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

/// Update accessed_at and increment access_count for each given id (best-effort — errors are ignored).
pub fn touch(conn: &Connection, ids: &[&str]) {
    for id in ids {
        // Codex fix: cap access_count at 1000, only increment if last access > 60s ago
        // Prevents gaming via repeated recall to inflate confidence
        let _ = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now'),
             access_count = MIN(access_count + 1, 1000)
             WHERE id = ?1
             AND (accessed_at < datetime('now', '-60 seconds') OR access_count = 0)",
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

/// Export all active memories as full Memory objects.
pub fn export_memories(conn: &Connection) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count
         FROM memory WHERE status = 'active' ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        let mt_str: String = row.get(1)?;
        let memory_type = match mt_str.as_str() {
            "decision" => MemoryType::Decision,
            "lesson" => MemoryType::Lesson,
            "pattern" => MemoryType::Pattern,
            "preference" => MemoryType::Preference,
            _ => MemoryType::Decision, // fallback
        };
        let tags_json: String = row.get(7)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        Ok(Memory {
            id: row.get(0)?,
            memory_type,
            title: row.get(2)?,
            content: row.get(3)?,
            confidence: row.get(4)?,
            status: forge_core::types::MemoryStatus::Active,
            project: row.get(6)?,
            tags,
            embedding: None,
            created_at: row.get(8)?,
            accessed_at: row.get(9)?,
            valence: row.get(10)?,
            intensity: row.get(11)?,
            hlc_timestamp: row.get(12)?,
            node_id: row.get(13)?,
            session_id: row.get::<_, String>(14).unwrap_or_default(),
            access_count: row.get::<_, i64>(15).unwrap_or(0) as u64,
        })
    })?;
    rows.collect()
}

/// Export all code files.
pub fn export_files(conn: &Connection) -> rusqlite::Result<Vec<CodeFile>> {
    let mut stmt = conn.prepare("SELECT id, path, language, project, hash, indexed_at FROM code_file")?;
    let rows = stmt.query_map([], |row| {
        Ok(CodeFile {
            id: row.get(0)?,
            path: row.get(1)?,
            language: row.get(2)?,
            project: row.get(3)?,
            hash: row.get(4)?,
            indexed_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Export all code symbols.
pub fn export_symbols(conn: &Connection) -> rusqlite::Result<Vec<CodeSymbol>> {
    let mut stmt = conn.prepare("SELECT id, name, kind, file_path, line_start, line_end, signature FROM code_symbol")?;
    let rows = stmt.query_map([], |row| {
        Ok(CodeSymbol {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            file_path: row.get(3)?,
            line_start: row.get::<_, Option<usize>>(4)?.unwrap_or(0),
            line_end: row.get(5)?,
            signature: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Export all edges as (from_id, to_id, edge_type, properties_json).
pub fn export_edges(conn: &Connection) -> rusqlite::Result<Vec<(String, String, String, String)>> {
    let mut stmt = conn.prepare("SELECT from_id, to_id, edge_type, properties FROM edge")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    rows.collect()
}

/// Count total code files in the database.
pub fn count_files(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT count(*) FROM code_file", [], |r| r.get(0))
}

/// Count total code symbols in the database.
pub fn count_symbols(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT count(*) FROM code_symbol", [], |r| r.get(0))
}

/// Insert an edge into the SQLite edge table (persisted, unlike in-memory GraphStore).
pub fn store_edge(conn: &Connection, from_id: &str, to_id: &str, edge_type: &str, properties: &str) -> rusqlite::Result<()> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
        params![id, from_id, to_id, edge_type, properties],
    )?;
    Ok(())
}

/// Stop words filtered out before word-overlap comparison in semantic dedup.
/// These inflate overlap scores for unrelated memories and should be excluded.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
    "to", "in", "for", "of", "and", "or", "with", "on", "at", "by",
    "from", "that", "this", "it", "as", "not", "but", "do", "has",
    "have", "had", "will", "would", "can", "could", "should", "may",
    "might", "we", "i", "you", "they", "he", "she", "its", "our",
    "their", "my", "your",
];

/// Extract meaningful words from text: lowercase, split on non-alphanumeric,
/// filter out stop words and single-character tokens.
fn meaningful_words(text: &str) -> HashSet<String> {
    let stop: HashSet<&str> = STOP_WORDS.iter().copied().collect();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !stop.contains(w))
        .map(String::from)
        .collect()
}

/// Find and merge near-duplicate memories using word overlap on title AND content.
/// Only deduplicates memories of the same type and project.
/// Computes title overlap and content overlap separately, then takes the max of
/// (weighted average, title score, content score) — so a strong match in either
/// title or content alone is sufficient to flag a duplicate.
/// Stop words are filtered before comparison so only meaningful words count.
/// Threshold: 0.6 combined score.
/// Returns number of duplicates merged (marked as superseded).
pub fn semantic_dedup(conn: &Connection) -> rusqlite::Result<usize> {
    // Get all active memory IDs with titles, types, projects, AND content
    let mut stmt = conn.prepare(
        "SELECT id, title, memory_type, COALESCE(project, ''), content FROM memory WHERE status = 'active' ORDER BY confidence DESC, created_at DESC"
    )?;
    let memories: Vec<(String, String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut merged = 0usize;
    let mut to_delete: HashSet<String> = HashSet::new();
    // Track which survivor supersedes each deleted memory (for causal chain edges)
    let mut superseded_by: Vec<(String, String)> = Vec::new(); // (superseded_id, survivor_id)

    for i in 0..memories.len() {
        let (ref id_a, ref title_a, ref type_a, ref project_a, ref content_a) = memories[i];
        if to_delete.contains(id_a) {
            continue;
        }

        let title_words_a = meaningful_words(title_a);
        let content_words_a = meaningful_words(content_a);

        for (id_b, title_b, type_b, project_b, content_b) in memories.iter().skip(i + 1) {
            if to_delete.contains(id_b) {
                continue;
            }
            if type_a != type_b {
                continue; // only dedup same type
            }
            if project_a != project_b {
                continue; // only dedup within same project (Codex fix: cross-project safety)
            }

            let title_words_b = meaningful_words(title_b);
            let content_words_b = meaningful_words(content_b);

            // Title overlap
            let title_intersection = title_words_a.intersection(&title_words_b).count() as f64;
            let title_max = title_words_a.len().max(title_words_b.len()) as f64;
            let title_score = if title_max > 0.0 { title_intersection / title_max } else { 0.0 };

            // Content overlap
            let content_intersection = content_words_a.intersection(&content_words_b).count() as f64;
            let content_max = content_words_a.len().max(content_words_b.len()) as f64;
            let content_score = if content_max > 0.0 { content_intersection / content_max } else { 0.0 };

            // Combined: weighted average (title 0.4, content 0.6) OR max of either score.
            // Using max ensures a strong match in either title or content is sufficient.
            let weighted = title_score * 0.4 + content_score * 0.6;
            let combined = weighted.max(title_score).max(content_score);

            if combined > 0.6 {
                // Mark the later one (id_b) for deletion
                // ORDER BY confidence DESC ensures we keep the higher-confidence memory (Codex fix: deterministic survivor)
                to_delete.insert(id_b.clone());
                superseded_by.push((id_b.clone(), id_a.clone()));
                merged += 1;
            }
        }
    }

    for id in &to_delete {
        conn.execute(
            "UPDATE memory SET status = 'superseded' WHERE id = ?1",
            params![id],
        )?;
    }

    // Create "supersedes" edges for causal chain tracking
    for (superseded_id, survivor_id) in &superseded_by {
        let _ = store_edge(conn, survivor_id, superseded_id, "supersedes", "{}");
    }

    Ok(merged)
}

/// Link memories that share 2+ tags with "related_to" edges.
/// Returns the number of edges created.
pub fn link_related_memories(conn: &Connection) -> rusqlite::Result<usize> {
    // Query all active memories with their tags
    let mut stmt = conn.prepare(
        "SELECT id, tags FROM memory WHERE status = 'active'"
    )?;
    let memories: Vec<(String, Vec<String>)> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let tags_json: String = row.get(1)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok((id, tags))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut created = 0usize;

    for i in 0..memories.len() {
        let (ref id_a, ref tags_a) = memories[i];
        if tags_a.is_empty() {
            continue;
        }

        for (id_b, tags_b) in memories.iter().skip(i + 1) {
            if tags_b.is_empty() {
                continue;
            }

            // Count shared tags
            let shared = tags_a.iter().filter(|t| tags_b.contains(t)).count();
            if shared >= 2 {
                // Check if edge already exists
                let exists: bool = conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'related_to'",
                        params![id_a, id_b],
                        |row| row.get(0),
                    )
                    .unwrap_or(false);

                if !exists {
                    let props = serde_json::json!({"shared_tags": shared}).to_string();
                    store_edge(conn, id_a, id_b, "related_to", &props)?;
                    created += 1;
                }
            }
        }
    }

    Ok(created)
}

/// Find memories with high access_count (>= 5) for reconsolidation.
/// These heavily-accessed memories are validated by usage and deserve a confidence boost.
pub fn find_reconsolidation_candidates(conn: &Connection) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, title, content, confidence, status, project, tags,
                created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count
         FROM memory WHERE status = 'active' AND access_count >= 5
         ORDER BY access_count DESC LIMIT 5"
    )?;
    let rows = stmt.query_map([], |row| {
        let mt_str: String = row.get(1)?;
        let memory_type = match mt_str.as_str() {
            "decision" => MemoryType::Decision,
            "lesson" => MemoryType::Lesson,
            "pattern" => MemoryType::Pattern,
            "preference" => MemoryType::Preference,
            _ => MemoryType::Decision,
        };
        let tags_json: String = row.get(7)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        Ok(Memory {
            id: row.get(0)?,
            memory_type,
            title: row.get(2)?,
            content: row.get(3)?,
            confidence: row.get(4)?,
            status: status_from_str(&row.get::<_, String>(5)?),
            project: row.get(6)?,
            tags,
            embedding: None,
            created_at: row.get(8)?,
            accessed_at: row.get(9)?,
            valence: row.get(10)?,
            intensity: row.get(11)?,
            hlc_timestamp: row.get(12)?,
            node_id: row.get(13)?,
            session_id: row.get::<_, String>(14).unwrap_or_default(),
            access_count: row.get::<_, i64>(15).unwrap_or(0) as u64,
        })
    })?;
    rows.collect()
}

/// Promote recurring lessons to patterns (episodic -> semantic consolidation).
///
/// If 3+ active lessons share >50% word overlap in titles AND same project,
/// create a Pattern memory with boosted confidence and supersede the individual
/// lessons. This is the neuroscience-inspired consolidation where specific
/// events (episodic) become general knowledge (semantic).
pub fn promote_recurring_lessons(conn: &Connection) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, title, content, confidence, project FROM memory
         WHERE memory_type = 'lesson' AND status = 'active'
         ORDER BY confidence DESC"
    )?;

    let lessons: Vec<(String, String, String, f64, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if lessons.len() < 3 {
        return Ok(0);
    }

    let mut promoted = 0usize;
    let mut processed: HashSet<String> = HashSet::new();

    for i in 0..lessons.len() {
        let (ref id_a, ref title_a, ref content_a, _conf_a, ref project_a) = lessons[i];
        if processed.contains(id_a) { continue; }

        let words_a: HashSet<String> = title_a.to_lowercase()
            .split_whitespace().map(String::from).collect();

        let mut cluster: Vec<usize> = vec![i];

        for (j, (ref id_b, ref title_b, _, _, ref project_b)) in lessons.iter().enumerate().skip(i + 1) {
            if processed.contains(id_b) { continue; }
            if project_a != project_b { continue; }

            let words_b: HashSet<String> = title_b.to_lowercase()
                .split_whitespace().map(String::from).collect();
            let intersection = words_a.intersection(&words_b).count() as f64;
            let max_len = words_a.len().max(words_b.len()) as f64;

            if max_len > 0.0 && (intersection / max_len) > 0.5 {
                cluster.push(j);
            }
        }

        if cluster.len() >= 3 {
            // Promote: create a Pattern from the cluster
            let best_conf = cluster.iter()
                .map(|&idx| lessons[idx].3)
                .fold(0.0f64, f64::max);
            let boosted = (best_conf + 0.1).min(1.0);

            let mut pattern = Memory::new(
                MemoryType::Pattern,
                title_a.clone(),
                format!("Promoted from {} recurring lessons: {}", cluster.len(), content_a),
            )
            .with_confidence(boosted);

            if let Some(ref p) = project_a {
                pattern.project = Some(p.clone());
            }

            let _ = remember(conn, &pattern);

            // Supersede the individual lessons
            for &idx in &cluster {
                let id = &lessons[idx].0;
                conn.execute(
                    "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                    params![id],
                )?;
                processed.insert(id.clone());
            }

            promoted += 1;
        }
    }

    Ok(promoted)
}

/// Merge memories with very high embedding similarity (>0.9 cosine).
/// This catches duplicates that lexical overlap misses.
/// Returns number of memories merged (marked as superseded).
///
/// Uses KNN search per active memory to find near-duplicates in embedding space.
/// cosine_distance < 0.1 means similarity > 0.9.
pub fn embedding_merge(conn: &Connection) -> rusqlite::Result<usize> {
    // Get all active memory IDs that have embeddings
    let mut stmt = conn.prepare(
        "SELECT m.id, m.memory_type, m.confidence FROM memory m
         JOIN memory_vec v ON v.id = m.id
         WHERE m.status = 'active'
         ORDER BY m.confidence DESC, m.created_at DESC"
    )?;
    let memories: Vec<(String, String, f64)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut merged = 0usize;
    let mut already_superseded: HashSet<String> = HashSet::new();

    for (id, mem_type, _confidence) in &memories {
        if already_superseded.contains(id) {
            continue;
        }

        // Retrieve embedding for this memory
        let emb_result: rusqlite::Result<Vec<u8>> = conn.query_row(
            "SELECT embedding FROM memory_vec WHERE id = ?1",
            params![id],
            |row| row.get(0),
        );
        let emb_bytes = match emb_result {
            Ok(b) => b,
            Err(_) => continue,
        };

        // KNN search for similar embeddings (search for more than we need to filter)
        let mut knn_stmt = conn.prepare(
            "SELECT v.id, v.distance FROM memory_vec v
             WHERE v.embedding MATCH ?1 AND k = 10"
        )?;
        let neighbors: Vec<(String, f64)> = knn_stmt
            .query_map(params![emb_bytes], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (neighbor_id, distance) in &neighbors {
            if neighbor_id == id {
                continue; // skip self
            }
            if already_superseded.contains(neighbor_id) {
                continue;
            }
            if *distance >= 0.1 {
                continue; // cosine distance >= 0.1 means similarity < 0.9
            }

            // Check that neighbor is same type and active
            let neighbor_info: Option<(String, String)> = conn.query_row(
                "SELECT memory_type, status FROM memory WHERE id = ?1",
                params![neighbor_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;

            match neighbor_info {
                Some((ref n_type, ref n_status)) if n_type == mem_type && n_status == "active" => {
                    // Mark the neighbor as superseded (current memory has higher confidence
                    // due to ORDER BY confidence DESC)
                    conn.execute(
                        "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                        params![neighbor_id],
                    )?;
                    // Create supersedes edge
                    let _ = store_edge(conn, id, neighbor_id, "supersedes", "{}");
                    already_superseded.insert(neighbor_id.clone());
                    merged += 1;
                }
                _ => continue,
            }
        }
    }

    Ok(merged)
}

/// Strengthen edges that connect frequently-accessed memories.
/// Finds edges between memories that were both accessed in the last 24 hours
/// and increments a "strength" property (capped at 1.0).
/// Returns the number of edges strengthened.
pub fn strengthen_active_edges(conn: &Connection) -> rusqlite::Result<usize> {
    // Find edges between recently-accessed active memories
    let mut stmt = conn.prepare(
        "SELECT e.id, e.properties FROM edge e
         JOIN memory m1 ON e.from_id = m1.id
         JOIN memory m2 ON e.to_id = m2.id
         WHERE m1.accessed_at > datetime('now', '-24 hours')
           AND m2.accessed_at > datetime('now', '-24 hours')
           AND m1.status = 'active'
           AND m2.status = 'active'"
    )?;

    let edges: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut strengthened = 0usize;

    for (edge_id, properties) in &edges {
        let mut props: serde_json::Value =
            serde_json::from_str(properties).unwrap_or(serde_json::json!({}));
        let current = props.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let new_strength = (current + 0.1_f64).min(1.0);
        props["strength"] = serde_json::json!(new_strength);

        conn.execute(
            "UPDATE edge SET properties = ?1 WHERE id = ?2",
            params![props.to_string(), edge_id],
        )?;
        strengthened += 1;
    }

    Ok(strengthened)
}

/// Detect contradictory memories: same tags but opposite valence.
/// Creates diagnostic warnings for the agent to review.
/// A contradiction is when two active memories share 2+ tags,
/// one has valence='positive' and the other 'negative',
/// and both have intensity > 0.5 (strong signals, not weak noise).
/// Returns number of contradiction pairs found.
pub fn detect_contradictions(conn: &Connection) -> rusqlite::Result<usize> {
    use crate::db::diagnostics::{store_diagnostic, Diagnostic};

    // Query all active memories with tags, valence, and intensity
    let mut stmt = conn.prepare(
        "SELECT id, title, tags, valence, intensity FROM memory
         WHERE status = 'active' AND valence IN ('positive', 'negative') AND intensity > 0.5"
    )?;

    let memories: Vec<(String, String, Vec<String>, String, f64)> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let tags_json: String = row.get(2)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let valence: String = row.get(3)?;
            let intensity: f64 = row.get(4)?;
            Ok((id, title, tags, valence, intensity))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut found = 0usize;

    for i in 0..memories.len() {
        let (ref id_a, ref title_a, ref tags_a, ref valence_a, _) = memories[i];
        if tags_a.len() < 2 {
            continue;
        }

        for (id_b, title_b, tags_b, valence_b, _) in memories.iter().skip(i + 1) {
            if tags_b.len() < 2 {
                continue;
            }

            // Must have opposite valence
            if valence_a == valence_b {
                continue;
            }

            // Count shared tags
            let shared = tags_a.iter().filter(|t| tags_b.contains(t)).count();
            if shared < 2 {
                continue;
            }

            // Check if this contradiction diagnostic already exists
            let diag_id = format!("contradiction-{}-{}", id_a, id_b);
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM diagnostic WHERE id = ?1",
                    params![diag_id],
                    |row| row.get(0),
                )
                .unwrap_or(false);

            if exists {
                continue;
            }

            // Create diagnostic warning
            let message = format!(
                "Contradictory memories detected: \"{}\" ({}) vs \"{}\" ({}). {} shared tags.",
                title_a, valence_a, title_b, valence_b, shared
            );
            let diag = Diagnostic {
                id: diag_id,
                file_path: "memory://contradictions".to_string(),
                severity: "warning".to_string(),
                message,
                source: "forge-consolidator".to_string(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(86400), // 24 hours
            };
            store_diagnostic(conn, &diag)?;
            found += 1;
        }
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use forge_core::types::{Memory, MemoryType, CodeFile, CodeSymbol};

    fn open_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
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

    #[test]
    fn test_remember_dedup_by_title() {
        let conn = open_db();
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "First version");
        remember(&conn, &m1).unwrap();

        let m2 = Memory::new(MemoryType::Decision, "Use JWT", "Updated version")
            .with_confidence(0.95);
        remember(&conn, &m2).unwrap();

        // Should still be 1 decision, not 2
        let h = health(&conn).unwrap();
        assert_eq!(h.decisions, 1, "dedup should prevent duplicate titles");

        // Content should be updated
        let results = recall_bm25(&conn, "JWT", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Updated"), "content should be updated");
        // Confidence should be bumped to the higher value
        assert!(
            (results[0].confidence - 0.95).abs() < 0.001,
            "confidence should be max of old (0.9) and new (0.95), got {}",
            results[0].confidence
        );
    }

    #[test]
    fn test_recall_project_scoped() {
        let conn = open_db();

        // Insert: 2 forge memories, 1 backend memory, 1 global (project=NULL)
        let m1 = Memory::new(MemoryType::Decision, "JWT for forge", "auth")
            .with_project("forge");
        remember(&conn, &m1).unwrap();

        let m2 = Memory::new(MemoryType::Decision, "CORS for forge", "cors")
            .with_project("forge");
        remember(&conn, &m2).unwrap();

        let m3 = Memory::new(MemoryType::Decision, "REST for backend", "api")
            .with_project("backend");
        remember(&conn, &m3).unwrap();

        let m4 = Memory::new(MemoryType::Decision, "Use conventional commits", "global rule");
        // project is None by default — global
        remember(&conn, &m4).unwrap();

        // Project-scoped: forge → 2 forge + 1 global = 3
        let results = recall_bm25_project(&conn, "forge backend global conventional JWT CORS REST commits", Some("forge"), 10).unwrap();
        let titles: Vec<&str> = results.iter().map(|r| r.title.as_str()).collect();
        assert!(titles.iter().any(|t| t.contains("JWT")), "should find forge memory JWT, got: {:?}", titles);
        assert!(titles.iter().any(|t| t.contains("CORS")), "should find forge memory CORS, got: {:?}", titles);
        assert!(titles.iter().any(|t| t.contains("conventional")), "should find global memory, got: {:?}", titles);
        assert!(!titles.iter().any(|t| t.contains("REST")), "should NOT find backend memory, got: {:?}", titles);
        assert_eq!(results.len(), 3, "forge scope should return 2 forge + 1 global = 3");

        // No project filter → all 4
        let all = recall_bm25_project(&conn, "forge backend global conventional JWT CORS REST commits", None, 10).unwrap();
        assert_eq!(all.len(), 4, "no project filter should return all 4 memories");
    }

    #[test]
    fn test_global_memory_in_all_projects() {
        let conn = open_db();

        let m = Memory::new(MemoryType::Pattern, "Always test first", "TDD everywhere");
        remember(&conn, &m).unwrap(); // project = None → global

        // Should appear in any project query
        let r1 = recall_bm25_project(&conn, "test", Some("forge"), 10).unwrap();
        assert_eq!(r1.len(), 1, "global memory should appear in forge project");
        let r2 = recall_bm25_project(&conn, "test", Some("backend"), 10).unwrap();
        assert_eq!(r2.len(), 1, "global memory should appear in backend project");
    }

    #[test]
    fn test_recall_project_empty_string_is_global() {
        let conn = open_db();

        // Memory with empty string project should also be treated as global
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at)
             VALUES ('empty-proj', 'decision', 'Empty project memory', 'content', 0.9, 'active', '', '[]', datetime('now'), datetime('now'))",
            [],
        ).unwrap();

        let results = recall_bm25_project(&conn, "empty project memory", Some("anyproject"), 10).unwrap();
        assert_eq!(results.len(), 1, "empty-string project memory should appear as global");
    }

    #[test]
    fn test_health_by_project() {
        let conn = open_db();

        let mut m1 = Memory::new(MemoryType::Decision, "Forge decision", "content");
        m1 = m1.with_project("forge");
        remember(&conn, &m1).unwrap();

        let mut m2 = Memory::new(MemoryType::Lesson, "Backend lesson", "content");
        m2 = m2.with_project("backend");
        remember(&conn, &m2).unwrap();

        let m3 = Memory::new(MemoryType::Pattern, "Global pattern", "content");
        remember(&conn, &m3).unwrap(); // no project → _global

        let result = health_by_project(&conn).unwrap();
        assert_eq!(result.get("forge").unwrap().decisions, 1);
        assert_eq!(result.get("backend").unwrap().lessons, 1);
        assert_eq!(result.get("_global").unwrap().patterns, 1);
        assert_eq!(result.len(), 3, "should have 3 projects: forge, backend, _global");
    }

    #[test]
    fn test_health_by_project_empty() {
        let conn = open_db();
        let result = health_by_project(&conn).unwrap();
        assert!(result.is_empty(), "empty db should return empty map");
    }

    #[test]
    fn test_remember_dedup_different_types_allowed() {
        let conn = open_db();
        // Same title but different types should NOT dedup
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "Decision content");
        let m2 = Memory::new(MemoryType::Lesson, "Use JWT", "Lesson content");
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        let h = health(&conn).unwrap();
        assert_eq!(h.decisions, 1);
        assert_eq!(h.lessons, 1);
    }

    #[test]
    fn test_dedup_memories() {
        let conn = open_db();
        // Insert 3 memories with same title directly (bypassing remember dedup)
        for i in 0..3 {
            conn.execute(
                "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
                 VALUES (?1, 'decision', 'Same title', 'content', ?2, 'active', '[]', datetime('now'), datetime('now'))",
                params![format!("d{}", i), 0.5 + (i as f64) * 0.1],
            ).unwrap();
        }
        assert_eq!(health(&conn).unwrap().decisions, 3);

        let deleted = dedup_memories(&conn).unwrap();
        assert_eq!(deleted, 2, "should remove 2 duplicates");
        assert_eq!(health(&conn).unwrap().decisions, 1);

        // The surviving one should be the highest confidence (d2, conf=0.7)
        let survivor: (String, f64) = conn.query_row(
            "SELECT id, confidence FROM memory WHERE status = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(survivor.0, "d2");
        assert!((survivor.1 - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_promote_recurring_lessons() {
        let conn = open_db();

        // Store 3 similar lessons (recurring theme)
        for i in 0..3 {
            let mem = Memory::new(
                MemoryType::Lesson,
                &format!("Always run tests before pushing v{}", i),
                "Learned from breaking prod",
            ).with_confidence(0.7);
            remember(&conn, &mem).unwrap();
        }

        let promoted = promote_recurring_lessons(&conn).unwrap();
        assert!(promoted > 0, "should promote recurring lesson to pattern");

        // Verify a Pattern was created
        let h = health(&conn).unwrap();
        assert!(h.patterns > 0, "should have at least one pattern");
    }

    #[test]
    fn test_no_promotion_for_unique_lessons() {
        let conn = open_db();

        // Store 3 different lessons (no recurring theme)
        let mem1 = Memory::new(MemoryType::Lesson, "Use JWT", "Auth").with_confidence(0.7);
        let mem2 = Memory::new(MemoryType::Lesson, "Write docs", "Quality").with_confidence(0.7);
        let mem3 = Memory::new(MemoryType::Lesson, "Test edge cases", "Coverage").with_confidence(0.7);
        remember(&conn, &mem1).unwrap();
        remember(&conn, &mem2).unwrap();
        remember(&conn, &mem3).unwrap();

        let promoted = promote_recurring_lessons(&conn).unwrap();
        assert_eq!(promoted, 0, "unique lessons should not be promoted");
    }

    // --- meaningful_words helper tests ---

    #[test]
    fn test_meaningful_words_filters_stop_words() {
        let words = meaningful_words("the quick brown fox is a fast animal");
        assert!(!words.contains("the"));
        assert!(!words.contains("is"));
        assert!(!words.contains("a"));
        assert!(words.contains("quick"));
        assert!(words.contains("brown"));
        assert!(words.contains("fox"));
        assert!(words.contains("fast"));
        assert!(words.contains("animal"));
    }

    #[test]
    fn test_meaningful_words_filters_single_chars() {
        let words = meaningful_words("I am a b c developer");
        // "I", "a", "b", "c" are all single chars or stop words
        assert!(!words.contains("b"));
        assert!(!words.contains("c"));
        assert!(words.contains("am"));
        assert!(words.contains("developer"));
    }

    #[test]
    fn test_meaningful_words_splits_on_punctuation() {
        let words = meaningful_words("graph-edges auto_generated: AFFECTS (extraction)");
        assert!(words.contains("graph"));
        assert!(words.contains("edges"));
        assert!(words.contains("auto"));
        assert!(words.contains("generated"));
        assert!(words.contains("affects"));
        assert!(words.contains("extraction"));
    }

    // --- semantic_dedup tests ---

    /// Helper: insert a memory directly into the DB for dedup testing, bypassing remember() dedup.
    fn insert_memory_for_dedup(conn: &Connection, id: &str, mem_type: &str, title: &str, content: &str, project: &str, confidence: f64) {
        let proj_val: Option<&str> = if project.is_empty() { None } else { Some(project) };
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, '[]', datetime('now'), datetime('now'))",
            params![id, mem_type, title, content, confidence, proj_val],
        ).unwrap();
    }

    #[test]
    fn test_semantic_dedup_identical_titles_same_content() {
        let conn = open_db();
        // Two memories with identical titles and content should dedup
        insert_memory_for_dedup(&conn, "a1", "decision", "Use SQLite for storage", "SQLite FTS5 fast recall", "", 0.9);
        insert_memory_for_dedup(&conn, "a2", "decision", "Use SQLite for storage", "SQLite FTS5 fast recall", "", 0.8);

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 1, "identical title+content should be deduped");

        // Higher confidence (a1=0.9) should survive
        let status_a1: String = conn.query_row("SELECT status FROM memory WHERE id = 'a1'", [], |r| r.get(0)).unwrap();
        let status_a2: String = conn.query_row("SELECT status FROM memory WHERE id = 'a2'", [], |r| r.get(0)).unwrap();
        assert_eq!(status_a1, "active");
        assert_eq!(status_a2, "superseded");
    }

    #[test]
    fn test_semantic_dedup_different_titles_same_content() {
        let conn = open_db();
        // Different titles but same content should now be caught via content overlap
        insert_memory_for_dedup(&conn, "b1", "lesson", "Code indexer broken", "The code indexer keeps crashing when parsing large files with many symbols", "", 0.9);
        insert_memory_for_dedup(&conn, "b2", "lesson", "Indexer crash bug", "The code indexer keeps crashing when parsing large files with many symbols", "", 0.8);

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 1, "different titles but same content should be deduped via content overlap");
    }

    #[test]
    fn test_semantic_dedup_stop_words_only_overlap_no_dedup() {
        let conn = open_db();
        // Two memories where only stop words overlap — should NOT dedup
        insert_memory_for_dedup(&conn, "c1", "decision", "Use JWT authentication", "Token based auth with RS256 signing", "", 0.9);
        insert_memory_for_dedup(&conn, "c2", "decision", "Deploy Kubernetes cluster", "Container orchestration with Helm charts", "", 0.8);

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 0, "memories with no meaningful word overlap should not be deduped");
    }

    #[test]
    fn test_semantic_dedup_different_types_no_dedup() {
        let conn = open_db();
        // Same title and content but different types — should NOT dedup
        insert_memory_for_dedup(&conn, "d1", "decision", "Use SQLite storage", "SQLite FTS5 for recall", "", 0.9);
        insert_memory_for_dedup(&conn, "d2", "lesson", "Use SQLite storage", "SQLite FTS5 for recall", "", 0.8);

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 0, "different types should never be deduped");
    }

    #[test]
    fn test_semantic_dedup_different_projects_no_dedup() {
        let conn = open_db();
        // Same title and content but different projects — should NOT dedup
        insert_memory_for_dedup(&conn, "e1", "decision", "Use SQLite storage", "SQLite FTS5 for recall", "forge", 0.9);
        insert_memory_for_dedup(&conn, "e2", "decision", "Use SQLite storage", "SQLite FTS5 for recall", "backend", 0.8);

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 0, "different projects should never be deduped");
    }

    #[test]
    fn test_semantic_dedup_known_audit_duplicates() {
        let conn = open_db();
        // The known near-duplicate pair from audit:
        // "Graph edges auto-generated: AFFECTS (extraction) and related_to (consolidation)"
        // vs "Graph edges auto-generated from memory extraction"
        // These share significant meaningful words: graph, edges, auto, generated, extraction
        insert_memory_for_dedup(
            &conn, "f1", "decision",
            "Graph edges auto-generated: AFFECTS (extraction) and related_to (consolidation)",
            "The system automatically generates graph edges during memory extraction and consolidation phases",
            "", 0.9,
        );
        insert_memory_for_dedup(
            &conn, "f2", "decision",
            "Graph edges auto-generated from memory extraction",
            "Graph edges are automatically generated from the memory extraction process",
            "", 0.8,
        );

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 1, "known audit near-duplicates should be caught by combined title+content dedup");

        // Higher confidence should survive
        let status_f1: String = conn.query_row("SELECT status FROM memory WHERE id = 'f1'", [], |r| r.get(0)).unwrap();
        assert_eq!(status_f1, "active", "higher confidence memory should survive");
    }

    #[test]
    fn test_semantic_dedup_creates_supersedes_edges() {
        let conn = open_db();
        insert_memory_for_dedup(&conn, "g1", "decision", "Use SQLite for storage", "Fast BM25 recall engine", "", 0.9);
        insert_memory_for_dedup(&conn, "g2", "decision", "Use SQLite for storage", "Fast BM25 recall engine", "", 0.8);

        semantic_dedup(&conn).unwrap();

        // Check that a supersedes edge was created
        let edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = 'g1' AND to_id = 'g2' AND edge_type = 'supersedes'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(edge_count, 1, "supersedes edge should be created from survivor to superseded");
    }

    #[test]
    fn test_semantic_dedup_near_similar_content_different_titles() {
        let conn = open_db();
        // Near-similar content (most words overlap) but different titles
        insert_memory_for_dedup(
            &conn, "h1", "lesson",
            "Code indexer keeps crashing",
            "The code indexer crashes when processing Rust files with complex generics and trait implementations",
            "", 0.9,
        );
        insert_memory_for_dedup(
            &conn, "h2", "lesson",
            "Indexer failure on complex code",
            "Code indexer crashes when processing Rust files with complex generics and trait implementations",
            "", 0.8,
        );

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 1, "near-similar content should be deduped even with different titles");
    }

    #[test]
    fn test_semantic_dedup_completely_unrelated() {
        let conn = open_db();
        // Completely unrelated memories should not be deduped
        insert_memory_for_dedup(
            &conn, "i1", "decision",
            "Use PostgreSQL for analytics",
            "Complex aggregation queries benefit from PostgreSQL columnar extensions",
            "", 0.9,
        );
        insert_memory_for_dedup(
            &conn, "i2", "decision",
            "Deploy with Docker Compose",
            "Multi-container orchestration simplifies local development environment setup",
            "", 0.8,
        );

        let merged = semantic_dedup(&conn).unwrap();
        assert_eq!(merged, 0, "completely unrelated memories should not be deduped");
    }

    // --- Sleep-cycle graph consolidation tests ---

    #[test]
    fn test_embedding_merge_high_similarity() {
        let conn = open_db();
        use crate::db::vec::store_embedding;

        // Create two memories with identical embeddings (distance = 0, similarity = 1.0)
        let m1 = Memory::new(MemoryType::Decision, "Use Postgres for data", "Postgres is great")
            .with_confidence(0.9);
        let m2 = Memory::new(MemoryType::Decision, "PostgreSQL for storage", "PG is reliable")
            .with_confidence(0.7);
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        // Store identical embeddings for both
        let emb: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01).sin()).collect();
        store_embedding(&conn, &m1.id, &emb).unwrap();
        store_embedding(&conn, &m2.id, &emb).unwrap();

        let merged = embedding_merge(&conn).unwrap();
        assert_eq!(merged, 1, "should merge 1 near-duplicate");

        // The higher-confidence one (m1, 0.9) should survive
        let m1_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", params![m1.id], |row| row.get(0),
        ).unwrap();
        let m2_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", params![m2.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(m1_status, "active", "higher confidence memory should survive");
        assert_eq!(m2_status, "superseded", "lower confidence memory should be superseded");

        // Verify supersedes edge was created
        let edge_exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'supersedes'",
            params![m1.id, m2.id],
            |row| row.get(0),
        ).unwrap();
        assert!(edge_exists, "supersedes edge should exist");
    }

    #[test]
    fn test_embedding_merge_low_similarity() {
        let conn = open_db();
        use crate::db::vec::store_embedding;

        // Create two memories with very different embeddings
        let m1 = Memory::new(MemoryType::Decision, "Use Rust for backend", "Performance")
            .with_confidence(0.9);
        let m2 = Memory::new(MemoryType::Decision, "Use React for frontend", "UI framework")
            .with_confidence(0.8);
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        // Store very different embeddings
        let emb1: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01).sin()).collect();
        let emb2: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01 + 100.0).cos()).collect();
        store_embedding(&conn, &m1.id, &emb1).unwrap();
        store_embedding(&conn, &m2.id, &emb2).unwrap();

        let merged = embedding_merge(&conn).unwrap();
        assert_eq!(merged, 0, "dissimilar memories should not be merged");

        // Both should still be active
        let m1_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", params![m1.id], |row| row.get(0),
        ).unwrap();
        let m2_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", params![m2.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(m1_status, "active");
        assert_eq!(m2_status, "active");
    }

    #[test]
    fn test_strengthen_active_edges() {
        let conn = open_db();

        // Create two memories accessed recently (now)
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "Auth tokens");
        let m2 = Memory::new(MemoryType::Decision, "Use HTTPS", "Security");
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        // Create edge between them
        store_edge(&conn, &m1.id, &m2.id, "related_to", "{}").unwrap();

        // Strengthen edges
        let strengthened = strengthen_active_edges(&conn).unwrap();
        assert_eq!(strengthened, 1, "should strengthen 1 edge");

        // Verify strength property was set
        let props_str: String = conn.query_row(
            "SELECT properties FROM edge WHERE from_id = ?1 AND to_id = ?2",
            params![m1.id, m2.id],
            |row| row.get(0),
        ).unwrap();
        let props: serde_json::Value = serde_json::from_str(&props_str).unwrap();
        let strength = props.get("strength").and_then(|v| v.as_f64()).unwrap();
        assert!((strength - 0.1).abs() < 0.001, "strength should be 0.1 after first increment");

        // Strengthen again — should increment to 0.2
        let strengthened2 = strengthen_active_edges(&conn).unwrap();
        assert_eq!(strengthened2, 1);
        let props_str2: String = conn.query_row(
            "SELECT properties FROM edge WHERE from_id = ?1 AND to_id = ?2",
            params![m1.id, m2.id],
            |row| row.get(0),
        ).unwrap();
        let props2: serde_json::Value = serde_json::from_str(&props_str2).unwrap();
        let strength2 = props2.get("strength").and_then(|v| v.as_f64()).unwrap();
        assert!((strength2 - 0.2).abs() < 0.001, "strength should be 0.2 after second increment");
    }

    #[test]
    fn test_detect_contradictions() {
        let conn = open_db();
        use crate::db::diagnostics;

        // Create two memories with shared tags but opposite valence and high intensity
        let m1 = Memory::new(MemoryType::Decision, "Microservices are great", "They scale well")
            .with_tags(vec!["architecture".into(), "scaling".into(), "design".into()])
            .with_valence("positive", 0.8);
        let m2 = Memory::new(MemoryType::Decision, "Microservices cause problems", "Too complex")
            .with_tags(vec!["architecture".into(), "scaling".into(), "complexity".into()])
            .with_valence("negative", 0.9);
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        let found = detect_contradictions(&conn).unwrap();
        assert_eq!(found, 1, "should detect 1 contradiction");

        // Verify diagnostic was created
        let diags = diagnostics::get_all_active_diagnostics(&conn).unwrap();
        let contradiction_diags: Vec<_> = diags.iter()
            .filter(|d| d.source == "forge-consolidator" && d.severity == "warning")
            .collect();
        assert_eq!(contradiction_diags.len(), 1, "should have 1 contradiction diagnostic");
        assert!(contradiction_diags[0].message.contains("Microservices are great"));
        assert!(contradiction_diags[0].message.contains("Microservices cause problems"));

        // Running again should not create duplicate diagnostics
        let found2 = detect_contradictions(&conn).unwrap();
        assert_eq!(found2, 0, "should not re-detect same contradiction");
    }

    #[test]
    fn test_detect_contradictions_ignores_low_intensity() {
        let conn = open_db();
        use crate::db::diagnostics;

        // Create two memories with shared tags, opposite valence, but LOW intensity (< 0.5)
        let m1 = Memory::new(MemoryType::Decision, "REST might be ok", "Acceptable")
            .with_tags(vec!["api".into(), "design".into()])
            .with_valence("positive", 0.3);
        let m2 = Memory::new(MemoryType::Decision, "REST has issues", "Some downsides")
            .with_tags(vec!["api".into(), "design".into()])
            .with_valence("negative", 0.2);
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        let found = detect_contradictions(&conn).unwrap();
        assert_eq!(found, 0, "should not detect weak contradictions (intensity < 0.5)");

        // Verify no diagnostics were created
        let diags = diagnostics::get_all_active_diagnostics(&conn).unwrap();
        let contradiction_diags: Vec<_> = diags.iter()
            .filter(|d| d.source == "forge-consolidator")
            .collect();
        assert_eq!(contradiction_diags.len(), 0, "no contradiction diagnostics for weak signals");
    }
}
