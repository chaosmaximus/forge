use forge_core::types::{CodeFile, CodeSymbol, Memory, MemoryStatus, MemoryType};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

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
        MemoryType::Protocol => "protocol",
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
    let alternatives_json =
        serde_json::to_string(&memory.alternatives).unwrap_or_else(|_| "[]".to_string());
    let participants_json =
        serde_json::to_string(&memory.participants).unwrap_or_else(|_| "[]".to_string());

    let org_id = memory.organization_id.as_deref().unwrap_or("default");

    // Check for existing memory with same title, type, project, AND organization.
    // Including project+org in the dedup key prevents cross-project and cross-org
    // merging where a decision from one tenant silently overwrites another's.
    let existing_id: Option<String> = conn.query_row(
        "SELECT id FROM memory WHERE title = ?1 AND memory_type = ?2 AND COALESCE(project, '') = COALESCE(?3, '') AND COALESCE(organization_id, 'default') = ?4 AND status = 'active'",
        params![memory.title, mt, memory.project, org_id],
        |row| row.get(0),
    ).optional()?;

    if let Some(existing_id) = existing_id {
        // Update existing — bump confidence if higher, update content + alternatives/participants
        conn.execute(
            "UPDATE memory SET content = ?1, confidence = MAX(confidence, ?2), accessed_at = ?3,
             hlc_timestamp = ?4, node_id = ?5, alternatives = ?6, participants = ?7
             WHERE id = ?8",
            params![
                memory.content,
                memory.confidence,
                memory.accessed_at,
                memory.hlc_timestamp,
                memory.node_id,
                alternatives_json,
                participants_json,
                existing_id
            ],
        )?;
    } else {
        // Insert new
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count, alternatives, participants, organization_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                memory.id, mt, memory.title, memory.content,
                memory.confidence, status,
                memory.project, tags_json,
                memory.created_at, memory.accessed_at,
                memory.valence, memory.intensity,
                memory.hlc_timestamp, memory.node_id,
                memory.session_id, memory.access_count as i64,
                alternatives_json, participants_json,
                org_id,
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
    let alternatives_json =
        serde_json::to_string(&memory.alternatives).unwrap_or_else(|_| "[]".to_string());
    let participants_json =
        serde_json::to_string(&memory.participants).unwrap_or_else(|_| "[]".to_string());
    let org_id = memory.organization_id.as_deref().unwrap_or("default");

    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count, alternatives, participants, organization_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            memory.id, mt, memory.title, memory.content,
            memory.confidence, status,
            memory.project, tags_json,
            memory.created_at, memory.accessed_at,
            memory.valence, memory.intensity,
            memory.hlc_timestamp, memory.node_id,
            memory.session_id, memory.access_count as i64,
            alternatives_json, participants_json,
            org_id,
        ],
    )?;
    Ok(())
}

/// Boost activation level for a memory (capped at 1.0).
/// Used to track which memories are actively being used.
/// Activation decays over time in the consolidator.
/// Boost activation level for a memory. Best-effort: silently no-ops on read-only connections.
/// This is called from CompileContext/Recall which run on read-only connections in the
/// actor architecture. The boost is an optimization hint, not critical.
pub fn boost_activation(conn: &Connection, memory_id: &str, amount: f64) -> rusqlite::Result<()> {
    match conn.execute(
        "UPDATE memory SET activation_level = MIN(1.0, COALESCE(activation_level, 0.0) + ?1) WHERE id = ?2",
        params![amount, memory_id],
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("readonly") => Ok(()), // read-only conn — skip silently
        Err(e) => Err(e),
    }
}

/// Decay all activation levels by a multiplicative factor.
/// Memories with activation_level <= threshold are zeroed out to avoid float dust.
/// Returns the number of rows updated.
pub fn decay_activation_levels(conn: &Connection) -> rusqlite::Result<usize> {
    let updated = conn.execute(
        "UPDATE memory SET activation_level = activation_level * 0.95 WHERE activation_level > 0.01",
        [],
    )?;
    // Zero out dust
    conn.execute(
        "UPDATE memory SET activation_level = 0.0 WHERE activation_level > 0.0 AND activation_level <= 0.01",
        [],
    )?;
    Ok(updated)
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
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if cleaned.is_empty() {
                return None; // drop pure-punctuation tokens like "*"
            }
            // FTS5 escape: double any internal double-quotes (shouldn't exist after cleaning, but defensive)
            let escaped = cleaned.replace('"', "\"\"");
            Some(format!("\"{escaped}\""))
        })
        .collect();

    if terms.is_empty() {
        return String::new();
    }

    terms.join(" OR ")
}

/// Full-text search using FTS5 BM25 scoring. Returns active memories ranked by relevance.
/// When `org_id` is `Some("X")`, only returns memories from that organization.
/// When `org_id` is `None`, returns all active memories (backward compat).
pub fn recall_bm25(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<BM25Result>> {
    recall_bm25_org(conn, query, limit, None)
}

/// Full-text search using FTS5 BM25 scoring with optional organization filter.
pub fn recall_bm25_org(
    conn: &Connection,
    query: &str,
    limit: usize,
    org_id: Option<&str>,
) -> rusqlite::Result<Vec<BM25Result>> {
    // NEW-2: Sanitize the query to prevent FTS5 operator injection
    let safe_query = sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new()); // No valid search terms after sanitization
    }

    match org_id {
        Some(org) => {
            let sql = "
                SELECT m.id, m.title, m.content, bm25(memory_fts) AS score, m.memory_type, m.confidence, m.valence, m.intensity
                FROM memory_fts
                JOIN memory m ON memory_fts.rowid = m.rowid
                WHERE memory_fts MATCH ?1
                  AND m.status = 'active'
                  AND COALESCE(m.organization_id, 'default') = ?2
                ORDER BY score
                LIMIT ?3
            ";
            let mut stmt = conn.prepare(sql)?;
            let results = stmt.query_map(params![safe_query, org, limit as i64], |row| {
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
        None => {
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
    }
}

/// Full-text search using FTS5 BM25 scoring with optional project and organization filter.
///
/// When `project` is `Some("X")`, returns only memories where `project = 'X'`
/// OR `project IS NULL` OR `project = ''` (global memories visible in every project).
/// When `project` is `None`, returns all active memories (existing behavior).
/// When `org_id` is `Some("X")`, additionally filters to that organization.
pub fn recall_bm25_project(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<BM25Result>> {
    recall_bm25_project_org(conn, query, project, limit, None)
}

/// Full-text search with project + organization filtering.
pub fn recall_bm25_project_org(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: usize,
    org_id: Option<&str>,
) -> rusqlite::Result<Vec<BM25Result>> {
    let safe_query = sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new());
    }

    // Build the org filter clause
    let org_filter = if org_id.is_some() {
        " AND COALESCE(m.organization_id, 'default') = ?4"
    } else {
        ""
    };

    match project {
        Some(proj) => {
            let sql = format!(
                "SELECT m.id, m.title, m.content, bm25(memory_fts) AS score, m.memory_type, m.confidence, m.valence, m.intensity
                 FROM memory_fts
                 JOIN memory m ON memory_fts.rowid = m.rowid
                 WHERE memory_fts MATCH ?1
                   AND m.status = 'active'
                   AND (m.project = ?2 OR m.project IS NULL OR m.project = ''){org_filter}
                 ORDER BY score
                 LIMIT ?3",
            );
            let mut stmt = conn.prepare(&sql)?;
            let mapper = |row: &rusqlite::Row| {
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
            };
            if let Some(org) = org_id {
                stmt.query_map(params![safe_query, proj, limit as i64, org], mapper)?
                    .collect()
            } else {
                stmt.query_map(params![safe_query, proj, limit as i64], mapper)?
                    .collect()
            }
        }
        None => recall_bm25_org(conn, query, limit, org_id),
    }
}

/// Soft-delete a memory by setting status to 'superseded'.
/// When `org_id` is `Some("X")`, additionally checks the memory belongs to that org.
/// Returns true if a row was updated (was active before).
pub fn forget(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    forget_org(conn, id, None)
}

/// Soft-delete a memory with optional organization scoping.
pub fn forget_org(conn: &Connection, id: &str, org_id: Option<&str>) -> rusqlite::Result<bool> {
    let rows_changed = match org_id {
        Some(org) => conn.execute(
            "UPDATE memory SET status = 'superseded' WHERE id = ?1 AND status = 'active' AND COALESCE(organization_id, 'default') = ?2",
            params![id, org],
        )?,
        None => conn.execute(
            "UPDATE memory SET status = 'superseded' WHERE id = ?1 AND status = 'active'",
            params![id],
        )?,
    };
    Ok(rows_changed > 0)
}

/// Health counts grouped by project.
/// When `org_id` is `Some("X")`, only counts memories from that organization.
pub fn health_by_project(
    conn: &Connection,
) -> rusqlite::Result<std::collections::HashMap<String, HealthCounts>> {
    health_by_project_org(conn, None)
}

/// Health counts grouped by project with optional organization filter.
pub fn health_by_project_org(
    conn: &Connection,
    org_id: Option<&str>,
) -> rusqlite::Result<std::collections::HashMap<String, HealthCounts>> {
    let (sql, use_org) = match org_id {
        Some(_) => (
            "SELECT COALESCE(NULLIF(project, ''), '_global') as proj, memory_type, count(*) as cnt
             FROM memory WHERE status = 'active' AND COALESCE(organization_id, 'default') = ?1 GROUP BY proj, memory_type",
            true,
        ),
        None => (
            "SELECT COALESCE(NULLIF(project, ''), '_global') as proj, memory_type, count(*) as cnt
             FROM memory WHERE status = 'active' GROUP BY proj, memory_type",
            false,
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let mut projects: std::collections::HashMap<String, HealthCounts> =
        std::collections::HashMap::new();
    let mapper = |row: &rusqlite::Row| -> rusqlite::Result<(String, String, usize)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    };
    let collected: Vec<(String, String, usize)> = if use_org {
        stmt.query_map(params![org_id.unwrap()], mapper)?
            .flatten()
            .collect()
    } else {
        stmt.query_map([], mapper)?.flatten().collect()
    };

    for (proj, mtype, count) in collected {
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
    let total_edges: usize = conn
        .query_row("SELECT count(*) FROM edge", [], |r| r.get(0))
        .unwrap_or(0);
    for counts in projects.values_mut() {
        counts.edges = total_edges;
    }

    Ok(projects)
}

/// Count active memories per type and total edges.
/// When `org_id` is `Some("X")`, only counts memories from that organization.
pub fn health(conn: &Connection) -> rusqlite::Result<HealthCounts> {
    health_org(conn, None)
}

/// Count active memories per type with optional organization filter.
pub fn health_org(conn: &Connection, org_id: Option<&str>) -> rusqlite::Result<HealthCounts> {
    let count_type = |type_name: &str| -> rusqlite::Result<usize> {
        match org_id {
            Some(org) => conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = ?1 AND status = 'active' AND COALESCE(organization_id, 'default') = ?2",
                params![type_name, org],
                |row| row.get::<_, i64>(0),
            ),
            None => conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = ?1 AND status = 'active'",
                params![type_name],
                |row| row.get::<_, i64>(0),
            ),
        }
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
pub fn decay_memories(conn: &Connection, limit: usize) -> rusqlite::Result<(usize, usize)> {
    let mut stmt = conn.prepare(
        "SELECT id, confidence, accessed_at FROM memory WHERE status = 'active' LIMIT ?1",
    )?;

    let rows: Vec<(String, f64, String)> = stmt
        .query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

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
        if let Err(e) = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now'),
             access_count = MIN(access_count + 1, 1000)
             WHERE id = ?1
             AND (accessed_at < datetime('now', '-60 seconds') OR access_count = 0)",
            params![id],
        ) {
            eprintln!("[ops] failed to touch memory {id}: {e}");
        }
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

/// List all code files currently in the index.
pub fn list_code_files(conn: &Connection) -> Vec<CodeFile> {
    let mut stmt = match conn
        .prepare("SELECT id, path, language, project, hash, indexed_at FROM code_file LIMIT 10000")
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |row| {
        Ok(CodeFile {
            id: row.get(0)?,
            path: row.get(1)?,
            language: row.get(2)?,
            project: row.get(3)?,
            hash: row.get(4)?,
            indexed_at: row.get(5)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(|r| r.ok()).collect()
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

    conn.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _current_paths (path TEXT PRIMARY KEY)",
        [],
    )?;
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
/// When `org_id` is `Some("X")`, only exports memories from that organization.
pub fn export_memories(conn: &Connection) -> rusqlite::Result<Vec<Memory>> {
    export_memories_org(conn, None)
}

/// Export active memories with optional organization filter.
pub fn export_memories_org(
    conn: &Connection,
    org_id: Option<&str>,
) -> rusqlite::Result<Vec<Memory>> {
    let (sql, use_org) = match org_id {
        Some(_) => (
            "SELECT id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count, COALESCE(activation_level, 0.0), organization_id
             FROM memory WHERE status = 'active' AND COALESCE(organization_id, 'default') = ?1 ORDER BY created_at DESC",
            true,
        ),
        None => (
            "SELECT id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count, COALESCE(activation_level, 0.0), organization_id
             FROM memory WHERE status = 'active' ORDER BY created_at DESC",
            false,
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let mapper = |row: &rusqlite::Row| -> rusqlite::Result<Memory> {
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
            activation_level: row.get::<_, f64>(16).unwrap_or(0.0),
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: row.get::<_, Option<String>>(17)?,
        })
    };
    if use_org {
        stmt.query_map(params![org_id.unwrap()], mapper)?.collect()
    } else {
        stmt.query_map([], mapper)?.collect()
    }
}

/// Export all code files.
pub fn export_files(conn: &Connection) -> rusqlite::Result<Vec<CodeFile>> {
    let mut stmt =
        conn.prepare("SELECT id, path, language, project, hash, indexed_at FROM code_file")?;
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
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, file_path, line_start, line_end, signature FROM code_symbol",
    )?;
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

/// List all code symbols (for call edge detection when symbols are cached and not in memory).
pub fn list_symbols(conn: &Connection) -> rusqlite::Result<Vec<CodeSymbol>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, file_path, line_start, line_end, signature FROM code_symbol",
    )?;
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

/// Insert an edge into the SQLite edge table (persisted, unlike in-memory GraphStore).
pub fn store_edge(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    edge_type: &str,
    properties: &str,
) -> rusqlite::Result<()> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
        params![id, from_id, to_id, edge_type, properties],
    )?;
    Ok(())
}

// ── Observability: metrics tracking ──

/// Aggregated stats for a time period.
#[derive(Debug, Clone, Default)]
pub struct StatsData {
    pub period_hours: u64,
    pub extractions: usize,
    pub extraction_errors: usize,
    pub tokens_in: usize,
    pub tokens_out: usize,
    pub total_cost_usd: f64,
    pub avg_latency_ms: usize,
    pub memories_created: usize,
}

/// Estimate cost in USD for an extraction call.
/// Prices as of 2026 (approximate per million tokens).
pub fn estimate_cost(model: &str, tokens_in: usize, tokens_out: usize) -> f64 {
    let (price_in, price_out) = match model {
        m if m.contains("haiku") => (0.25, 1.25),
        m if m.contains("sonnet") => (3.0, 15.0),
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("gemini") => (0.0, 0.0), // free tier
        m if m.contains("gemma") => (0.0, 0.0),  // local/free
        _ => (0.0, 0.0),                         // ollama = free
    };
    (tokens_in as f64 * price_in + tokens_out as f64 * price_out) / 1_000_000.0
}

/// Store a metric entry (extraction, embedding, etc.).
pub fn store_metric(
    conn: &Connection,
    metric_type: &str,
    model: &str,
    tokens_in: usize,
    tokens_out: usize,
    latency_ms: u64,
    status: &str,
) -> rusqlite::Result<()> {
    let cost = estimate_cost(model, tokens_in, tokens_out);
    conn.execute(
        "INSERT INTO metrics (id, metric_type, timestamp, model, tokens_in, tokens_out, latency_ms, cost_usd, status)
         VALUES (?1, ?2, datetime('now'), ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            format!("metric-{}", ulid::Ulid::new()),
            metric_type,
            model,
            tokens_in as i64,
            tokens_out as i64,
            latency_ms as i64,
            cost,
            status,
        ],
    )?;
    Ok(())
}

/// Query aggregated stats for a time period.
pub fn query_stats(conn: &Connection, hours: u64) -> rusqlite::Result<StatsData> {
    let (total_extractions, total_tokens_in, total_tokens_out, total_cost, avg_latency): (
        i64,
        i64,
        i64,
        f64,
        f64,
    ) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0),
                COALESCE(SUM(cost_usd), 0.0), COALESCE(AVG(latency_ms), 0.0)
         FROM metrics WHERE metric_type = 'extraction'
           AND timestamp > datetime('now', ?1)",
        params![format!("-{} hours", hours)],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;

    let errors: i64 = conn.query_row(
        "SELECT COUNT(*) FROM metrics WHERE metric_type = 'extraction' AND status != 'ok'
           AND timestamp > datetime('now', ?1)",
        params![format!("-{} hours", hours)],
        |row| row.get(0),
    )?;

    let memory_growth: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE created_at > datetime('now', ?1)",
        params![format!("-{} hours", hours)],
        |row| row.get(0),
    )?;

    Ok(StatsData {
        period_hours: hours,
        extractions: total_extractions as usize,
        extraction_errors: errors as usize,
        tokens_in: total_tokens_in as usize,
        tokens_out: total_tokens_out as usize,
        total_cost_usd: total_cost,
        avg_latency_ms: avg_latency as usize,
        memories_created: memory_growth as usize,
    })
}

/// Stop words filtered out before word-overlap comparison in semantic dedup.
/// These inflate overlap scores for unrelated memories and should be excluded.
use crate::common::STOP_WORDS;

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

/// Public wrapper for meaningful_words (used by healing worker).
pub fn meaningful_words_pub(text: &str) -> HashSet<String> {
    meaningful_words(text)
}

/// Find active memories with similar titles using FTS5 full-text search.
///
/// Returns `(id, title)` pairs for active memories of the given `memory_type`
/// whose FTS5 content matches the meaningful words from `title`. This is a
/// fast pre-filter for the extractor's near-duplicate check.
pub fn find_similar_by_title(
    conn: &Connection,
    title: &str,
    memory_type: &str,
    limit: usize,
) -> rusqlite::Result<Vec<(String, String)>> {
    let words = meaningful_words(title);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let fts_terms: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
    let fts_query = fts_terms.join(" OR ");
    let sql = "
        SELECT m.id, m.title
        FROM memory_fts
        JOIN memory m ON memory_fts.rowid = m.rowid
        WHERE memory_fts MATCH ?1
          AND m.status = 'active'
          AND m.memory_type = ?2
        ORDER BY bm25(memory_fts)
        LIMIT ?3
    ";
    let mut stmt = conn.prepare(sql)?;
    let results = stmt.query_map(params![fts_query, memory_type, limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    results.collect()
}

/// Backfill project on memories that have project = NULL.
///
/// Phase 1: Join memory → session on session_id to inherit the session's project.
/// Phase 2: If only one distinct project exists in the DB, assign it to remaining orphans.
/// Returns (updated_count, remaining_orphan_count).
pub fn backfill_project_from_sessions(conn: &Connection) -> rusqlite::Result<(usize, usize)> {
    // Phase 1: Update orphans whose session has a known project
    let phase1 = conn.execute(
        "UPDATE memory SET project = (
            SELECT s.project FROM session s
            WHERE s.id = memory.session_id AND s.project IS NOT NULL AND s.project != ''
            LIMIT 1
        )
        WHERE (project IS NULL OR project = '')
          AND session_id IS NOT NULL
          AND EXISTS (
            SELECT 1 FROM session s
            WHERE s.id = memory.session_id AND s.project IS NOT NULL AND s.project != ''
          )",
        [],
    )?;

    // Phase 2: If only one distinct project, assign to remaining orphans
    let distinct_projects: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT project FROM memory WHERE project IS NOT NULL AND project != '' LIMIT 2"
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut phase2 = 0usize;
    if distinct_projects.len() == 1 {
        let project = &distinct_projects[0];
        phase2 = conn.execute(
            "UPDATE memory SET project = ?1 WHERE (project IS NULL OR project = '')",
            params![project],
        )?;
    }

    let remaining: usize = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE project IS NULL OR project = ''",
        [],
        |row| row.get(0),
    )?;
    Ok((phase1 + phase2, remaining))
}

/// Soft-delete memories with quality_score = 0.0 and zero access_count.
/// These are extracted garbage that was never accessed — safe to remove.
/// Returns the number of memories soft-deleted.
pub fn cleanup_garbage_memories(conn: &Connection) -> rusqlite::Result<usize> {
    let deleted = conn.execute(
        "UPDATE memory SET deleted_at = datetime('now')
         WHERE status = 'active'
         AND quality_score < 0.01
         AND access_count = 0
         AND deleted_at IS NULL
         AND created_at < datetime('now', '-7 days')",
        [],
    )?;
    Ok(deleted)
}

/// Purge faded memories older than the given number of days.
/// Deletes memories with status='faded' AND created_at < threshold.
/// Also cleans up associated edges and FTS entries (via triggers).
/// Returns number of memories purged.
pub fn purge_faded_memories(conn: &Connection, older_than_days: i64) -> rusqlite::Result<usize> {
    // Collect IDs of faded memories older than threshold
    let threshold = format!("-{older_than_days} days");
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM memory WHERE status = 'faded' AND created_at < datetime('now', ?1)",
        )?;
        let rows = stmt.query_map(params![threshold], |row| row.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    if ids.is_empty() {
        return Ok(0);
    }

    let count = ids.len();

    // Delete edges referencing these memories
    for id in &ids {
        conn.execute(
            "DELETE FROM edge WHERE from_id = ?1 OR to_id = ?1",
            params![id],
        )?;
    }

    // Delete from memory_vec (embedding virtual table)
    for id in &ids {
        let _ = conn.execute("DELETE FROM memory_vec WHERE id = ?1", params![id]);
    }

    // Delete from memory (FTS cleanup happens via triggers)
    for id in &ids {
        conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
    }

    Ok(count)
}

/// Remove affects edges whose target file no longer exists on disk.
/// Affects edges use `file:{path}` as to_id. If the path doesn't exist, the edge is orphaned.
///
/// M4: Only checks absolute paths or paths resolved against known project roots from code_file.
/// Relative paths without a known project root are left alone (CWD varies between contexts).
/// Returns number of edges removed.
pub fn cleanup_orphaned_affects_edges(conn: &Connection) -> rusqlite::Result<usize> {
    // Collect known project roots from code_file paths (for resolving relative affects edges)
    let project_roots: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT DISTINCT project FROM code_file WHERE project != ''")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let edges: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, to_id FROM edge WHERE edge_type = 'affects' AND to_id LIKE 'file:%'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut removed = 0usize;
    for (id, to_id) in &edges {
        let path = to_id.strip_prefix("file:").unwrap_or(to_id);
        let p = std::path::Path::new(path);

        // Only check existence for absolute paths or paths we can resolve.
        // Skip relative paths when no project roots are known — CWD may differ
        // from project root, which would falsely mark valid edges as orphaned.
        let exists = if p.is_absolute() {
            p.exists()
        } else if project_roots.is_empty() {
            true // assume exists — can't verify without project root
        } else {
            project_roots
                .iter()
                .any(|root| std::path::Path::new(root).join(path).exists())
        };

        if !exists {
            conn.execute("DELETE FROM edge WHERE id = ?1", params![id])?;
            removed += 1;
        }
    }

    Ok(removed)
}

/// Remove code_file and code_symbol entries for files that no longer exist on disk.
/// Returns (files_removed, symbols_removed).
pub fn cleanup_orphan_code_entries(conn: &Connection) -> rusqlite::Result<(usize, usize)> {
    // Query all code_file paths
    let paths: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT id, path FROM code_file")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut files_removed = 0usize;
    let mut symbols_removed = 0usize;

    for (id, path) in &paths {
        if !std::path::Path::new(path).exists() {
            // Delete symbols for this file
            let syms = conn.execute(
                "DELETE FROM code_symbol WHERE file_path = ?1",
                params![path],
            )?;
            symbols_removed += syms;

            // Delete the file entry
            conn.execute("DELETE FROM code_file WHERE id = ?1", params![id])?;
            files_removed += 1;
        }
    }

    Ok((files_removed, symbols_removed))
}

/// Normalize fragmented project names using HOME-based prefix stripping.
/// Applies the same logic as extract_project_from_path but to existing memories.
/// Returns the number of memories updated.
pub fn normalize_project_names(conn: &Connection) -> rusqlite::Result<usize> {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return Ok(0),
    };
    let home_prefix = format!("-{}-", home.trim_start_matches('/').replace('/', "-"));

    // Find all unique project names that look like they were extracted incorrectly
    // (single-word names that are path segments, not real project names)
    let mut stmt = conn.prepare(
        "SELECT DISTINCT project FROM memory
         WHERE project IS NOT NULL AND project != '' AND deleted_at IS NULL",
    )?;
    let projects: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut total_updated = 0;
    for proj in &projects {
        let normalized = if let Some(rest) = proj.strip_prefix(&home_prefix) {
            // HOME-based path: "-home-user-project" → "project"
            if rest.is_empty() {
                continue;
            }
            rest.to_string()
        } else {
            // CWD-based path: extract last path segment
            // Pattern: "mnt-colab-disk-User-project" or "-mnt-colab-disk-User-project"
            let trimmed = proj.trim_start_matches('-');
            if trimmed.contains('-')
                && (trimmed.starts_with("mnt-")
                    || trimmed.starts_with("home-")
                    || trimmed.starts_with("tmp-"))
            {
                // This looks like an encoded path — extract the last segment
                trimmed.rsplit('-').next().unwrap_or(trimmed).to_string()
            } else {
                continue; // Already a clean project name
            }
        };

        if !normalized.is_empty() && normalized != *proj {
            let updated = conn.execute(
                "UPDATE memory SET project = ?1 WHERE project = ?2 AND deleted_at IS NULL",
                rusqlite::params![normalized, proj],
            )?;
            total_updated += updated;
        }
    }
    Ok(total_updated)
}

/// Find and merge near-duplicate memories using word overlap on title AND content.
/// Only deduplicates memories of the same type and project.
/// Computes title overlap and content overlap separately, then takes the max of
/// (weighted average, title score, content score) — so a strong match in either
/// title or content alone is sufficient to flag a duplicate.
/// Stop words are filtered before comparison so only meaningful words count.
/// Threshold: 0.65 combined score.
/// Returns number of duplicates merged (marked as superseded).
pub fn semantic_dedup(conn: &Connection, limit: usize) -> rusqlite::Result<usize> {
    // Get active memory IDs with titles, types, projects, AND content — bounded to prevent O(N^2) blowup
    let mut stmt = conn.prepare(
        "SELECT id, title, memory_type, COALESCE(project, ''), content FROM memory WHERE status = 'active' ORDER BY confidence DESC, created_at DESC LIMIT ?1"
    )?;
    let memories: Vec<(String, String, String, String, String)> = stmt
        .query_map(params![limit as i64], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
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
            let title_score = if title_max > 0.0 {
                title_intersection / title_max
            } else {
                0.0
            };

            // Content overlap
            let content_intersection =
                content_words_a.intersection(&content_words_b).count() as f64;
            let content_max = content_words_a.len().max(content_words_b.len()) as f64;
            let content_score = if content_max > 0.0 {
                content_intersection / content_max
            } else {
                0.0
            };

            // Combined: weighted average (title 0.5, content 0.5) OR max of either score.
            // Using max ensures a strong match in either title or content is sufficient.
            // Threshold raised to 0.65 to avoid suppressing memories with similar titles but different content.
            let weighted = title_score * 0.5 + content_score * 0.5;
            let combined = weighted.max(title_score).max(content_score);

            if combined > 0.65 {
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
        if let Err(e) = store_edge(conn, survivor_id, superseded_id, "supersedes", "{}") {
            eprintln!("[ops] failed to store supersedes edge: {e}");
        }
    }

    Ok(merged)
}

/// Link memories that share 2+ tags with "related_to" edges.
/// Returns the number of edges created.
pub fn link_related_memories(conn: &Connection, limit: usize) -> rusqlite::Result<usize> {
    // Query active memories with their tags — bounded to prevent O(N^2) blowup
    let mut stmt = conn.prepare("SELECT id, tags FROM memory WHERE status = 'active' LIMIT ?1")?;
    let memories: Vec<(String, Vec<String>)> = stmt
        .query_map(params![limit as i64], |row| {
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
                created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count, COALESCE(activation_level, 0.0), organization_id
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
            activation_level: row.get::<_, f64>(16).unwrap_or(0.0),
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: row.get::<_, Option<String>>(17)?,
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
pub fn promote_recurring_lessons(conn: &Connection, limit: usize) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, title, content, confidence, project FROM memory
         WHERE memory_type = 'lesson' AND status = 'active'
         ORDER BY confidence DESC LIMIT ?1",
    )?;

    let lessons: Vec<(String, String, String, f64, Option<String>)> = stmt
        .query_map(params![limit as i64], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
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
        if processed.contains(id_a) {
            continue;
        }

        let words_a: HashSet<String> = title_a
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();

        let mut cluster: Vec<usize> = vec![i];

        for (j, (ref id_b, ref title_b, _, _, ref project_b)) in
            lessons.iter().enumerate().skip(i + 1)
        {
            if processed.contains(id_b) {
                continue;
            }
            if project_a != project_b {
                continue;
            }

            let words_b: HashSet<String> = title_b
                .to_lowercase()
                .split_whitespace()
                .map(String::from)
                .collect();
            let intersection = words_a.intersection(&words_b).count() as f64;
            let max_len = words_a.len().max(words_b.len()) as f64;

            if max_len > 0.0 && (intersection / max_len) > 0.5 {
                cluster.push(j);
            }
        }

        if cluster.len() >= 3 {
            // Promote: create a Pattern from the cluster
            let best_conf = cluster
                .iter()
                .map(|&idx| lessons[idx].3)
                .fold(0.0f64, f64::max);
            let boosted = (best_conf + 0.1).min(1.0);

            let mut pattern = Memory::new(
                MemoryType::Pattern,
                title_a.clone(),
                format!(
                    "Promoted from {} recurring lessons: {}",
                    cluster.len(),
                    content_a
                ),
            )
            .with_confidence(boosted);

            if let Some(ref p) = project_a {
                pattern.project = Some(p.clone());
            }

            if let Err(e) = remember(conn, &pattern) {
                eprintln!("[ops] failed to store promoted pattern '{title_a}': {e}");
            }

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
    // Get active memory IDs that have embeddings — capped at 200 to bound the N+1 pattern below.
    // NOTE: Each iteration queries memory_vec per-item (N+1). Batch-loading embeddings from
    // the sqlite-vec virtual table is not straightforward (virtual tables don't support IN clauses
    // the same way), so we cap the outer query instead to limit blast radius.
    let mut stmt = conn.prepare(
        "SELECT m.id, m.memory_type, m.confidence FROM memory m
         JOIN memory_vec v ON v.id = m.id
         WHERE m.status = 'active'
         ORDER BY m.confidence DESC, m.created_at DESC
         LIMIT 200",
    )?;
    let memories: Vec<(String, String, f64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut merged = 0usize;
    let mut already_superseded: HashSet<String> = HashSet::new();

    for (id, mem_type, _confidence) in &memories {
        if already_superseded.contains(id) {
            continue;
        }

        // Retrieve embedding for this memory (N+1 pattern — bounded by LIMIT 200 above)
        let emb_result: rusqlite::Result<Vec<u8>> = conn.query_row(
            "SELECT embedding FROM memory_vec WHERE id = ?1",
            params![id],
            |row| row.get(0),
        );
        let emb_bytes = match emb_result {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[consolidator] embedding lookup failed for {id}: {e}");
                continue;
            }
        };

        // KNN search for similar embeddings (search for more than we need to filter)
        let mut knn_stmt = conn.prepare(
            "SELECT v.id, v.distance FROM memory_vec v
             WHERE v.embedding MATCH ?1 AND k = 10",
        )?;
        let neighbors: Vec<(String, f64)> = knn_stmt
            .query_map(params![emb_bytes], |row| Ok((row.get(0)?, row.get(1)?)))?
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
            let neighbor_info: Option<(String, String)> = conn
                .query_row(
                    "SELECT memory_type, status FROM memory WHERE id = ?1",
                    params![neighbor_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            match neighbor_info {
                Some((ref n_type, ref n_status)) if n_type == mem_type && n_status == "active" => {
                    // Mark the neighbor as superseded (current memory has higher confidence
                    // due to ORDER BY confidence DESC)
                    conn.execute(
                        "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                        params![neighbor_id],
                    )?;
                    // Create supersedes edge
                    if let Err(e) = store_edge(conn, id, neighbor_id, "supersedes", "{}") {
                        eprintln!("[consolidator] failed to create supersedes edge: {e}");
                    }
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
           AND m2.status = 'active'",
    )?;

    let edges: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut strengthened = 0usize;

    for (edge_id, properties) in &edges {
        let mut props: serde_json::Value =
            serde_json::from_str(properties).unwrap_or(serde_json::json!({}));
        let current = props
            .get("strength")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
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
         WHERE status = 'active' AND valence IN ('positive', 'negative') AND intensity > 0.5",
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
            let diag_id = format!("contradiction-{id_a}-{id_b}");
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
                "Contradictory memories detected: \"{title_a}\" ({valence_a}) vs \"{title_b}\" ({valence_b}). {shared} shared tags."
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

            // Create a 'contradicts' edge between the two memories
            let edge_id = format!("edge-contradiction-{id_a}-{id_b}");
            let _ = conn.execute(
                "INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
                 VALUES (?1, ?2, ?3, 'contradicts', ?4, ?5, ?5)",
                params![
                    edge_id, id_a, id_b,
                    format!("{{\"shared_tags\":{}}}", shared),
                    forge_core::time::now_iso(),
                ],
            );
            found += 1;
        }
    }

    Ok(found)
}

/// FNV-1a 64-bit hash for deterministic position hints.
fn fnv_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Map memory_type to (layer_name, y_position).
fn layer_for_memory_type(memory_type: &str) -> (&str, f64) {
    match memory_type {
        "decision" => ("experience", 4.0),
        "lesson" => ("experience", 4.0),
        "pattern" => ("experience", 3.5),
        "preference" => ("experience", 3.0),
        _ => ("experience", 4.0),
    }
}

/// Get graph data for Cortex 3D brain map visualization.
/// Returns memory nodes with position hints + edges between them.
pub fn get_graph_data(
    conn: &Connection,
    layer_filter: Option<&str>,
    limit_per_layer: usize,
) -> rusqlite::Result<(
    Vec<forge_core::protocol::GraphNode>,
    Vec<forge_core::protocol::GraphEdge>,
)> {
    use forge_core::protocol::{GraphEdge, GraphNode};

    let mut nodes = Vec::new();
    let limit_total = (limit_per_layer * 8) as i64;

    // ── Experience layer: memories ──
    if layer_filter.is_none() || layer_filter == Some("experience") {
        let mut stmt = conn.prepare(
            "SELECT id, title, memory_type, confidence, COALESCE(activation_level, 0.0)
             FROM memory WHERE status = 'active'
             ORDER BY COALESCE(activation_level, 0.0) DESC, confidence DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit_total], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        for row in rows.flatten() {
            let (id, title, memory_type, confidence, activation) = row;
            let (layer, y) = layer_for_memory_type(&memory_type);
            let layer_str = layer.to_string();
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            nodes.push(GraphNode {
                id,
                title,
                memory_type,
                layer: layer_str,
                confidence,
                activation_level: activation,
                x,
                y,
                z,
            });
        }
    }

    // ── Platform layer (Layer 0) ──
    if layer_filter.is_none() || layer_filter == Some("platform") {
        let mut stmt = conn.prepare("SELECT key, value FROM platform LIMIT ?1")?;
        let rows = stmt.query_map(params![limit_per_layer as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            let (key, value) = row;
            let id = format!("platform:{key}");
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            nodes.push(GraphNode {
                id,
                title: format!("{key}: {value}"),
                memory_type: "platform".to_string(),
                layer: "platform".to_string(),
                confidence: 1.0,
                activation_level: 0.0,
                x,
                y: 0.0,
                z,
            });
        }
    }

    // ── Tool layer (Layer 1) ──
    if layer_filter.is_none() || layer_filter == Some("tool") {
        let mut stmt = conn.prepare("SELECT id, name, kind FROM tool LIMIT ?1")?;
        let rows = stmt.query_map(params![limit_per_layer as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows.flatten() {
            let (id, name, kind) = row;
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            nodes.push(GraphNode {
                id,
                title: name,
                memory_type: kind,
                layer: "tool".to_string(),
                confidence: 1.0,
                activation_level: 0.0,
                x,
                y: 1.0,
                z,
            });
        }
    }

    // ── Skill layer (Layer 2) ──
    if layer_filter.is_none() || layer_filter == Some("skill") {
        let mut stmt =
            conn.prepare("SELECT id, name, domain, success_count FROM skill LIMIT ?1")?;
        let rows = stmt.query_map(params![limit_per_layer as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows.flatten() {
            let (id, name, domain, success_count) = row;
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            let confidence = (0.5 + (success_count as f64 * 0.1)).min(1.0);
            nodes.push(GraphNode {
                id,
                title: format!("[{domain}] {name}"),
                memory_type: "skill".to_string(),
                layer: "skill".to_string(),
                confidence,
                activation_level: 0.0,
                x,
                y: 2.0,
                z,
            });
        }
    }

    // ── Identity layer (Layer 6) ──
    if layer_filter.is_none() || layer_filter == Some("identity") {
        let mut stmt = conn.prepare(
            "SELECT id, agent, facet, description, strength FROM identity WHERE active = 1 LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit_per_layer as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        for row in rows.flatten() {
            let (id, agent, facet, _description, strength) = row;
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            nodes.push(GraphNode {
                id,
                title: format!("[{agent}] {facet}"),
                memory_type: "identity".to_string(),
                layer: "identity".to_string(),
                confidence: strength,
                activation_level: 0.0,
                x,
                y: 6.0,
                z,
            });
        }
    }

    // ── Disposition layer (Layer 7) ──
    if layer_filter.is_none() || layer_filter == Some("disposition") {
        let mut stmt =
            conn.prepare("SELECT id, agent, trait_name, value FROM disposition LIMIT ?1")?;
        let rows = stmt.query_map(params![limit_per_layer as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })?;
        for row in rows.flatten() {
            let (id, agent, trait_name, value) = row;
            let hash = fnv_hash(id.as_bytes());
            let x = ((hash % 1000) as f64 / 500.0) - 1.0;
            let z = (((hash >> 16) % 1000) as f64 / 500.0) - 1.0;
            nodes.push(GraphNode {
                id,
                title: format!("[{agent}] {trait_name}"),
                memory_type: "disposition".to_string(),
                layer: "disposition".to_string(),
                confidence: value,
                activation_level: 0.0,
                x,
                y: 7.0,
                z,
            });
        }
    }

    // ── Edges ──
    // Only include edges where both from_id and to_id are in our node set
    let node_ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut edges = Vec::new();
    let mut edge_stmt =
        conn.prepare("SELECT from_id, to_id, edge_type, properties FROM edge LIMIT ?1")?;
    let edge_rows = edge_stmt.query_map(params![limit_total * 2], |row| {
        let from_id: String = row.get(0)?;
        let to_id: String = row.get(1)?;
        let edge_type: String = row.get(2)?;
        let props: String = row.get(3)?;
        let strength: f64 = serde_json::from_str::<serde_json::Value>(&props)
            .ok()
            .and_then(|v| v.get("strength").and_then(|s| s.as_f64()))
            .unwrap_or(0.5);
        Ok(GraphEdge {
            from_id,
            to_id,
            edge_type,
            strength,
        })
    })?;
    for edge in edge_rows.flatten() {
        if node_ids.contains(edge.from_id.as_str()) || node_ids.contains(edge.to_id.as_str()) {
            edges.push(edge);
        }
    }

    Ok((nodes, edges))
}

// ── v2.0 Entity CRUD operations ──

use forge_core::types::{ForgeUser, Organization, Reality, Team, TeamMember};

/// Create the default organization if it does not exist.
pub fn create_default_org(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO organization (id, name, created_at, updated_at)
         VALUES ('default', 'Default', datetime('now'), datetime('now'))",
        [],
    )?;
    Ok(())
}

/// Get an organization by ID.
pub fn get_organization(conn: &Connection, id: &str) -> rusqlite::Result<Option<Organization>> {
    conn.query_row(
        "SELECT id, name, created_at, updated_at FROM organization WHERE id = ?1",
        params![id],
        |row| {
            Ok(Organization {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// List all organizations.
pub fn list_organizations(conn: &Connection) -> rusqlite::Result<Vec<Organization>> {
    let mut stmt =
        conn.prepare("SELECT id, name, created_at, updated_at FROM organization ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(Organization {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Create the default local user if it does not exist.
pub fn create_default_user(conn: &Connection, username: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO forge_user (id, name, email, organization_id, created_at, updated_at)
         VALUES ('local', ?1, NULL, 'default', datetime('now'), datetime('now'))",
        params![username],
    )?;
    Ok(())
}

/// Get a user by ID.
pub fn get_user(conn: &Connection, id: &str) -> rusqlite::Result<Option<ForgeUser>> {
    conn.query_row(
        "SELECT id, name, email, organization_id, created_at, updated_at FROM forge_user WHERE id = ?1",
        params![id],
        |row| Ok(ForgeUser {
            id: row.get(0)?,
            name: row.get(1)?,
            email: row.get(2)?,
            organization_id: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        }),
    ).optional()
}

/// List users in an organization.
pub fn list_users(conn: &Connection, org_id: &str) -> rusqlite::Result<Vec<ForgeUser>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, email, organization_id, created_at, updated_at FROM forge_user WHERE organization_id = ?1 ORDER BY name"
    )?;
    let rows = stmt.query_map(params![org_id], |row| {
        Ok(ForgeUser {
            id: row.get(0)?,
            name: row.get(1)?,
            email: row.get(2)?,
            organization_id: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Create a new team. Returns the team ID.
pub fn create_team(
    conn: &Connection,
    name: &str,
    org_id: &str,
    created_by: &str,
) -> rusqlite::Result<String> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
         VALUES (?1, ?2, ?3, ?4, 'active', datetime('now'))",
        params![id, name, org_id, created_by],
    )?;
    Ok(id)
}

/// Get a team by ID, scoped to the given organization.
pub fn get_team(conn: &Connection, id: &str, org_id: &str) -> rusqlite::Result<Option<Team>> {
    conn.query_row(
        "SELECT id, name, organization_id, created_by, status, created_at FROM team WHERE id = ?1 AND organization_id = ?2",
        params![id, org_id],
        |row| Ok(Team {
            id: row.get(0)?,
            name: row.get(1)?,
            organization_id: row.get(2)?,
            created_by: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
        }),
    ).optional()
}

/// List teams in an organization.
pub fn list_teams(conn: &Connection, org_id: &str) -> rusqlite::Result<Vec<Team>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, organization_id, created_by, status, created_at FROM team WHERE organization_id = ?1 ORDER BY name"
    )?;
    let rows = stmt.query_map(params![org_id], |row| {
        Ok(Team {
            id: row.get(0)?,
            name: row.get(1)?,
            organization_id: row.get(2)?,
            created_by: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Add a member to a team. Verifies the team belongs to the given organization first.
pub fn add_team_member(
    conn: &Connection,
    team_id: &str,
    user_id: &str,
    role: &str,
    org_id: &str,
) -> rusqlite::Result<()> {
    // Verify the team belongs to the specified organization
    let team_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM team WHERE id = ?1 AND organization_id = ?2",
        params![team_id, org_id],
        |row| row.get(0),
    )?;
    if !team_exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    conn.execute(
        "INSERT OR REPLACE INTO team_member (team_id, user_id, role, joined_at)
         VALUES (?1, ?2, ?3, datetime('now'))",
        params![team_id, user_id, role],
    )?;
    Ok(())
}

/// List members of a team. Verifies the team belongs to the given organization first.
pub fn list_team_members(
    conn: &Connection,
    team_id: &str,
    org_id: &str,
) -> rusqlite::Result<Vec<TeamMember>> {
    // Verify the team belongs to the specified organization
    let team_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM team WHERE id = ?1 AND organization_id = ?2",
        params![team_id, org_id],
        |row| row.get(0),
    )?;
    if !team_exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    let mut stmt = conn.prepare(
        "SELECT team_id, user_id, role, joined_at FROM team_member WHERE team_id = ?1 ORDER BY joined_at"
    )?;
    let rows = stmt.query_map(params![team_id], |row| {
        Ok(TeamMember {
            team_id: row.get(0)?,
            user_id: row.get(1)?,
            role: row.get(2)?,
            joined_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Store a reality record (upsert by ID).
pub fn store_reality(conn: &Connection, reality: &Reality) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO reality (id, name, reality_type, detected_from, project_path, domain, organization_id, owner_type, owner_id, engine_status, engine_pid, created_at, last_active, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            reality.id, reality.name, reality.reality_type,
            reality.detected_from, reality.project_path, reality.domain,
            reality.organization_id, reality.owner_type, reality.owner_id,
            reality.engine_status, reality.engine_pid,
            reality.created_at, reality.last_active, reality.metadata,
        ],
    )?;
    Ok(())
}

/// Get a reality by ID, scoped to the given organization.
pub fn get_reality(conn: &Connection, id: &str, org_id: &str) -> rusqlite::Result<Option<Reality>> {
    conn.query_row(
        "SELECT id, name, reality_type, detected_from, project_path, domain, organization_id, owner_type, owner_id, engine_status, engine_pid, created_at, last_active, COALESCE(metadata, '{}')
         FROM reality WHERE id = ?1 AND organization_id = ?2",
        params![id, org_id],
        |row| Ok(Reality {
            id: row.get(0)?,
            name: row.get(1)?,
            reality_type: row.get(2)?,
            detected_from: row.get(3)?,
            project_path: row.get(4)?,
            domain: row.get(5)?,
            organization_id: row.get(6)?,
            owner_type: row.get(7)?,
            owner_id: row.get(8)?,
            engine_status: row.get(9)?,
            engine_pid: row.get(10)?,
            created_at: row.get(11)?,
            last_active: row.get(12)?,
            metadata: row.get(13)?,
        }),
    ).optional()
}

/// Get a reality by its project path, scoped to the given organization.
pub fn get_reality_by_path(
    conn: &Connection,
    path: &str,
    org_id: &str,
) -> rusqlite::Result<Option<Reality>> {
    conn.query_row(
        "SELECT id, name, reality_type, detected_from, project_path, domain, organization_id, owner_type, owner_id, engine_status, engine_pid, created_at, last_active, COALESCE(metadata, '{}')
         FROM reality WHERE project_path = ?1 AND organization_id = ?2 LIMIT 1",
        params![path, org_id],
        |row| Ok(Reality {
            id: row.get(0)?,
            name: row.get(1)?,
            reality_type: row.get(2)?,
            detected_from: row.get(3)?,
            project_path: row.get(4)?,
            domain: row.get(5)?,
            organization_id: row.get(6)?,
            owner_type: row.get(7)?,
            owner_id: row.get(8)?,
            engine_status: row.get(9)?,
            engine_pid: row.get(10)?,
            created_at: row.get(11)?,
            last_active: row.get(12)?,
            metadata: row.get(13)?,
        }),
    ).optional()
}

/// List realities in an organization.
pub fn list_realities(conn: &Connection, org_id: &str) -> rusqlite::Result<Vec<Reality>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, reality_type, detected_from, project_path, domain, organization_id, owner_type, owner_id, engine_status, engine_pid, created_at, last_active, COALESCE(metadata, '{}')
         FROM reality WHERE organization_id = ?1 ORDER BY last_active DESC"
    )?;
    let rows = stmt.query_map(params![org_id], |row| {
        Ok(Reality {
            id: row.get(0)?,
            name: row.get(1)?,
            reality_type: row.get(2)?,
            detected_from: row.get(3)?,
            project_path: row.get(4)?,
            domain: row.get(5)?,
            organization_id: row.get(6)?,
            owner_type: row.get(7)?,
            owner_id: row.get(8)?,
            engine_status: row.get(9)?,
            engine_pid: row.get(10)?,
            created_at: row.get(11)?,
            last_active: row.get(12)?,
            metadata: row.get(13)?,
        })
    })?;
    rows.collect()
}

/// Update the last_active timestamp for a reality, scoped to the given organization.
pub fn update_reality_last_active(
    conn: &Connection,
    id: &str,
    org_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE reality SET last_active = datetime('now') WHERE id = ?1 AND organization_id = ?2",
        params![id, org_id],
    )?;
    Ok(())
}

// ── v2.0 Scoped Configuration CRUD + Resolution ──

use forge_core::types::entity::{ConfigScopeEntry, ResolvedConfigValue};
use std::collections::HashMap;

/// Valid scope types for configuration resolution, ordered from most specific to least specific.
const VALID_SCOPE_TYPES: &[&str] = &[
    "session",
    "agent",
    "reality",
    "user",
    "team",
    "organization",
];

/// Validate that a scope_type string is one of the known scope types.
pub fn validate_scope_type(scope_type: &str) -> bool {
    VALID_SCOPE_TYPES.contains(&scope_type)
}

/// Set (upsert) a scoped configuration entry.
#[allow(clippy::too_many_arguments)]
pub fn set_scoped_config(
    conn: &Connection,
    scope_type: &str,
    scope_id: &str,
    key: &str,
    value: &str,
    locked: bool,
    ceiling: Option<f64>,
    set_by: &str,
) -> rusqlite::Result<()> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO config_scope (id, scope_type, scope_id, key, value, locked, ceiling, set_by, set_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
         ON CONFLICT(scope_type, scope_id, key) DO UPDATE SET
           value = excluded.value,
           locked = excluded.locked,
           ceiling = excluded.ceiling,
           set_by = excluded.set_by,
           set_at = excluded.set_at",
        params![id, scope_type, scope_id, key, value, locked as i32, ceiling, set_by],
    )?;
    Ok(())
}

/// Get a single scoped configuration entry.
pub fn get_scoped_config(
    conn: &Connection,
    scope_type: &str,
    scope_id: &str,
    key: &str,
) -> rusqlite::Result<Option<ConfigScopeEntry>> {
    conn.query_row(
        "SELECT id, scope_type, scope_id, key, value, locked, ceiling, set_by, set_at
         FROM config_scope WHERE scope_type = ?1 AND scope_id = ?2 AND key = ?3",
        params![scope_type, scope_id, key],
        |row| {
            let locked_int: i32 = row.get(5)?;
            Ok(ConfigScopeEntry {
                id: row.get(0)?,
                scope_type: row.get(1)?,
                scope_id: row.get(2)?,
                key: row.get(3)?,
                value: row.get(4)?,
                locked: locked_int != 0,
                ceiling: row.get(6)?,
                set_by: row.get(7)?,
                set_at: row.get(8)?,
            })
        },
    )
    .optional()
}

/// List all scoped configuration entries for a given scope.
pub fn list_scoped_config(
    conn: &Connection,
    scope_type: &str,
    scope_id: &str,
) -> rusqlite::Result<Vec<ConfigScopeEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_type, scope_id, key, value, locked, ceiling, set_by, set_at
         FROM config_scope WHERE scope_type = ?1 AND scope_id = ?2 ORDER BY key",
    )?;
    let rows = stmt.query_map(params![scope_type, scope_id], |row| {
        let locked_int: i32 = row.get(5)?;
        Ok(ConfigScopeEntry {
            id: row.get(0)?,
            scope_type: row.get(1)?,
            scope_id: row.get(2)?,
            key: row.get(3)?,
            value: row.get(4)?,
            locked: locked_int != 0,
            ceiling: row.get(6)?,
            set_by: row.get(7)?,
            set_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

/// Delete a scoped configuration entry. Returns true if a row was deleted.
pub fn delete_scoped_config(
    conn: &Connection,
    scope_type: &str,
    scope_id: &str,
    key: &str,
) -> rusqlite::Result<bool> {
    let changes = conn.execute(
        "DELETE FROM config_scope WHERE scope_type = ?1 AND scope_id = ?2 AND key = ?3",
        params![scope_type, scope_id, key],
    )?;
    Ok(changes > 0)
}

/// Resolve a single configuration key through the scope chain.
///
/// Resolution algorithm:
/// 1. Build scope chain from most-specific to least-specific (session->agent->reality->user->team->org)
/// 2. Walk from LEAST specific to MOST specific:
///    - If entry is locked, set result = this value, mark locked
///    - Track tightest ceiling from any ancestor
///    - If NOT locked by ancestor, set result = this value (most-specific wins)
/// 3. Apply ceiling: if final value is numeric and exceeds ceiling, clamp it
#[allow(clippy::too_many_arguments)]
pub fn resolve_scoped_config(
    conn: &Connection,
    key: &str,
    session_id: Option<&str>,
    agent: Option<&str>,
    reality_id: Option<&str>,
    user_id: Option<&str>,
    team_id: Option<&str>,
    org_id: Option<&str>,
) -> rusqlite::Result<Option<ResolvedConfigValue>> {
    // Build scope chain from least-specific to most-specific
    let mut scope_chain: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = org_id {
        scope_chain.push(("organization", id));
    }
    if let Some(id) = team_id {
        scope_chain.push(("team", id));
    }
    if let Some(id) = user_id {
        scope_chain.push(("user", id));
    }
    if let Some(id) = reality_id {
        scope_chain.push(("reality", id));
    }
    if let Some(id) = agent {
        scope_chain.push(("agent", id));
    }
    if let Some(id) = session_id {
        scope_chain.push(("session", id));
    }

    if scope_chain.is_empty() {
        return Ok(None);
    }

    // Fetch entries for this key from all scope levels
    let mut entries: Vec<(usize, ConfigScopeEntry)> = Vec::new();
    for (idx, (st, sid)) in scope_chain.iter().enumerate() {
        if let Some(entry) = get_scoped_config(conn, st, sid, key)? {
            entries.push((idx, entry));
        }
    }

    if entries.is_empty() {
        return Ok(None);
    }

    // Walk from least-specific (lowest index) to most-specific (highest index)
    let mut result_value: Option<String> = None;
    let mut result_scope_type: Option<String> = None;
    let mut result_scope_id: Option<String> = None;
    let mut is_locked = false;
    let mut tightest_ceiling: Option<f64> = None;

    // Entries are already ordered by index (least-specific first)
    for (_, entry) in &entries {
        // Track tightest ceiling from any level
        if let Some(c) = entry.ceiling {
            tightest_ceiling = Some(match tightest_ceiling {
                Some(existing) => existing.min(c),
                None => c,
            });
        }

        if is_locked {
            // A less-specific scope locked it; don't override
            continue;
        }

        // Most-specific wins (we walk least-specific first, so keep overwriting)
        result_value = Some(entry.value.clone());
        result_scope_type = Some(entry.scope_type.clone());
        result_scope_id = Some(entry.scope_id.clone());

        if entry.locked {
            is_locked = true;
        }
    }

    let mut final_value = match result_value {
        Some(v) => v,
        None => return Ok(None),
    };

    // Apply ceiling: if final value is numeric and exceeds ceiling, clamp it
    let mut ceiling_applied = false;
    if let Some(ceiling) = tightest_ceiling {
        if let Ok(numeric) = final_value.parse::<f64>() {
            if numeric > ceiling {
                final_value = ceiling.to_string();
                ceiling_applied = true;
            }
        }
    }

    Ok(Some(ResolvedConfigValue {
        key: key.to_string(),
        value: final_value,
        source_scope_type: result_scope_type.unwrap_or_default(),
        source_scope_id: result_scope_id.unwrap_or_default(),
        locked: is_locked,
        ceiling_applied,
    }))
}

/// Resolve effective configuration for all keys in the scope chain.
///
/// Collects all unique keys from every scope level, then resolves each one.
pub fn resolve_effective_config(
    conn: &Connection,
    session_id: Option<&str>,
    agent: Option<&str>,
    reality_id: Option<&str>,
    user_id: Option<&str>,
    team_id: Option<&str>,
    org_id: Option<&str>,
) -> rusqlite::Result<HashMap<String, ResolvedConfigValue>> {
    // Build scope chain to collect all keys
    let mut scope_chain: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = org_id {
        scope_chain.push(("organization", id));
    }
    if let Some(id) = team_id {
        scope_chain.push(("team", id));
    }
    if let Some(id) = user_id {
        scope_chain.push(("user", id));
    }
    if let Some(id) = reality_id {
        scope_chain.push(("reality", id));
    }
    if let Some(id) = agent {
        scope_chain.push(("agent", id));
    }
    if let Some(id) = session_id {
        scope_chain.push(("session", id));
    }

    // Collect all unique keys across all scope levels
    let mut all_keys: HashSet<String> = HashSet::new();
    for (st, sid) in &scope_chain {
        let entries = list_scoped_config(conn, st, sid)?;
        for entry in entries {
            all_keys.insert(entry.key);
        }
    }

    // Resolve each key
    let mut result = HashMap::new();
    for key in &all_keys {
        if let Some(resolved) = resolve_scoped_config(
            conn, key, session_id, agent, reality_id, user_id, team_id, org_id,
        )? {
            result.insert(key.clone(), resolved);
        }
    }

    Ok(result)
}

/// Classify memory portability for memories with portability='unknown'.
///
/// Rules:
/// - universal: memory_type = 'preference' OR tags contain 'principle'/'heuristic'
/// - reality_bound: content contains file paths or port numbers
/// - domain_transferable: everything else (conservative default)
pub fn classify_portability(conn: &Connection, batch_limit: usize) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, title, content, COALESCE(tags, '[]')
         FROM memory WHERE portability = 'unknown' AND status = 'active'
         LIMIT ?1",
    )?;
    let rows: Vec<(String, String, String, String, String)> = stmt
        .query_map(params![batch_limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut classified = 0usize;
    for (id, memory_type, _title, content, tags) in &rows {
        let has_locality = contains_file_path(content) || contains_port_number(content);

        // Locality evidence DOMINATES — if content references specific files/ports,
        // it's reality_bound regardless of tags or type.
        let portability = if has_locality {
            "reality_bound"
        } else if memory_type == "preference" {
            "universal"
        } else {
            // Parse tags as JSON array for exact matching (not substring)
            let parsed_tags: Vec<String> = serde_json::from_str(tags).unwrap_or_default();
            let has_universal_tag = parsed_tags.iter().any(|t| {
                let lower = t.to_lowercase();
                lower == "principle" || lower == "heuristic"
            });
            if has_universal_tag {
                "universal"
            } else {
                "domain_transferable"
            }
        };

        conn.execute(
            "UPDATE memory SET portability = ?1 WHERE id = ?2",
            params![portability, id],
        )?;
        classified += 1;
    }

    Ok(classified)
}

/// Check if content contains file path patterns.
fn contains_file_path(content: &str) -> bool {
    // Match patterns like /path/to/file.ext, ./relative, src/main.rs
    for word in content.split_whitespace() {
        let w = word.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == ',');
        if (w.contains('/') || w.contains('\\'))
            && w.len() > 3
            && !w.starts_with("http")
            && !w.starts_with("//")
        {
            // Check for file extension pattern
            if let Some(last_segment) = w.rsplit('/').next() {
                if last_segment.contains('.') && last_segment.len() > 2 {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if content contains port number patterns (e.g., :3000, :8080).
fn contains_port_number(content: &str) -> bool {
    let bytes = content.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b':' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let len = end - start;
            if (2..=5).contains(&len) {
                return true;
            }
        }
    }
    false
}

/// Ensure default organization and local user exist (idempotent, called on first run).
pub fn ensure_defaults(conn: &Connection) -> rusqlite::Result<()> {
    create_default_org(conn)?;
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "local".to_string());
    create_default_user(conn, &username)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use forge_core::types::{CodeFile, CodeSymbol, Memory, MemoryType};

    fn open_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_remember_and_recall() {
        let conn = open_db();

        let m = Memory::new(
            MemoryType::Decision,
            "Use SQLite for storage",
            "SQLite FTS5 gives fast BM25 recall",
        );
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
        assert_eq!(
            after.len(),
            0,
            "superseded memory should not appear in recall"
        );

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
        assert!(
            !results.is_empty(),
            "should find JWT despite FTS5 operator chars in query"
        );
    }

    #[test]
    fn test_sanitize_fts5_query() {
        let sanitized = sanitize_fts5_query("JWT AND authentication NOT bad");
        assert_eq!(
            sanitized,
            r#""JWT" OR "AND" OR "authentication" OR "NOT" OR "bad""#
        );

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

        let (checked, faded) = decay_memories(&conn, 1000).unwrap();
        assert_eq!(checked, 2, "should check both memories");
        assert_eq!(
            faded, 0,
            "30-day memory at 0.9 base should not be faded yet"
        );

        // Crucially: stored confidence is NEVER modified
        let mid_conf: f64 = conn
            .query_row("SELECT confidence FROM memory WHERE id = 'mid1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            (mid_conf - 0.9).abs() < 0.001,
            "stored confidence must remain 0.9, got {mid_conf}"
        );

        let new_conf: f64 = conn
            .query_row("SELECT confidence FROM memory WHERE id = 'new1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            (new_conf - 0.9).abs() < 0.001,
            "stored confidence must remain 0.9, got {new_conf}"
        );
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

        let (checked, faded) = decay_memories(&conn, 1000).unwrap();
        assert_eq!(checked, 2);
        assert_eq!(faded, 1, "90-day-old memory should be faded");

        let old_status: String = conn
            .query_row("SELECT status FROM memory WHERE id = 'old1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(old_status, "faded");

        let new_status: String = conn
            .query_row("SELECT status FROM memory WHERE id = 'new1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(new_status, "active");

        // Stored confidence is STILL not modified
        let old_conf: f64 = conn
            .query_row("SELECT confidence FROM memory WHERE id = 'old1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            (old_conf - 0.9).abs() < 0.001,
            "stored confidence must remain 0.9 even after fading, got {old_conf}"
        );
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
        let (_, faded1) = decay_memories(&conn, 1000).unwrap();
        let (_, faded2) = decay_memories(&conn, 1000).unwrap();
        let (_, faded3) = decay_memories(&conn, 1000).unwrap();

        assert_eq!(
            faded1, faded2,
            "repeated decay runs must produce same result"
        );
        assert_eq!(
            faded2, faded3,
            "repeated decay runs must produce same result"
        );

        // Confidence is still untouched
        let conf: f64 = conn
            .query_row("SELECT confidence FROM memory WHERE id = 'm1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            (conf - 0.9).abs() < 0.001,
            "confidence must not change across multiple decay runs, got {conf}"
        );
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
        assert!(
            dt.unwrap() > 1_700_000_000.0,
            "parsed datetime should be a reasonable epoch"
        );

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
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "a".into(),
            indexed_at: "1".into(),
        };
        let f2 = CodeFile {
            id: "f2".into(),
            path: "src/old.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "b".into(),
            indexed_at: "1".into(),
        };
        store_file(&conn, &f1).unwrap();
        store_file(&conn, &f2).unwrap();

        let s1 = CodeSymbol {
            id: "s1".into(),
            name: "main".into(),
            kind: "function".into(),
            file_path: "src/main.rs".into(),
            line_start: 1,
            line_end: Some(10),
            signature: Some("fn main()".into()),
        };
        let s2 = CodeSymbol {
            id: "s2".into(),
            name: "old_fn".into(),
            kind: "function".into(),
            file_path: "src/old.rs".into(),
            line_start: 1,
            line_end: Some(5),
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
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "a".into(),
            indexed_at: "1".into(),
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
        assert_eq!(
            count_files(&conn).unwrap(),
            1,
            "upsert should not duplicate"
        );

        let stored_hash: String = conn
            .query_row("SELECT hash FROM code_file WHERE id = 'f1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored_hash, "def", "upsert should update hash");
    }

    #[test]
    fn test_remember_dedup_by_title() {
        let conn = open_db();
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "First version");
        remember(&conn, &m1).unwrap();

        let m2 =
            Memory::new(MemoryType::Decision, "Use JWT", "Updated version").with_confidence(0.95);
        remember(&conn, &m2).unwrap();

        // Should still be 1 decision, not 2
        let h = health(&conn).unwrap();
        assert_eq!(h.decisions, 1, "dedup should prevent duplicate titles");

        // Content should be updated
        let results = recall_bm25(&conn, "JWT", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0].content.contains("Updated"),
            "content should be updated"
        );
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
        let m1 = Memory::new(MemoryType::Decision, "JWT for forge", "auth").with_project("forge");
        remember(&conn, &m1).unwrap();

        let m2 = Memory::new(MemoryType::Decision, "CORS for forge", "cors").with_project("forge");
        remember(&conn, &m2).unwrap();

        let m3 =
            Memory::new(MemoryType::Decision, "REST for backend", "api").with_project("backend");
        remember(&conn, &m3).unwrap();

        let m4 = Memory::new(
            MemoryType::Decision,
            "Use conventional commits",
            "global rule",
        );
        // project is None by default — global
        remember(&conn, &m4).unwrap();

        // Project-scoped: forge → 2 forge + 1 global = 3
        let results = recall_bm25_project(
            &conn,
            "forge backend global conventional JWT CORS REST commits",
            Some("forge"),
            10,
        )
        .unwrap();
        let titles: Vec<&str> = results.iter().map(|r| r.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("JWT")),
            "should find forge memory JWT, got: {titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.contains("CORS")),
            "should find forge memory CORS, got: {titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.contains("conventional")),
            "should find global memory, got: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains("REST")),
            "should NOT find backend memory, got: {titles:?}"
        );
        assert_eq!(
            results.len(),
            3,
            "forge scope should return 2 forge + 1 global = 3"
        );

        // No project filter → all 4
        let all = recall_bm25_project(
            &conn,
            "forge backend global conventional JWT CORS REST commits",
            None,
            10,
        )
        .unwrap();
        assert_eq!(
            all.len(),
            4,
            "no project filter should return all 4 memories"
        );
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
        assert_eq!(
            r2.len(),
            1,
            "global memory should appear in backend project"
        );
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

        let results =
            recall_bm25_project(&conn, "empty project memory", Some("anyproject"), 10).unwrap();
        assert_eq!(
            results.len(),
            1,
            "empty-string project memory should appear as global"
        );
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
        assert_eq!(
            result.len(),
            3,
            "should have 3 projects: forge, backend, _global"
        );
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
        let survivor: (String, f64) = conn
            .query_row(
                "SELECT id, confidence FROM memory WHERE status = 'active'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
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
                format!("Always run tests before pushing v{i}"),
                "Learned from breaking prod",
            )
            .with_confidence(0.7);
            remember(&conn, &mem).unwrap();
        }

        let promoted = promote_recurring_lessons(&conn, 1000).unwrap();
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
        let mem3 =
            Memory::new(MemoryType::Lesson, "Test edge cases", "Coverage").with_confidence(0.7);
        remember(&conn, &mem1).unwrap();
        remember(&conn, &mem2).unwrap();
        remember(&conn, &mem3).unwrap();

        let promoted = promote_recurring_lessons(&conn, 1000).unwrap();
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
    fn insert_memory_for_dedup(
        conn: &Connection,
        id: &str,
        mem_type: &str,
        title: &str,
        content: &str,
        project: &str,
        confidence: f64,
    ) {
        let proj_val: Option<&str> = if project.is_empty() {
            None
        } else {
            Some(project)
        };
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
        insert_memory_for_dedup(
            &conn,
            "a1",
            "decision",
            "Use SQLite for storage",
            "SQLite FTS5 fast recall",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "a2",
            "decision",
            "Use SQLite for storage",
            "SQLite FTS5 fast recall",
            "",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(merged, 1, "identical title+content should be deduped");

        // Higher confidence (a1=0.9) should survive
        let status_a1: String = conn
            .query_row("SELECT status FROM memory WHERE id = 'a1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let status_a2: String = conn
            .query_row("SELECT status FROM memory WHERE id = 'a2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status_a1, "active");
        assert_eq!(status_a2, "superseded");
    }

    #[test]
    fn test_semantic_dedup_different_titles_same_content() {
        let conn = open_db();
        // Different titles but same content should now be caught via content overlap
        insert_memory_for_dedup(
            &conn,
            "b1",
            "lesson",
            "Code indexer broken",
            "The code indexer keeps crashing when parsing large files with many symbols",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "b2",
            "lesson",
            "Indexer crash bug",
            "The code indexer keeps crashing when parsing large files with many symbols",
            "",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 1,
            "different titles but same content should be deduped via content overlap"
        );
    }

    #[test]
    fn test_semantic_dedup_stop_words_only_overlap_no_dedup() {
        let conn = open_db();
        // Two memories where only stop words overlap — should NOT dedup
        insert_memory_for_dedup(
            &conn,
            "c1",
            "decision",
            "Use JWT authentication",
            "Token based auth with RS256 signing",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "c2",
            "decision",
            "Deploy Kubernetes cluster",
            "Container orchestration with Helm charts",
            "",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 0,
            "memories with no meaningful word overlap should not be deduped"
        );
    }

    #[test]
    fn test_semantic_dedup_different_types_no_dedup() {
        let conn = open_db();
        // Same title and content but different types — should NOT dedup
        insert_memory_for_dedup(
            &conn,
            "d1",
            "decision",
            "Use SQLite storage",
            "SQLite FTS5 for recall",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "d2",
            "lesson",
            "Use SQLite storage",
            "SQLite FTS5 for recall",
            "",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(merged, 0, "different types should never be deduped");
    }

    #[test]
    fn test_semantic_dedup_different_projects_no_dedup() {
        let conn = open_db();
        // Same title and content but different projects — should NOT dedup
        insert_memory_for_dedup(
            &conn,
            "e1",
            "decision",
            "Use SQLite storage",
            "SQLite FTS5 for recall",
            "forge",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "e2",
            "decision",
            "Use SQLite storage",
            "SQLite FTS5 for recall",
            "backend",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(merged, 0, "different projects should never be deduped");
    }

    #[test]
    fn test_semantic_dedup_known_audit_duplicates() {
        let conn = open_db();
        // Near-duplicate pair with very similar titles AND content (should exceed 0.65 threshold).
        // Both title and content share many meaningful words.
        insert_memory_for_dedup(
            &conn, "f1", "decision",
            "Graph edges auto-generated from extraction and consolidation",
            "Graph edges are automatically generated during memory extraction and consolidation phases for knowledge linking",
            "", 0.9,
        );
        insert_memory_for_dedup(
            &conn, "f2", "decision",
            "Graph edges auto-generated from extraction and consolidation process",
            "Graph edges are automatically generated during memory extraction and consolidation for linking knowledge",
            "", 0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 1,
            "near-duplicates with similar title+content should be caught at 0.65 threshold"
        );

        // Higher confidence should survive
        let status_f1: String = conn
            .query_row("SELECT status FROM memory WHERE id = 'f1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status_f1, "active",
            "higher confidence memory should survive"
        );
    }

    #[test]
    fn test_semantic_dedup_creates_supersedes_edges() {
        let conn = open_db();
        insert_memory_for_dedup(
            &conn,
            "g1",
            "decision",
            "Use SQLite for storage",
            "Fast BM25 recall engine",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "g2",
            "decision",
            "Use SQLite for storage",
            "Fast BM25 recall engine",
            "",
            0.8,
        );

        semantic_dedup(&conn, 1000).unwrap();

        // Check that a supersedes edge was created
        let edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = 'g1' AND to_id = 'g2' AND edge_type = 'supersedes'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(
            edge_count, 1,
            "supersedes edge should be created from survivor to superseded"
        );
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

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 1,
            "near-similar content should be deduped even with different titles"
        );
    }

    #[test]
    fn test_semantic_dedup_completely_unrelated() {
        let conn = open_db();
        // Completely unrelated memories should not be deduped
        insert_memory_for_dedup(
            &conn,
            "i1",
            "decision",
            "Use PostgreSQL for analytics",
            "Complex aggregation queries benefit from PostgreSQL columnar extensions",
            "",
            0.9,
        );
        insert_memory_for_dedup(
            &conn,
            "i2",
            "decision",
            "Deploy with Docker Compose",
            "Multi-container orchestration simplifies local development environment setup",
            "",
            0.8,
        );

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 0,
            "completely unrelated memories should not be deduped"
        );
    }

    // --- Sleep-cycle graph consolidation tests ---

    #[test]
    fn test_embedding_merge_high_similarity() {
        let conn = open_db();
        use crate::db::vec::store_embedding;

        // Create two memories with identical embeddings (distance = 0, similarity = 1.0)
        let m1 = Memory::new(
            MemoryType::Decision,
            "Use Postgres for data",
            "Postgres is great",
        )
        .with_confidence(0.9);
        let m2 = Memory::new(
            MemoryType::Decision,
            "PostgreSQL for storage",
            "PG is reliable",
        )
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
        let m1_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                params![m1.id],
                |row| row.get(0),
            )
            .unwrap();
        let m2_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                params![m2.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            m1_status, "active",
            "higher confidence memory should survive"
        );
        assert_eq!(
            m2_status, "superseded",
            "lower confidence memory should be superseded"
        );

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
        let m2 = Memory::new(
            MemoryType::Decision,
            "Use React for frontend",
            "UI framework",
        )
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
        let m1_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                params![m1.id],
                |row| row.get(0),
            )
            .unwrap();
        let m2_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                params![m2.id],
                |row| row.get(0),
            )
            .unwrap();
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
        let props_str: String = conn
            .query_row(
                "SELECT properties FROM edge WHERE from_id = ?1 AND to_id = ?2",
                params![m1.id, m2.id],
                |row| row.get(0),
            )
            .unwrap();
        let props: serde_json::Value = serde_json::from_str(&props_str).unwrap();
        let strength = props.get("strength").and_then(|v| v.as_f64()).unwrap();
        assert!(
            (strength - 0.1).abs() < 0.001,
            "strength should be 0.1 after first increment"
        );

        // Strengthen again — should increment to 0.2
        let strengthened2 = strengthen_active_edges(&conn).unwrap();
        assert_eq!(strengthened2, 1);
        let props_str2: String = conn
            .query_row(
                "SELECT properties FROM edge WHERE from_id = ?1 AND to_id = ?2",
                params![m1.id, m2.id],
                |row| row.get(0),
            )
            .unwrap();
        let props2: serde_json::Value = serde_json::from_str(&props_str2).unwrap();
        let strength2 = props2.get("strength").and_then(|v| v.as_f64()).unwrap();
        assert!(
            (strength2 - 0.2).abs() < 0.001,
            "strength should be 0.2 after second increment"
        );
    }

    #[test]
    fn test_detect_contradictions() {
        let conn = open_db();
        use crate::db::diagnostics;

        // Create two memories with shared tags but opposite valence and high intensity
        let m1 = Memory::new(
            MemoryType::Decision,
            "Microservices are great",
            "They scale well",
        )
        .with_tags(vec![
            "architecture".into(),
            "scaling".into(),
            "design".into(),
        ])
        .with_valence("positive", 0.8);
        let m2 = Memory::new(
            MemoryType::Decision,
            "Microservices cause problems",
            "Too complex",
        )
        .with_tags(vec![
            "architecture".into(),
            "scaling".into(),
            "complexity".into(),
        ])
        .with_valence("negative", 0.9);
        remember(&conn, &m1).unwrap();
        remember(&conn, &m2).unwrap();

        let found = detect_contradictions(&conn).unwrap();
        assert_eq!(found, 1, "should detect 1 contradiction");

        // Verify diagnostic was created
        let diags = diagnostics::get_all_active_diagnostics(&conn).unwrap();
        let contradiction_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.source == "forge-consolidator" && d.severity == "warning")
            .collect();
        assert_eq!(
            contradiction_diags.len(),
            1,
            "should have 1 contradiction diagnostic"
        );
        assert!(contradiction_diags[0]
            .message
            .contains("Microservices are great"));
        assert!(contradiction_diags[0]
            .message
            .contains("Microservices cause problems"));

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
        assert_eq!(
            found, 0,
            "should not detect weak contradictions (intensity < 0.5)"
        );

        // Verify no diagnostics were created
        let diags = diagnostics::get_all_active_diagnostics(&conn).unwrap();
        let contradiction_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.source == "forge-consolidator")
            .collect();
        assert_eq!(
            contradiction_diags.len(),
            0,
            "no contradiction diagnostics for weak signals"
        );
    }

    // ── v2.0 Entity CRUD tests ──

    #[test]
    fn test_ensure_defaults_idempotent() {
        let conn = open_db();
        // First call creates default org + user
        ensure_defaults(&conn).unwrap();
        // Second call should not error (idempotent)
        ensure_defaults(&conn).unwrap();

        let org = get_organization(&conn, "default").unwrap();
        assert!(org.is_some(), "default org should exist");
        assert_eq!(org.unwrap().name, "Default");

        let user = get_user(&conn, "local").unwrap();
        assert!(user.is_some(), "local user should exist");
        assert_eq!(user.as_ref().unwrap().organization_id, "default");
    }

    #[test]
    fn test_organization_crud() {
        let conn = open_db();
        create_default_org(&conn).unwrap();

        // Get
        let org = get_organization(&conn, "default").unwrap().unwrap();
        assert_eq!(org.id, "default");
        assert_eq!(org.name, "Default");

        // List
        let orgs = list_organizations(&conn).unwrap();
        assert_eq!(orgs.len(), 1);
        assert_eq!(orgs[0].id, "default");

        // Get non-existent
        let none = get_organization(&conn, "nonexistent").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn test_user_crud() {
        let conn = open_db();
        create_default_org(&conn).unwrap();
        create_default_user(&conn, "testuser").unwrap();

        // Get
        let user = get_user(&conn, "local").unwrap().unwrap();
        assert_eq!(user.id, "local");
        assert_eq!(user.name, "testuser");
        assert_eq!(user.organization_id, "default");
        assert!(user.email.is_none());

        // List by org
        let users = list_users(&conn, "default").unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "testuser");

        // List with wrong org
        let empty = list_users(&conn, "nonexistent").unwrap();
        assert!(empty.is_empty());

        // Idempotent: calling again should not fail
        create_default_user(&conn, "testuser2").unwrap();
        // Name should still be original since INSERT OR IGNORE
        let user2 = get_user(&conn, "local").unwrap().unwrap();
        assert_eq!(user2.name, "testuser");
    }

    #[test]
    fn test_team_crud() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        // Create team
        let team_id = create_team(&conn, "Backend Team", "default", "local").unwrap();
        assert!(!team_id.is_empty());

        // Get team (with correct org)
        let team = get_team(&conn, &team_id, "default").unwrap().unwrap();
        assert_eq!(team.name, "Backend Team");
        assert_eq!(team.organization_id, "default");
        assert_eq!(team.created_by, "local");
        assert_eq!(team.status, "active");

        // List teams
        let teams = list_teams(&conn, "default").unwrap();
        assert_eq!(teams.len(), 1);

        // Get non-existent team
        let none = get_team(&conn, "nonexistent", "default").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn test_team_cross_org_access_denied() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let team_id = create_team(&conn, "Secret Team", "default", "local").unwrap();

        // Accessing with wrong org_id should return None
        let none = get_team(&conn, &team_id, "wrong_org").unwrap();
        assert!(
            none.is_none(),
            "get_team with wrong org_id must return None"
        );

        // Accessing with correct org_id should succeed
        let some = get_team(&conn, &team_id, "default").unwrap();
        assert!(some.is_some());
    }

    #[test]
    fn test_team_members() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();
        let team_id = create_team(&conn, "Frontend Team", "default", "local").unwrap();

        // Add members (with org verification)
        add_team_member(&conn, &team_id, "local", "admin", "default").unwrap();
        add_team_member(&conn, &team_id, "user2", "member", "default").unwrap();

        // List members (with org verification)
        let members = list_team_members(&conn, &team_id, "default").unwrap();
        assert_eq!(members.len(), 2);

        // Verify roles
        let admin = members.iter().find(|m| m.user_id == "local").unwrap();
        assert_eq!(admin.role, "admin");
        let member = members.iter().find(|m| m.user_id == "user2").unwrap();
        assert_eq!(member.role, "member");

        // Re-add with different role should replace
        add_team_member(&conn, &team_id, "local", "member", "default").unwrap();
        let members2 = list_team_members(&conn, &team_id, "default").unwrap();
        assert_eq!(members2.len(), 2);
        let updated = members2.iter().find(|m| m.user_id == "local").unwrap();
        assert_eq!(updated.role, "member");
    }

    #[test]
    fn test_team_member_cross_org_denied() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();
        let team_id = create_team(&conn, "Secure Team", "default", "local").unwrap();
        add_team_member(&conn, &team_id, "local", "admin", "default").unwrap();

        // add_team_member with wrong org should fail
        let result = add_team_member(&conn, &team_id, "attacker", "admin", "wrong_org");
        assert!(result.is_err(), "add_team_member with wrong org must fail");

        // list_team_members with wrong org should fail
        let result = list_team_members(&conn, &team_id, "wrong_org");
        assert!(
            result.is_err(),
            "list_team_members with wrong org must fail"
        );
    }

    #[test]
    fn test_reality_crud() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let reality = Reality {
            id: "r1".to_string(),
            name: "forge".to_string(),
            reality_type: "code".to_string(),
            detected_from: Some("git".to_string()),
            project_path: Some("/home/user/forge".to_string()),
            domain: Some("rust".to_string()),
            organization_id: "default".to_string(),
            owner_type: "user".to_string(),
            owner_id: "local".to_string(),
            engine_status: "idle".to_string(),
            engine_pid: None,
            created_at: "2026-04-05T00:00:00Z".to_string(),
            last_active: "2026-04-05T00:00:00Z".to_string(),
            metadata: "{}".to_string(),
        };

        // Store
        store_reality(&conn, &reality).unwrap();

        // Get by ID (with correct org)
        let got = get_reality(&conn, "r1", "default").unwrap().unwrap();
        assert_eq!(got.name, "forge");
        assert_eq!(got.reality_type, "code");
        assert_eq!(got.detected_from.as_deref(), Some("git"));
        assert_eq!(got.project_path.as_deref(), Some("/home/user/forge"));
        assert_eq!(got.domain.as_deref(), Some("rust"));
        assert!(got.engine_pid.is_none());

        // Get by path (with correct org)
        let by_path = get_reality_by_path(&conn, "/home/user/forge", "default")
            .unwrap()
            .unwrap();
        assert_eq!(by_path.id, "r1");

        // List
        let realities = list_realities(&conn, "default").unwrap();
        assert_eq!(realities.len(), 1);

        // Get non-existent
        let none = get_reality(&conn, "nonexistent", "default").unwrap();
        assert!(none.is_none());
        let none_path = get_reality_by_path(&conn, "/nonexistent", "default").unwrap();
        assert!(none_path.is_none());
    }

    #[test]
    fn test_reality_cross_org_access_denied() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let reality = Reality {
            id: "r_sec".to_string(),
            name: "secret-project".to_string(),
            reality_type: "code".to_string(),
            detected_from: None,
            project_path: Some("/home/user/secret".to_string()),
            domain: None,
            organization_id: "default".to_string(),
            owner_type: "user".to_string(),
            owner_id: "local".to_string(),
            engine_status: "idle".to_string(),
            engine_pid: None,
            created_at: "2026-04-05T00:00:00Z".to_string(),
            last_active: "2026-04-05T00:00:00Z".to_string(),
            metadata: "{}".to_string(),
        };
        store_reality(&conn, &reality).unwrap();

        // get_reality with wrong org_id should return None
        let none = get_reality(&conn, "r_sec", "wrong_org").unwrap();
        assert!(
            none.is_none(),
            "get_reality with wrong org_id must return None"
        );

        // get_reality_by_path with wrong org_id should return None
        let none = get_reality_by_path(&conn, "/home/user/secret", "wrong_org").unwrap();
        assert!(
            none.is_none(),
            "get_reality_by_path with wrong org_id must return None"
        );

        // update_reality_last_active with wrong org should NOT update
        let before = get_reality(&conn, "r_sec", "default").unwrap().unwrap();
        update_reality_last_active(&conn, "r_sec", "wrong_org").unwrap();
        let after = get_reality(&conn, "r_sec", "default").unwrap().unwrap();
        assert_eq!(
            before.last_active, after.last_active,
            "wrong org should not update last_active"
        );
    }

    #[test]
    fn test_reality_update_last_active() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let reality = Reality {
            id: "r2".to_string(),
            name: "test-project".to_string(),
            reality_type: "code".to_string(),
            detected_from: None,
            project_path: Some("/tmp/test".to_string()),
            domain: None,
            organization_id: "default".to_string(),
            owner_type: "user".to_string(),
            owner_id: "local".to_string(),
            engine_status: "idle".to_string(),
            engine_pid: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            last_active: "2026-01-01T00:00:00Z".to_string(),
            metadata: "{}".to_string(),
        };
        store_reality(&conn, &reality).unwrap();

        // Update last_active (with correct org)
        update_reality_last_active(&conn, "r2", "default").unwrap();

        // Verify it changed
        let updated = get_reality(&conn, "r2", "default").unwrap().unwrap();
        assert_ne!(
            updated.last_active, "2026-01-01T00:00:00Z",
            "last_active should have been updated"
        );
    }

    #[test]
    fn test_reality_upsert() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let reality = Reality {
            id: "r3".to_string(),
            name: "original".to_string(),
            reality_type: "code".to_string(),
            detected_from: None,
            project_path: None,
            domain: None,
            organization_id: "default".to_string(),
            owner_type: "user".to_string(),
            owner_id: "local".to_string(),
            engine_status: "idle".to_string(),
            engine_pid: None,
            created_at: "2026-04-05T00:00:00Z".to_string(),
            last_active: "2026-04-05T00:00:00Z".to_string(),
            metadata: "{}".to_string(),
        };
        store_reality(&conn, &reality).unwrap();

        // Upsert with updated name
        let updated = Reality {
            name: "updated".to_string(),
            ..reality
        };
        store_reality(&conn, &updated).unwrap();

        let got = get_reality(&conn, "r3", "default").unwrap().unwrap();
        assert_eq!(got.name, "updated");
    }

    #[test]
    fn test_reality_null_metadata_loads() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        // Insert a reality directly with NULL metadata to simulate legacy data
        conn.execute(
            "INSERT INTO reality (id, name, reality_type, organization_id, owner_type, owner_id, engine_status, created_at, last_active, metadata)
             VALUES ('r_null', 'null-meta', 'code', 'default', 'user', 'local', 'idle', datetime('now'), datetime('now'), NULL)",
            [],
        ).unwrap();

        // get_reality should handle NULL metadata via COALESCE
        let got = get_reality(&conn, "r_null", "default").unwrap().unwrap();
        assert_eq!(
            got.metadata, "{}",
            "NULL metadata should default to empty JSON object"
        );

        // list_realities should also handle it
        let realities = list_realities(&conn, "default").unwrap();
        let null_one = realities.iter().find(|r| r.id == "r_null").unwrap();
        assert_eq!(
            null_one.metadata, "{}",
            "NULL metadata in list should default to empty JSON object"
        );
    }

    #[test]
    fn test_visibility_parse_fail_closed() {
        use forge_core::types::Visibility;

        // Known values parse correctly
        assert_eq!(Visibility::parse("universal"), Visibility::Universal);
        assert_eq!(Visibility::parse("inherited"), Visibility::Inherited);
        assert_eq!(Visibility::parse("local"), Visibility::Local);
        assert_eq!(Visibility::parse("private"), Visibility::Private);

        // Unknown/garbage/empty values default to Private (fail-closed)
        assert_eq!(Visibility::parse("garbage"), Visibility::Private);
        assert_eq!(Visibility::parse(""), Visibility::Private);
        assert_eq!(Visibility::parse("PUBLIC"), Visibility::Private);
    }

    #[test]
    fn test_unique_reality_path_constraint() {
        let conn = open_db();
        ensure_defaults(&conn).unwrap();

        let reality1 = Reality {
            id: "r_dup1".to_string(),
            name: "first".to_string(),
            reality_type: "code".to_string(),
            detected_from: None,
            project_path: Some("/unique/path".to_string()),
            domain: None,
            organization_id: "default".to_string(),
            owner_type: "user".to_string(),
            owner_id: "local".to_string(),
            engine_status: "idle".to_string(),
            engine_pid: None,
            created_at: "2026-04-05T00:00:00Z".to_string(),
            last_active: "2026-04-05T00:00:00Z".to_string(),
            metadata: "{}".to_string(),
        };
        store_reality(&conn, &reality1).unwrap();

        // Attempting to insert a different reality with the same project_path should fail
        // (because store_reality uses INSERT OR REPLACE which keys on id, not project_path)
        // The unique index should prevent a raw INSERT with duplicate project_path
        let result = conn.execute(
            "INSERT INTO reality (id, name, reality_type, organization_id, owner_type, owner_id, engine_status, created_at, last_active, metadata, project_path)
             VALUES ('r_dup2', 'second', 'code', 'default', 'user', 'local', 'idle', datetime('now'), datetime('now'), '{}', '/unique/path')",
            [],
        );
        assert!(
            result.is_err(),
            "duplicate project_path should violate unique index"
        );

        // NULL project_path should be allowed for multiple realities
        let null1 = Reality {
            id: "r_null1".to_string(),
            project_path: None,
            ..reality1.clone()
        };
        let null2 = Reality {
            id: "r_null2".to_string(),
            name: "second-null".to_string(),
            project_path: None,
            ..reality1
        };
        store_reality(&conn, &null1).unwrap();
        store_reality(&conn, &null2).unwrap();
        // Both should exist
        let realities = list_realities(&conn, "default").unwrap();
        let null_count = realities
            .iter()
            .filter(|r| r.project_path.is_none())
            .count();
        assert!(
            null_count >= 2,
            "multiple realities with NULL project_path should be allowed"
        );
    }

    // ── Scoped Configuration Tests ──

    #[test]
    fn test_set_scoped_config_roundtrip() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "4096",
            false,
            None,
            "user",
        )
        .unwrap();
        let entry = get_scoped_config(&conn, "organization", "default", "max_tokens").unwrap();
        assert!(entry.is_some(), "config entry should exist after set");
        let entry = entry.unwrap();
        assert_eq!(entry.scope_type, "organization");
        assert_eq!(entry.scope_id, "default");
        assert_eq!(entry.key, "max_tokens");
        assert_eq!(entry.value, "4096");
        assert!(!entry.locked);
        assert!(entry.ceiling.is_none());
        assert_eq!(entry.set_by, "user");
    }

    #[test]
    fn test_set_scoped_config_upsert() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "4096",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "8192",
            true,
            Some(10000.0),
            "admin",
        )
        .unwrap();
        let entry = get_scoped_config(&conn, "organization", "default", "max_tokens")
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "8192", "upsert should update value");
        assert!(entry.locked, "upsert should update locked");
        assert_eq!(entry.ceiling, Some(10000.0), "upsert should update ceiling");
        assert_eq!(entry.set_by, "admin", "upsert should update set_by");
    }

    #[test]
    fn test_delete_scoped_config_exists() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "4096",
            false,
            None,
            "user",
        )
        .unwrap();
        let deleted = delete_scoped_config(&conn, "organization", "default", "max_tokens").unwrap();
        assert!(deleted, "delete should return true when entry exists");
        let entry = get_scoped_config(&conn, "organization", "default", "max_tokens").unwrap();
        assert!(entry.is_none(), "entry should be gone after delete");
    }

    #[test]
    fn test_delete_scoped_config_not_exists() {
        let conn = open_db();
        let deleted =
            delete_scoped_config(&conn, "organization", "default", "nonexistent").unwrap();
        assert!(
            !deleted,
            "delete should return false when entry does not exist"
        );
    }

    #[test]
    fn test_list_scoped_config_scope_filtering() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "4096",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "model",
            "gpt-4",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "max_tokens",
            "8192",
            false,
            None,
            "user",
        )
        .unwrap();

        let entries = list_scoped_config(&conn, "organization", "default").unwrap();
        assert_eq!(
            entries.len(),
            2,
            "should return only entries for that scope"
        );
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"max_tokens"));
        assert!(keys.contains(&"model"));

        let reality_entries = list_scoped_config(&conn, "reality", "r1").unwrap();
        assert_eq!(reality_entries.len(), 1);
    }

    #[test]
    fn test_resolve_no_scoped_config() {
        let conn = open_db();
        let result = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            None,
            None,
            None,
            Some("default"),
        )
        .unwrap();
        assert!(result.is_none(), "should return None when no config exists");
    }

    #[test]
    fn test_resolve_single_scope_level() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "3000",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            None,
            None,
            None,
            Some("default"),
        )
        .unwrap();
        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.value, "3000");
        assert_eq!(resolved.source_scope_type, "organization");
        assert_eq!(resolved.source_scope_id, "default");
        assert!(!resolved.locked);
        assert!(!resolved.ceiling_applied);
    }

    #[test]
    fn test_resolve_most_specific_wins() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "3000",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "max_tokens",
            "5000",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            Some("r1"),
            None,
            None,
            Some("default"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.value, "5000",
            "most-specific (reality) should win over org"
        );
        assert_eq!(resolved.source_scope_type, "reality");
    }

    #[test]
    fn test_resolve_locked_field() {
        let conn = open_db();
        // Org locks the value
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "3000",
            true,
            None,
            "admin",
        )
        .unwrap();
        // Reality tries to override
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "max_tokens",
            "5000",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            Some("r1"),
            None,
            None,
            Some("default"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.value, "3000",
            "locked org value should not be overridden by reality"
        );
        assert!(resolved.locked, "result should be marked as locked");
        assert_eq!(resolved.source_scope_type, "organization");
    }

    #[test]
    fn test_resolve_ceiling_enforcement() {
        let conn = open_db();
        // Org sets ceiling of 10000
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "3000",
            false,
            Some(10000.0),
            "admin",
        )
        .unwrap();
        // Reality sets 15000 (exceeds ceiling)
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "max_tokens",
            "15000",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            Some("r1"),
            None,
            None,
            Some("default"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.value, "10000",
            "value should be clamped to ceiling"
        );
        assert!(resolved.ceiling_applied, "ceiling_applied should be true");
        assert_eq!(
            resolved.source_scope_type, "reality",
            "source should still be reality (before clamping)"
        );
    }

    #[test]
    fn test_resolve_ceiling_plus_most_specific() {
        let conn = open_db();
        // Org ceiling = 10000
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "3000",
            false,
            Some(10000.0),
            "admin",
        )
        .unwrap();
        // User = 5000, under ceiling
        set_scoped_config(
            &conn,
            "user",
            "local",
            "max_tokens",
            "5000",
            false,
            None,
            "user",
        )
        .unwrap();
        // Reality = 8000, under ceiling
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "max_tokens",
            "8000",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "max_tokens",
            None,
            None,
            Some("r1"),
            Some("local"),
            None,
            Some("default"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.value, "8000",
            "reality is more specific than user, and under ceiling"
        );
        assert!(
            !resolved.ceiling_applied,
            "8000 < 10000 ceiling, not clamped"
        );
        assert_eq!(resolved.source_scope_type, "reality");
    }

    #[test]
    fn test_resolve_non_numeric_with_ceiling() {
        let conn = open_db();
        // Org sets ceiling (which only applies to numeric values)
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "model",
            "gpt-4",
            false,
            Some(10000.0),
            "admin",
        )
        .unwrap();
        // Reality overrides with non-numeric
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "model",
            "claude-opus-4",
            false,
            None,
            "user",
        )
        .unwrap();
        let resolved = resolve_scoped_config(
            &conn,
            "model",
            None,
            None,
            Some("r1"),
            None,
            None,
            Some("default"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.value, "claude-opus-4",
            "non-numeric value should not be clamped"
        );
        assert!(
            !resolved.ceiling_applied,
            "ceiling should be ignored for non-numeric values"
        );
    }

    #[test]
    fn test_resolve_effective_config_returns_all_keys() {
        let conn = open_db();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "max_tokens",
            "4096",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "organization",
            "default",
            "model",
            "gpt-4",
            false,
            None,
            "user",
        )
        .unwrap();
        set_scoped_config(
            &conn,
            "reality",
            "r1",
            "temperature",
            "0.7",
            false,
            None,
            "user",
        )
        .unwrap();

        let effective =
            resolve_effective_config(&conn, None, None, Some("r1"), None, None, Some("default"))
                .unwrap();
        assert_eq!(effective.len(), 3, "should have 3 resolved keys");
        assert!(effective.contains_key("max_tokens"));
        assert!(effective.contains_key("model"));
        assert!(effective.contains_key("temperature"));
        assert_eq!(effective["temperature"].source_scope_type, "reality");
    }

    #[test]
    fn test_validate_scope_type_valid() {
        assert!(validate_scope_type("organization"));
        assert!(validate_scope_type("team"));
        assert!(validate_scope_type("user"));
        assert!(validate_scope_type("reality"));
        assert!(validate_scope_type("agent"));
        assert!(validate_scope_type("session"));
    }

    #[test]
    fn test_validate_scope_type_invalid() {
        assert!(!validate_scope_type("invalid"));
        assert!(!validate_scope_type(""));
        assert!(!validate_scope_type("global"));
        assert!(!validate_scope_type("ORGANIZATION"));
    }

    // ── Portability Classification Tests ──

    #[test]
    fn test_classify_portability_universal_preference() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p1', 'preference', 'Always use UTC', 'timestamps', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'unknown')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 1);
        let port: String = conn
            .query_row("SELECT portability FROM memory WHERE id = 'p1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(port, "universal");
    }

    #[test]
    fn test_classify_portability_universal_principle_tag() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p2', 'decision', 'Fail loud', 'always fail loud', 0.9, 'active', '[\"principle\",\"quality\"]', datetime('now'), datetime('now'), 'unknown')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 1);
        let port: String = conn
            .query_row("SELECT portability FROM memory WHERE id = 'p2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(port, "universal");
    }

    #[test]
    fn test_classify_portability_reality_bound_file_path() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p3', 'decision', 'JWT config', 'Use RS256 for crates/daemon/src/auth.rs', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'unknown')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 1);
        let port: String = conn
            .query_row("SELECT portability FROM memory WHERE id = 'p3'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(port, "reality_bound");
    }

    #[test]
    fn test_classify_portability_reality_bound_port() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p4', 'decision', 'Vite config', 'Port 1420 is Vite dev server on :1420', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'unknown')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 1);
        let port: String = conn
            .query_row("SELECT portability FROM memory WHERE id = 'p4'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(port, "reality_bound");
    }

    #[test]
    fn test_classify_portability_domain_transferable_default() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p5', 'decision', 'Use TDD', 'Always write tests first', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'unknown')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 1);
        let port: String = conn
            .query_row("SELECT portability FROM memory WHERE id = 'p5'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(port, "domain_transferable");
    }

    #[test]
    fn test_classify_portability_skips_already_classified() {
        let conn = open_db();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, portability)
             VALUES ('p6', 'decision', 'Already classified', 'some content', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'universal')",
            [],
        ).unwrap();
        let count = classify_portability(&conn, 100).unwrap();
        assert_eq!(count, 0, "already-classified memories should be skipped");
    }

    // ── Bounded query tests ──

    #[test]
    fn test_decay_memories_respects_limit() {
        let conn = open_db();
        // Insert 15 old memories that would all be faded
        for i in 0..15 {
            conn.execute(
                &format!(
                    "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
                     VALUES ('decay_limit_{i}', 'decision', 'Old decision {i}', 'content', 0.9, 'active', '[]',
                             datetime('now', '-120 days'), datetime('now', '-120 days'))"
                ),
                [],
            ).unwrap();
        }

        // With limit=10, only 10 should be checked
        let (checked, faded) = decay_memories(&conn, 10).unwrap();
        assert_eq!(checked, 10, "limit=10 should check exactly 10 memories");
        assert_eq!(faded, 10, "all 10 checked should be faded (120 days old)");

        // 5 remain active
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 5,
            "5 memories should still be active after limited decay"
        );
    }

    #[test]
    fn test_semantic_dedup_respects_limit() {
        let conn = open_db();
        // Insert 15 identical memories that would all be deduped
        for i in 0..15 {
            conn.execute(
                &format!(
                    "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
                     VALUES ('dedup_limit_{i}', 'decision', 'Use Rust for daemon', 'Rust chosen for performance and safety', {conf}, 'active', '[]',
                             datetime('now'), datetime('now'))",
                    conf = 0.9 - (i as f64 * 0.01)
                ),
                [],
            ).unwrap();
        }

        // With limit=10, only first 10 are loaded and compared
        let merged = semantic_dedup(&conn, 10).unwrap();
        // 10 loaded → 1 survivor + 9 superseded
        assert_eq!(
            merged, 9,
            "limit=10 should merge 9 duplicates (keep 1 survivor out of 10)"
        );

        // 5 remain from the un-loaded batch + 1 survivor
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 6, "5 unloaded + 1 survivor = 6 active");
    }

    #[test]
    fn test_semantic_dedup_same_title_different_content_not_deduped() {
        let conn = open_db();

        // Two memories with similar (not identical) titles and completely different content.
        // These should NOT be deduped because neither title nor content overlap enough.
        insert_memory_for_dedup(
            &conn,
            "st-3",
            "decision",
            "Configure database connection pooling",
            "Use PgBouncer with transaction-level pooling. Set pool size to 2x CPU cores. \
             Monitor connection wait times and scale pool when p99 exceeds 50ms.",
            "myproject",
            0.85,
        );
        insert_memory_for_dedup(&conn, "st-4", "decision",
            "Configure deployment pipeline stages",
            "Use GitHub Actions with three stages: lint, test, deploy. Each stage runs in parallel \
             where possible. Deployment uses blue-green strategy with automatic rollback.",
            "myproject", 0.80);

        let merged = semantic_dedup(&conn, 1000).unwrap();
        assert_eq!(
            merged, 0,
            "memories with different titles AND different content must not be deduped"
        );

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 2, "both memories should remain active");
    }

    #[test]
    fn test_cleanup_orphaned_affects_edges() {
        let conn = open_db();

        // Seed a project root so cleanup can resolve relative paths.
        // Without this, relative paths are skipped (assumed valid) when project_roots is empty.
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        conn.execute(
            "INSERT INTO code_file (id, path, language, project, hash, indexed_at)
             VALUES ('cf-test', 'test.rs', 'rust', ?1, 'abc', datetime('now'))",
            params![&cwd],
        )
        .unwrap();

        // Create a memory
        let mem = Memory::new(
            MemoryType::Decision,
            "Auth decision",
            "Use JWT for auth in src/auth.rs",
        );
        remember(&conn, &mem).unwrap();

        // Create affects edges: one to a file that exists (Cargo.toml in project root),
        // one to a file that doesn't exist in any project root
        store_edge(&conn, &mem.id, "file:Cargo.toml", "affects", "{}").unwrap();
        store_edge(
            &conn,
            &mem.id,
            "file:src/nonexistent_file_xyz.rs",
            "affects",
            "{}",
        )
        .unwrap();

        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE edge_type = 'affects'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 2, "should have 2 affects edges");

        let removed = cleanup_orphaned_affects_edges(&conn).unwrap();
        assert_eq!(
            removed, 1,
            "should remove 1 orphaned edge (nonexistent file)"
        );

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE edge_type = 'affects'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 1,
            "should have 1 remaining affects edge (Cargo.toml exists)"
        );
    }

    #[test]
    fn test_cleanup_orphaned_affects_edges_no_project_roots() {
        let conn = open_db();
        let mem = Memory::new(MemoryType::Decision, "Test", "content");
        remember(&conn, &mem).unwrap();

        // With no project roots, relative paths should be preserved (not falsely deleted)
        store_edge(&conn, &mem.id, "file:src/unknown.rs", "affects", "{}").unwrap();

        let removed = cleanup_orphaned_affects_edges(&conn).unwrap();
        assert_eq!(
            removed, 0,
            "relative paths should NOT be deleted when no project roots known"
        );
    }
}
