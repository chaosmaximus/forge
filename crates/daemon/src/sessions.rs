use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// W1.32 (W28 review LOW-2 + LOW-3 + LOW-9): typed error surface for the
/// single-message lookup helper. Replaces the prior
/// `rusqlite::Result<Option<_>>` shape that conflated "no row", "many rows
/// matched the prefix", and "the input contained SQL LIKE wildcards / non-
/// ULID chars" into a single `Result<Option<_>>`. Each handler arm can now
/// emit a clean user-facing error without the `session_message_read failed:`
/// implementation prefix.
#[derive(Debug, thiserror::Error)]
pub enum MessageReadError {
    /// The caller-supplied ID/prefix contained characters outside Crockford
    /// base32 (`0-9A-Za-z` minus `I/L/O/U`). Real ULIDs never include `%`,
    /// `_`, or whitespace, so this rejection at the boundary prevents
    /// arbitrary SQL `LIKE` wildcards from leaking into the prefix path.
    #[error("invalid characters in message ID '{0}' — expected Crockford base32 (ULID format)")]
    InvalidChars(String),
    /// The prefix matched two or more rows. The user must type more
    /// characters to disambiguate.
    #[error("ambiguous message ID prefix '{prefix}' — type more characters (matches at least {count} rows)")]
    Ambiguous { prefix: String, count: usize },
    /// Underlying SQLite error.
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

/// W1.32 (W28 LOW-2): accept only Crockford base32 characters (ULID's
/// alphabet). ULIDs are case-insensitive on the wire but typically rendered
/// uppercase, so the validator allows both. Rejects `%` / `_` / whitespace
/// and any non-base32 char before the input ever reaches a `LIKE` clause.
fn is_valid_ulid_chars(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, 'I' | 'L' | 'O' | 'U' | 'i' | 'l' | 'o' | 'u'))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    /// A2A: capabilities this session advertises (JSON array string)
    pub capabilities: String,
    /// A2A: what the session is currently working on
    pub current_task: String,
}

/// A row from the session_message table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageRow {
    pub id: String,
    pub from_session: String,
    pub to_session: String,
    pub kind: String,
    pub topic: String,
    pub parts: String, // JSON
    pub status: String,
    pub in_reply_to: Option<String>,
    pub project: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

/// Register a new agent session.
///
/// Pre-release audit B-HIGH-1: now accepts `role` (org-role like
/// "CTO" / "Engineer" / "Reviewer"), persisted to the `session.role`
/// column that has existed since v2.1 but was never wired through.
///
/// Pre-release audit E-7 (deferred to follow-up): the underlying
/// upsert still uses INSERT OR REPLACE, which wipes lifecycle
/// columns (tool_use_count, budget_spent, working_set, etc.) on
/// re-registration of an existing id. Tracked as v0.6.1 fix.
#[allow(clippy::too_many_arguments)]
pub fn register_session(
    conn: &Connection,
    id: &str,
    agent: &str,
    project: Option<&str>,
    cwd: Option<&str>,
    capabilities: Option<&str>,
    current_task: Option<&str>,
    role: Option<&str>,
) -> rusqlite::Result<()> {
    let caps = capabilities.unwrap_or("[]");
    let task = current_task.unwrap_or("");
    // Derive project from CWD if not explicitly provided.
    // Walks up from CWD to find a project root (directory containing .git, Cargo.toml, etc.)
    // and uses its name. Falls back to CWD basename.
    let derived_project: Option<String> = match project {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => cwd.map(detect_project_name),
    };
    // Phase 2A-4d.3.1 #7: also set last_heartbeat_at = now so the reaper's
    // active → idle → ended lifecycle can track this session immediately.
    // Previously, sessions registered without a heartbeat sat in the
    // 24-hour orphan window — operators saw "many active sessions" because
    // these never transitioned through idle. Now: registration is itself a
    // heartbeat, and the next idle/ended transitions follow the configured
    // thresholds.
    conn.execute(
        "INSERT OR REPLACE INTO session (id, agent, project, cwd, started_at, status, capabilities, current_task, role, last_heartbeat_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), 'active', ?5, ?6, ?7, datetime('now'))",
        params![id, agent, derived_project, cwd, caps, task, role],
    )?;
    Ok(())
}

/// Detect project name from a CWD path by walking up to find the project root.
/// Looks for marker files (.git, Cargo.toml, package.json, pyproject.toml, go.mod).
/// Returns the name of the directory containing the marker, or CWD basename as fallback.
fn detect_project_name(cwd: &str) -> String {
    let markers = [
        ".git",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
    ];
    let mut dir = std::path::Path::new(cwd);

    // Walk up from CWD looking for project root markers (capped at 20 levels)
    let mut depth = 0;
    loop {
        if depth > 20 {
            break;
        }
        for marker in &markers {
            if dir.join(marker).exists() {
                return dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".into());
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => {
                dir = parent;
                depth += 1;
            }
            _ => break,
        }
    }

    // Fallback: CWD basename
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Mark a session as ended. Returns true if the session existed.
pub fn end_session(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let updated = conn.execute(
        "UPDATE session SET status = 'ended', ended_at = datetime('now') WHERE id = ?1 AND status IN ('active', 'idle')",
        params![id],
    )?;
    Ok(updated > 0)
}

/// List sessions. If active_only is true, only return active sessions.
pub fn list_sessions(conn: &Connection, active_only: bool) -> rusqlite::Result<Vec<Session>> {
    let sql = if active_only {
        "SELECT id, agent, project, cwd, started_at, ended_at, status, capabilities, current_task FROM session WHERE status IN ('active', 'idle') ORDER BY started_at DESC"
    } else {
        "SELECT id, agent, project, cwd, started_at, ended_at, status, capabilities, current_task FROM session ORDER BY started_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Session {
            id: row.get(0)?,
            agent: row.get(1)?,
            project: row.get(2)?,
            cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            status: row.get(6)?,
            capabilities: row.get(7)?,
            current_task: row.get(8)?,
        })
    })?;
    rows.collect()
}

/// Get the most recent active session ID for a given agent.
pub fn get_active_session_id(conn: &Connection, agent: &str) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT id FROM session WHERE agent = ?1 AND status IN ('active', 'idle') ORDER BY started_at DESC LIMIT 1",
        params![agent],
        |row| row.get(0),
    )
}

/// Return the session_id of the most recently activated session, regardless
/// of agent. Returns `None` if no session is active.
///
/// Used as a best-effort fallback by hook handlers whose Request variants
/// did not carry an explicit session_id (old clients, CLI convenience).
/// Added in SP1 review-fixup after the hardcoded `get_active_session_id(_, "cli")`
/// path was found to miss real Claude Code sessions (agent="claude-code").
pub fn get_latest_active_session_id(conn: &Connection) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM session WHERE status IN ('active', 'idle') ORDER BY started_at DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
}

/// Save working set (files touched + memories created) for a session.
/// Called at session-end to enable working set continuity.
pub fn save_working_set(conn: &Connection, session_id: &str) -> rusqlite::Result<()> {
    // Get session start time
    let started_at: String = conn
        .query_row(
            "SELECT started_at FROM session WHERE id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if started_at.is_empty() {
        return Ok(());
    }

    // Get files from perceptions created during this session
    let files: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT data FROM perception WHERE kind = 'file_change'
         AND created_at >= ?1 ORDER BY created_at DESC LIMIT 10",
        )
        .and_then(|mut stmt| stmt.query_map(params![started_at], |r| r.get(0))?.collect())
        .unwrap_or_default();

    // Get memories created during this session
    let memories: Vec<String> = conn
        .prepare("SELECT title FROM memory WHERE session_id = ?1 AND status = 'active' LIMIT 5")
        .and_then(|mut stmt| stmt.query_map(params![session_id], |r| r.get(0))?.collect())
        .unwrap_or_default();

    // Truncate individual items to prevent bloat (Codex fix: byte-bound working set)
    let files: Vec<String> = files
        .into_iter()
        .map(|f| f.chars().take(200).collect())
        .collect();
    let memories: Vec<String> = memories
        .into_iter()
        .map(|m| m.chars().take(100).collect())
        .collect();

    let mut working_set = serde_json::json!({
        "files": files,
        "memories": memories,
    })
    .to_string();

    // Hard cap at 4KB to prevent storage bloat
    if working_set.len() > 4096 {
        working_set.truncate(4096);
    }

    conn.execute(
        "UPDATE session SET working_set = ?1 WHERE id = ?2",
        params![working_set, session_id],
    )?;
    Ok(())
}

/// Get the working set from the last ended session for the same agent+project.
/// Used at session-start to restore context from the previous session.
pub fn get_last_working_set(
    conn: &Connection,
    agent: &str,
    project: Option<&str>,
) -> rusqlite::Result<String> {
    match project {
        Some(proj) => conn.query_row(
            "SELECT working_set FROM session WHERE agent = ?1 AND project = ?2 AND status = 'ended' AND working_set != ''
             ORDER BY ended_at DESC LIMIT 1",
            params![agent, proj],
            |row| row.get(0),
        ),
        None => conn.query_row(
            "SELECT working_set FROM session WHERE agent = ?1 AND status = 'ended' AND working_set != ''
             ORDER BY ended_at DESC LIMIT 1",
            params![agent],
            |row| row.get(0),
        ),
    }
}

/// Get a single session by ID.
pub fn get_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<Session>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent, project, cwd, started_at, ended_at, status, capabilities, current_task FROM session WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(Session {
            id: row.get(0)?,
            agent: row.get(1)?,
            project: row.get(2)?,
            cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            status: row.get(6)?,
            capabilities: row.get(7)?,
            current_task: row.get(8)?,
        }))
    } else {
        Ok(None)
    }
}

/// Increment tool_use_count for a session by a given delta.
/// Used by the extractor to track how many tool_use chunks were detected.
pub fn increment_tool_use_count(
    conn: &Connection,
    session_id: &str,
    delta: usize,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE session SET tool_use_count = tool_use_count + ?1 WHERE id = ?2",
        params![delta as i64, session_id],
    )?;
    Ok(())
}

/// End all active sessions matching an optional ID prefix.
/// If prefix is None, ends ALL active sessions.
/// Returns the number of sessions ended.
pub fn cleanup_sessions(conn: &Connection, prefix: Option<&str>) -> rusqlite::Result<usize> {
    match prefix {
        Some(pfx) => {
            let pattern = format!("{pfx}%");
            conn.execute(
                "UPDATE session SET status = 'ended', ended_at = datetime('now') WHERE status IN ('active', 'idle') AND id LIKE ?1",
                params![pattern],
            )
        }
        None => conn.execute(
            "UPDATE session SET status = 'ended', ended_at = datetime('now') WHERE status IN ('active', 'idle')",
            [],
        ),
    }
}

/// Auto-cleanup sessions that have been ACTIVE for more than 24 hours.
/// These are leaked sessions where the session-end hook never fired.
/// Called on daemon startup to prevent unbounded session accumulation.
pub fn cleanup_stale_sessions(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE session SET status = 'ended', ended_at = datetime('now') WHERE status IN ('active', 'idle') AND started_at < datetime('now', '-24 hours')",
        [],
    )
}

/// Update the heartbeat timestamp for a live (active OR idle) session.
///
/// Phase 2A-4d.3.1 #7 review B1: previously this filter was
/// `status = 'active'`, but with the new `active → idle → ended`
/// lifecycle, a heartbeat from a client whose session was just flipped
/// to `idle` by the reaper must NOT be silently dropped — that
/// zombifies the session (heartbeats land nowhere, and on the next
/// reaper pass it gets ended even though the client is alive).
///
/// Now: `IN ('active', 'idle')`. When the row is `'idle'`, this UPDATE
/// also flips it back to `'active'` atomically, restoring the live
/// state. Caller can detect the resume by checking the previous status
/// — but for simplicity we just emit `last_heartbeat_at = now` and
/// `status = 'active'` together; downstream observers see the session
/// re-enter the active set on the next read.
///
/// Returns true if updated (session exists and was live), false if the
/// session is missing or already ended.
pub fn update_heartbeat(conn: &Connection, session_id: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE session SET last_heartbeat_at = datetime('now'), status = 'active' \
         WHERE id = ?1 AND status IN ('active', 'idle')",
        params![session_id],
    )?;
    Ok(rows > 0)
}

/// Backfill project on memories that have session_id but no project.
/// Derives project from the linked session's project field.
pub fn backfill_project(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE memory SET project = (
            SELECT s.project FROM session s WHERE s.id = memory.session_id AND s.project IS NOT NULL
        ) WHERE (project IS NULL OR project = '') AND session_id != ''",
        [],
    )
}

// ── A2A FISP: Message CRUD ──

/// Send a message to another session (or broadcast to "*").
/// Returns the message ID.
#[allow(clippy::too_many_arguments)]
pub fn send_message(
    conn: &Connection,
    from_session: &str,
    to: &str,
    kind: &str,
    topic: &str,
    parts_json: &str,
    project: Option<&str>,
    timeout_secs: Option<u64>,
    meeting_id: Option<&str>,
) -> rusqlite::Result<String> {
    // Validate message size: parts_json must be under 64KB
    if parts_json.len() > 65536 {
        return Err(rusqlite::Error::InvalidParameterName(
            "message parts exceed 64KB limit".to_string(),
        ));
    }

    // Compute expires_at as a modifier string for SQLite datetime()
    // timeout_secs is u64 so no SQL injection risk, but we use parameterized query anyway
    let timeout_modifier = timeout_secs.map(|secs| format!("+{secs} seconds"));

    if to == "*" {
        // Broadcast: create one message per active session in the same project
        let sessions = match project {
            Some(proj) => {
                let mut stmt = conn.prepare(
                    "SELECT id FROM session WHERE status IN ('active', 'idle') AND project = ?1 AND id != ?2",
                )?;
                let rows =
                    stmt.query_map(params![proj, from_session], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<String>>>()?
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id FROM session WHERE status IN ('active', 'idle') AND id != ?1",
                )?;
                let rows = stmt.query_map(params![from_session], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<String>>>()?
            }
        };

        let broadcast_id = Ulid::new().to_string();
        for session_id in &sessions {
            let msg_id = Ulid::new().to_string();
            conn.execute(
                "INSERT INTO session_message (id, from_session, to_session, kind, topic, parts, status, project, created_at, expires_at, meeting_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, datetime('now'), CASE WHEN ?8 IS NOT NULL THEN datetime('now', ?8) ELSE NULL END, ?9)",
                params![msg_id, from_session, session_id, kind, topic, parts_json, project, timeout_modifier, meeting_id],
            )?;
        }
        Ok(broadcast_id)
    } else {
        let msg_id = Ulid::new().to_string();
        conn.execute(
            "INSERT INTO session_message (id, from_session, to_session, kind, topic, parts, status, project, created_at, expires_at, meeting_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, datetime('now'), CASE WHEN ?8 IS NOT NULL THEN datetime('now', ?8) ELSE NULL END, ?9)",
            params![msg_id, from_session, to, kind, topic, parts_json, project, timeout_modifier, meeting_id],
        )?;
        Ok(msg_id)
    }
}

/// Respond to a received request message.
/// Creates a NEW message with kind="response" and in_reply_to=message_id.
/// Updates the original message's status.
/// Returns false if the original message was not found.
pub fn respond_to_message(
    conn: &Connection,
    message_id: &str,
    from_session: &str,
    status: &str,
    parts_json: &str,
) -> rusqlite::Result<bool> {
    // Check original message exists and get its from_session (to send response back)
    let original = conn.query_row(
        "SELECT from_session, to_session, topic, project FROM session_message WHERE id = ?1",
        params![message_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    );

    match original {
        Ok((orig_from, orig_to, topic, project)) => {
            // Ownership check: only the original recipient can respond
            if orig_to != from_session {
                eprintln!("[a2a] WARN: session {from_session} tried to respond to message {message_id} addressed to {orig_to}");
                return Ok(false);
            }
            // Update the original message's status
            conn.execute(
                "UPDATE session_message SET status = ?1 WHERE id = ?2",
                params![status, message_id],
            )?;

            // Create a new response message
            let response_id = Ulid::new().to_string();
            conn.execute(
                "INSERT INTO session_message (id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at)
                 VALUES (?1, ?2, ?3, 'response', ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                params![response_id, from_session, orig_from, topic, parts_json, status, message_id, project],
            )?;
            Ok(true)
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e),
    }
}

/// List messages for a session (inbox). Limit capped at 1000.
pub fn list_messages(
    conn: &Connection,
    session_id: &str,
    status_filter: Option<&str>,
    limit: usize,
    offset: Option<usize>,
) -> rusqlite::Result<Vec<SessionMessageRow>> {
    let limit = limit.min(1000) as i64; // Cap at 1000, safe i64 cast
    let offset = offset.unwrap_or(0) as i64;
    let (sql, use_status) = match status_filter {
        Some(_) => (
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE to_session = ?1 AND status = ?2 ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
            true,
        ),
        None => (
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE to_session = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            false,
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<SessionMessageRow> {
        Ok(SessionMessageRow {
            id: row.get(0)?,
            from_session: row.get(1)?,
            to_session: row.get(2)?,
            kind: row.get(3)?,
            topic: row.get(4)?,
            parts: row.get(5)?,
            status: row.get(6)?,
            in_reply_to: row.get(7)?,
            project: row.get(8)?,
            created_at: row.get(9)?,
            delivered_at: row.get(10)?,
        })
    };
    let rows: Vec<rusqlite::Result<SessionMessageRow>> = if use_status {
        stmt.query_map(
            params![session_id, status_filter.unwrap_or(""), limit, offset],
            map_row,
        )?
        .collect()
    } else {
        stmt.query_map(params![session_id, limit, offset], map_row)?
            .collect()
    };
    rows.into_iter().collect()
}

/// W27 (F12+F14): look up a single message by exact ID or unambiguous prefix.
///
/// Resolution order:
/// 1. Exact match on `session_message.id`. If a row exists, return it.
/// 2. Prefix match (`id LIKE prefix || '%'`). If exactly one row matches,
///    return it. Multiple matches → `Err(NotFound)` with a clearer message
///    upstream so the user can disambiguate by typing more characters.
///
/// P3-4 W1.13 (W28 review HIGH-1): when `caller_session` is `Some`, both
/// the exact-match and prefix-match SQL are scoped to messages the
/// caller is a participant in (`to_session = ? OR from_session = ?`).
/// None preserves the W27 default-open semantics for backward compat.
///
/// The `messages` listing truncates IDs to 8 chars for display, so users
/// naturally copy-paste the prefix into `message-read`. Supporting prefix
/// lookup closes that surface gap without sacrificing exact-ID determinism.
pub fn read_message_by_id_or_prefix(
    conn: &Connection,
    id_or_prefix: &str,
    caller_session: Option<&str>,
) -> Result<Option<SessionMessageRow>, MessageReadError> {
    // W1.32 (W28 LOW-2): reject any input containing characters outside
    // ULID's Crockford base32 alphabet BEFORE binding to the LIKE clause.
    // `%` / `_` / whitespace are not valid ULID chars; rejecting them at
    // the boundary closes the unfiltered-wildcard surface.
    if !is_valid_ulid_chars(id_or_prefix) {
        return Err(MessageReadError::InvalidChars(id_or_prefix.to_string()));
    }
    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<SessionMessageRow> {
        Ok(SessionMessageRow {
            id: row.get(0)?,
            from_session: row.get(1)?,
            to_session: row.get(2)?,
            kind: row.get(3)?,
            topic: row.get(4)?,
            parts: row.get(5)?,
            status: row.get(6)?,
            in_reply_to: row.get(7)?,
            project: row.get(8)?,
            created_at: row.get(9)?,
            delivered_at: row.get(10)?,
        })
    };
    // Exact match first.
    let exact = match caller_session {
        Some(scope) => conn.query_row(
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE id = ?1 AND (to_session = ?2 OR from_session = ?2)",
            params![id_or_prefix, scope],
            map_row,
        ),
        None => conn.query_row(
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE id = ?1",
            params![id_or_prefix],
            map_row,
        ),
    };
    match exact {
        Ok(row) => return Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => {} // fall through to prefix
        Err(e) => return Err(e.into()),
    }
    // Prefix match — must be unambiguous.
    let rows: Vec<SessionMessageRow> = if let Some(scope) = caller_session {
        let mut stmt = conn.prepare(
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE id LIKE ?1 || '%' AND (to_session = ?2 OR from_session = ?2) LIMIT 2",
        )?;
        let collected: Vec<SessionMessageRow> = stmt
            .query_map(params![id_or_prefix, scope], map_row)?
            .filter_map(|r| r.ok())
            .collect();
        collected
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to, project, created_at, delivered_at
             FROM session_message WHERE id LIKE ?1 || '%' LIMIT 2",
        )?;
        let collected: Vec<SessionMessageRow> = stmt
            .query_map(params![id_or_prefix], map_row)?
            .filter_map(|r| r.ok())
            .collect();
        collected
    };
    match rows.len() {
        1 => Ok(Some(rows.into_iter().next().unwrap())),
        0 => Ok(None),
        n => Err(MessageReadError::Ambiguous {
            prefix: id_or_prefix.to_string(),
            count: n,
        }),
    }
}

/// Mark messages as read/consumed.
/// Only acks messages addressed TO the given session (ownership validation).
pub fn ack_messages(
    conn: &Connection,
    message_ids: &[String],
    caller_session: &str,
) -> rusqlite::Result<usize> {
    let mut count = 0;
    for id in message_ids {
        // Only ack messages addressed to the caller (ownership check)
        let updated = conn.execute(
            "UPDATE session_message SET status = 'read', delivered_at = datetime('now')
             WHERE id = ?1 AND to_session = ?2",
            params![id, caller_session],
        )?;
        count += updated;
    }
    Ok(count)
}

/// Admin/CLI ack: mark messages as read regardless of to_session.
/// Used when the CLI doesn't have a session context.
pub fn ack_messages_admin(conn: &Connection, message_ids: &[String]) -> rusqlite::Result<usize> {
    let mut count = 0;
    for id in message_ids {
        let updated = conn.execute(
            "UPDATE session_message SET status = 'read', delivered_at = datetime('now')
             WHERE id = ?1",
            params![id],
        )?;
        count += updated;
    }
    Ok(count)
}

// ── A2A Permission Model ──

/// Check if a message from one agent type to another is allowed.
/// In "open" mode, always returns true.
/// In "controlled" mode, checks the a2a_permission table.
/// Default: deny if no matching permission found in controlled mode.
pub fn check_a2a_permission(
    conn: &Connection,
    trust_mode: &str,
    from_agent: &str,
    to_agent: &str,
    from_project: Option<&str>,
    to_project: Option<&str>,
) -> bool {
    if trust_mode == "open" {
        return true;
    }

    // In controlled mode, check the permission table.
    // Match rules (priority order):
    // 1. Exact match (from_agent, to_agent, project scope)
    // 2. Wildcard match (from_agent="*" or to_agent="*")
    // 3. Project-scoped (from_project matches or NULL for any)
    // Default: deny
    let sql = "
        SELECT allowed FROM a2a_permission
        WHERE (from_agent = ?1 OR from_agent = '*')
          AND (to_agent = ?2 OR to_agent = '*')
          AND (from_project IS NULL OR from_project = ?3 OR ?3 IS NULL)
          AND (to_project IS NULL OR to_project = ?4 OR ?4 IS NULL)
        ORDER BY
            -- Prefer exact matches over wildcards
            CASE WHEN from_agent = ?1 AND to_agent = ?2 THEN 0
                 WHEN from_agent = ?1 OR to_agent = ?2 THEN 1
                 ELSE 2 END
        LIMIT 1
    ";

    conn.query_row(
        sql,
        params![from_agent, to_agent, from_project, to_project],
        |row| row.get::<_, i64>(0),
    )
    .map(|allowed| allowed != 0)
    .unwrap_or(false) // Default: deny if no matching permission
}

/// Grant an A2A permission. Returns the permission ID.
pub fn grant_a2a_permission(
    conn: &Connection,
    from_agent: &str,
    to_agent: &str,
    from_project: Option<&str>,
    to_project: Option<&str>,
) -> rusqlite::Result<String> {
    let id = Ulid::new().to_string();
    conn.execute(
        "INSERT INTO a2a_permission (id, from_agent, to_agent, from_project, to_project, allowed, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 'user', datetime('now'))",
        params![id, from_agent, to_agent, from_project, to_project],
    )?;
    Ok(id)
}

/// Revoke an A2A permission by ID. Returns true if the permission existed.
pub fn revoke_a2a_permission(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let deleted = conn.execute("DELETE FROM a2a_permission WHERE id = ?1", params![id])?;
    Ok(deleted > 0)
}

/// List all A2A permissions.
pub fn list_a2a_permissions(
    conn: &Connection,
) -> rusqlite::Result<Vec<forge_core::protocol::response::A2aPermission>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_agent, to_agent, from_project, to_project, allowed, created_by, created_at
         FROM a2a_permission ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(forge_core::protocol::response::A2aPermission {
            id: row.get(0)?,
            from_agent: row.get(1)?,
            to_agent: row.get(2)?,
            from_project: row.get(3)?,
            to_project: row.get(4)?,
            allowed: row.get::<_, i64>(5)? != 0,
            created_by: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Compile per-session KPIs from existing tables. Called at session end.
pub fn compile_session_kpis(
    conn: &Connection,
    session_id: &str,
) -> Option<forge_core::protocol::response::SessionKpis> {
    use std::collections::HashMap;

    // Session duration
    let duration_secs: u64 = conn.query_row(
        "SELECT CAST((julianday(COALESCE(ended_at, datetime('now'))) - julianday(started_at)) * 86400 AS INTEGER)
         FROM session WHERE id = ?1",
        params![session_id],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0).max(0) as u64;

    // Context injections from effectiveness table
    let (injection_count, chars_injected): (usize, usize) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(chars_injected), 0) FROM context_effectiveness WHERE session_id = ?1",
        params![session_id],
        |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? as usize)),
    ).unwrap_or((0, 0));

    // A2A messages sent/received
    let msgs_sent: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM session_message WHERE from_session = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    let msgs_received: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM session_message WHERE to_session = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    // Memories created during this session (via metadata containing session_id)
    // Escape LIKE metacharacters in session_id to prevent wildcard injection
    let escaped_sid = session_id
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let memories_created: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE metadata LIKE ?1 ESCAPE '\\'",
            params![format!("%{}%", escaped_sid)],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    // Hooks fired by type (from effectiveness table)
    let mut hooks_fired: HashMap<String, usize> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT hook_event, COUNT(*) FROM context_effectiveness WHERE session_id = ?1 GROUP BY hook_event"
    ) {
        if let Ok(rows) = stmt.query_map(params![session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        }) {
            for row in rows.flatten() {
                hooks_fired.insert(row.0, row.1);
            }
        }
    }

    Some(forge_core::protocol::response::SessionKpis {
        session_duration_secs: duration_secs,
        context_injections: injection_count,
        context_chars_injected: chars_injected,
        a2a_messages_sent: msgs_sent,
        a2a_messages_received: msgs_received,
        memories_created,
        hooks_fired,
    })
}

/// Count active sessions.
pub fn count_active_sessions(conn: &Connection) -> rusqlite::Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session WHERE status IN ('active', 'idle')",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Count total session messages.
pub fn count_all_messages(conn: &Connection) -> rusqlite::Result<usize> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM session_message", [], |row| row.get(0))?;
    Ok(count as usize)
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
    fn test_get_latest_active_session_id_returns_most_recent() {
        let conn = setup();

        // No active sessions → None.
        assert_eq!(get_latest_active_session_id(&conn).unwrap(), None);

        register_session(&conn, "sess-1", "cli", None, None, None, None, None).unwrap();
        // Ensure distinct started_at (SQLite datetime('now') has 1s resolution
        // in some builds; tick the clock explicitly so the ORDER BY is
        // deterministic on all platforms).
        conn.execute(
            "UPDATE session SET started_at = datetime('now', '-10 seconds') WHERE id = 'sess-1'",
            [],
        )
        .unwrap();
        register_session(&conn, "sess-2", "claude-code", None, None, None, None, None).unwrap();

        assert_eq!(
            get_latest_active_session_id(&conn).unwrap(),
            Some("sess-2".to_string()),
            "the most recently started active session should win, regardless of agent"
        );

        // Ending the most-recent one falls back to the older active session.
        end_session(&conn, "sess-2").unwrap();
        assert_eq!(
            get_latest_active_session_id(&conn).unwrap(),
            Some("sess-1".to_string()),
        );
    }

    #[test]
    fn test_register_and_list() {
        let conn = setup();
        register_session(
            &conn,
            "s1",
            "claude-code",
            Some("forge"),
            Some("/project"),
            None,
            None,
            None,
        )
        .unwrap();
        register_session(&conn, "s2", "cline", None, None, None, None, None).unwrap();

        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 2);
        let agents: Vec<&str> = active.iter().map(|s| s.agent.as_str()).collect();
        assert!(agents.contains(&"claude-code"));
        assert!(agents.contains(&"cline"));
    }

    #[test]
    fn test_end_session() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();

        assert!(end_session(&conn, "s1").unwrap());
        assert!(!end_session(&conn, "s1").unwrap()); // already ended

        let active = list_sessions(&conn, true).unwrap();
        assert!(active.is_empty());

        let all = list_sessions(&conn, false).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, "ended");
        assert!(all[0].ended_at.is_some());
    }

    #[test]
    fn test_register_duplicate_updates() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("proj1"), None, None, None, None).unwrap();
        register_session(&conn, "s1", "claude-code", Some("proj2"), None, None, None, None).unwrap();

        let all = list_sessions(&conn, false).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].project.as_deref(), Some("proj2"));
    }

    #[test]
    fn test_backfill_project() {
        let conn = setup();

        // Create a session with a project
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();

        // Create a memory with session_id but no project
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, session_id)
             VALUES ('m1', 'decision', 'Test Decision', 'content', 0.9, 'active', '', '[]', datetime('now'), datetime('now'), 's1')",
            [],
        ).unwrap();

        // Verify memory has no project
        let project: String = conn
            .query_row(
                "SELECT COALESCE(project, '') FROM memory WHERE id = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project, "");

        // Run backfill
        let updated = backfill_project(&conn).unwrap();
        assert_eq!(updated, 1);

        // Verify memory now has the session's project
        let project: String = conn
            .query_row("SELECT project FROM memory WHERE id = 'm1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(project, "forge");
    }

    #[test]
    fn test_backfill_project_no_session() {
        let conn = setup();

        // Create a memory with no session_id
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, session_id)
             VALUES ('m1', 'decision', 'Test', 'content', 0.9, 'active', '', '[]', datetime('now'), datetime('now'), '')",
            [],
        ).unwrap();

        // Backfill should not touch memories without session_id
        let updated = backfill_project(&conn).unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn test_cleanup_stale_sessions() {
        let conn = setup();

        // Create a session with a started_at timestamp >24h ago
        conn.execute(
            "INSERT INTO session (id, agent, project, cwd, started_at, status) VALUES ('stale1', 'claude-code', 'proj', NULL, datetime('now', '-25 hours'), 'active')",
            [],
        ).unwrap();

        // Create a recent active session (should NOT be cleaned up)
        register_session(
            &conn,
            "recent1",
            "claude-code",
            Some("proj"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Create an already-ended old session (should NOT be touched)
        conn.execute(
            "INSERT INTO session (id, agent, project, cwd, started_at, ended_at, status) VALUES ('ended1', 'claude-code', 'proj', NULL, datetime('now', '-48 hours'), datetime('now', '-47 hours'), 'ended')",
            [],
        ).unwrap();

        // Verify initial state: 2 active sessions
        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 2);

        // Run cleanup
        let cleaned = cleanup_stale_sessions(&conn).unwrap();
        assert_eq!(cleaned, 1, "should clean up exactly 1 stale session");

        // Verify: only recent session remains active
        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "recent1");

        // Verify: stale session is now ended
        let stale = get_session(&conn, "stale1").unwrap().unwrap();
        assert_eq!(stale.status, "ended");
        assert!(stale.ended_at.is_some());

        // Verify: already-ended session was not modified
        let ended = get_session(&conn, "ended1").unwrap().unwrap();
        assert_eq!(ended.status, "ended");
    }

    #[test]
    fn test_cleanup_stale_sessions_none_to_clean() {
        let conn = setup();

        // Only recent sessions
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", None, None, None, None, None).unwrap();

        let cleaned = cleanup_stale_sessions(&conn).unwrap();
        assert_eq!(cleaned, 0, "should not clean up any recent sessions");

        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_get_session() {
        let conn = setup();
        register_session(
            &conn,
            "s1",
            "claude-code",
            Some("forge"),
            Some("/cwd"),
            None,
            None,
            None,
        )
        .unwrap();

        let s = get_session(&conn, "s1").unwrap().unwrap();
        assert_eq!(s.agent, "claude-code");
        assert_eq!(s.project.as_deref(), Some("forge"));
        assert_eq!(s.cwd.as_deref(), Some("/cwd"));

        assert!(get_session(&conn, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_tool_use_count_tracking() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();

        // Initial count should be 0
        let count: i64 = conn
            .query_row(
                "SELECT tool_use_count FROM session WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "initial tool_use_count should be 0");

        // Increment by 3
        increment_tool_use_count(&conn, "s1", 3).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT tool_use_count FROM session WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3, "tool_use_count should be 3 after increment");

        // Increment again by 2
        increment_tool_use_count(&conn, "s1", 2).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT tool_use_count FROM session WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 5, "tool_use_count should accumulate to 5");
    }

    #[test]
    fn test_tool_use_count_nonexistent_session() {
        let conn = setup();

        // Incrementing a non-existent session should not error (just 0 rows updated)
        let result = increment_tool_use_count(&conn, "nonexistent", 1);
        assert!(result.is_ok(), "should not error on nonexistent session");
    }

    #[test]
    fn test_cross_session_perception() {
        let conn = setup();

        // Register two sessions
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();

        // Verify there are 2 active sessions
        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 2, "should have 2 active sessions");

        // Simulate cross-session perception (as handler.rs does for decisions)
        let perception = forge_core::types::manas::Perception {
            id: format!("xsession-{}", ulid::Ulid::new()),
            kind: forge_core::types::manas::PerceptionKind::CrossSessionDecision,
            data: "Another session stored decision: Use JWT for auth".to_string(),
            severity: forge_core::types::manas::Severity::Info,
            project: Some("forge".to_string()),
            created_at: forge_core::time::now_iso(),
            expires_at: Some(forge_core::time::now_offset(600)),
            consumed: false,
        };
        crate::db::manas::store_perception(&conn, &perception).unwrap();

        // Verify perception was stored
        let perceptions = crate::db::manas::list_unconsumed_perceptions(&conn, None, None).unwrap();
        let cross_session = perceptions
            .iter()
            .find(|p| p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision);
        assert!(
            cross_session.is_some(),
            "cross-session perception should be stored"
        );
        assert!(
            cross_session.unwrap().data.contains("JWT"),
            "perception should contain the decision title"
        );
    }

    #[test]
    fn test_cleanup_sessions_with_prefix() {
        let conn = setup();
        register_session(
            &conn,
            "hook-test-1",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        register_session(
            &conn,
            "hook-test-2",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        register_session(
            &conn,
            "real-session-1",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Cleanup only hook-test sessions
        let ended = cleanup_sessions(&conn, Some("hook-test")).unwrap();
        assert_eq!(ended, 2, "should end 2 hook-test sessions");

        // Real session still active
        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "real-session-1");
    }

    #[test]
    fn test_cleanup_sessions_all() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", None, None, None, None, None).unwrap();

        let ended = cleanup_sessions(&conn, None).unwrap();
        assert_eq!(ended, 2, "should end all active sessions");

        let active = list_sessions(&conn, true).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn test_cleanup_sessions_no_match() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();

        let ended = cleanup_sessions(&conn, Some("nonexistent")).unwrap();
        assert_eq!(ended, 0, "should not end any sessions");

        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 1);
    }

    // ── A2A Message CRUD Tests ──

    #[test]
    fn test_send_and_list_message() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();

        let msg_id = send_message(
            &conn,
            "s1",
            "s2",
            "notification",
            "file_changed",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();
        assert!(!msg_id.is_empty());

        let messages = list_messages(&conn, "s2", None, 10, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from_session, "s1");
        assert_eq!(messages[0].to_session, "s2");
        assert_eq!(messages[0].kind, "notification");
        assert_eq!(messages[0].topic, "file_changed");
        assert_eq!(messages[0].status, "pending");
    }

    #[test]
    fn test_broadcast_message() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s3", "codex", Some("forge"), None, None, None, None).unwrap();

        send_message(
            &conn,
            "s1",
            "*",
            "notification",
            "schema_changed",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();

        // s2 and s3 should each get a message, s1 (sender) should not
        let s2_msgs = list_messages(&conn, "s2", None, 10, None).unwrap();
        assert_eq!(s2_msgs.len(), 1, "s2 should receive broadcast");
        let s3_msgs = list_messages(&conn, "s3", None, 10, None).unwrap();
        assert_eq!(s3_msgs.len(), 1, "s3 should receive broadcast");
        let s1_msgs = list_messages(&conn, "s1", None, 10, None).unwrap();
        assert_eq!(s1_msgs.len(), 0, "sender should not receive own broadcast");
    }

    #[test]
    fn test_respond_to_message() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();

        let msg_id = send_message(
            &conn,
            "s1",
            "s2",
            "request",
            "review_code",
            "[]",
            None,
            None,
            None,
        )
        .unwrap();

        let found = respond_to_message(
            &conn,
            &msg_id,
            "s2",
            "completed",
            r#"[{"kind":"text","text":"LGTM"}]"#,
        )
        .unwrap();
        assert!(found, "should find and respond to original message");

        // Original message status should be updated
        let msgs = list_messages(&conn, "s2", Some("completed"), 10, None).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].status, "completed");
    }

    #[test]
    fn test_ack_messages() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();

        let id1 = send_message(
            &conn,
            "api",
            "s1",
            "notification",
            "t1",
            "[]",
            None,
            None,
            None,
        )
        .unwrap();
        let id2 = send_message(
            &conn,
            "api",
            "s1",
            "notification",
            "t2",
            "[]",
            None,
            None,
            None,
        )
        .unwrap();

        let acked = ack_messages(&conn, &[id1.clone(), id2.clone()], "s1").unwrap();
        assert_eq!(acked, 2);

        // Messages should now be "read"
        let pending = list_messages(&conn, "s1", Some("pending"), 10, None).unwrap();
        assert_eq!(pending.len(), 0, "no pending messages after ack");
        let read = list_messages(&conn, "s1", Some("read"), 10, None).unwrap();
        assert_eq!(read.len(), 2, "both messages should be read");
    }

    #[test]
    fn test_respond_to_nonexistent_message() {
        let conn = setup();
        let found = respond_to_message(&conn, "nonexistent", "s1", "completed", "[]").unwrap();
        assert!(!found, "should not find nonexistent message");
    }

    #[test]
    fn test_register_session_with_capabilities() {
        let conn = setup();
        register_session(
            &conn,
            "s1",
            "claude-code",
            Some("forge"),
            None,
            Some(r#"["code_edit","bash"]"#),
            Some("Building A2A"),
            None,
        )
        .unwrap();

        let sessions = list_sessions(&conn, true).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].capabilities, r#"["code_edit","bash"]"#);
        assert_eq!(sessions[0].current_task, "Building A2A");
    }

    #[test]
    fn test_list_messages_with_status_filter() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();

        send_message(
            &conn,
            "api",
            "s1",
            "notification",
            "t1",
            "[]",
            None,
            None,
            None,
        )
        .unwrap();
        let id2 = send_message(
            &conn,
            "api",
            "s1",
            "notification",
            "t2",
            "[]",
            None,
            None,
            None,
        )
        .unwrap();
        ack_messages(&conn, &[id2], "s1").unwrap();

        let pending = list_messages(&conn, "s1", Some("pending"), 10, None).unwrap();
        assert_eq!(pending.len(), 1, "should have 1 pending message");
        let read = list_messages(&conn, "s1", Some("read"), 10, None).unwrap();
        assert_eq!(read.len(), 1, "should have 1 read message");
        let all = list_messages(&conn, "s1", None, 10, None).unwrap();
        assert_eq!(all.len(), 2, "should have 2 total messages");
    }

    #[test]
    fn test_list_messages_with_offset() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None, None, None, None).unwrap();

        // Insert 5 messages so we can page through them
        for i in 0..5 {
            send_message(
                &conn,
                "api",
                "s1",
                "notification",
                &format!("topic_{i}"),
                "[]",
                None,
                None,
                None,
            )
            .unwrap();
        }

        // All 5 with no offset
        let all = list_messages(&conn, "s1", None, 10, None).unwrap();
        assert_eq!(all.len(), 5, "should have 5 total messages");

        // First page: limit 2, offset 0
        let page1 = list_messages(&conn, "s1", None, 2, Some(0)).unwrap();
        assert_eq!(page1.len(), 2, "page 1 should have 2 messages");

        // Second page: limit 2, offset 2
        let page2 = list_messages(&conn, "s1", None, 2, Some(2)).unwrap();
        assert_eq!(page2.len(), 2, "page 2 should have 2 messages");

        // Third page: limit 2, offset 4
        let page3 = list_messages(&conn, "s1", None, 2, Some(4)).unwrap();
        assert_eq!(page3.len(), 1, "page 3 should have 1 message");

        // Pages should not overlap — all ids should be unique
        let mut all_ids: Vec<String> = page1
            .iter()
            .chain(page2.iter())
            .chain(page3.iter())
            .map(|m| m.id.clone())
            .collect();
        let total = all_ids.len();
        all_ids.sort();
        all_ids.dedup();
        assert_eq!(
            all_ids.len(),
            total,
            "paginated ids must be unique (no overlap)"
        );

        // Offset past the end should return empty
        let empty = list_messages(&conn, "s1", None, 10, Some(100)).unwrap();
        assert_eq!(empty.len(), 0, "offset past end should return empty");

        // Offset with status filter
        let pending_page = list_messages(&conn, "s1", Some("pending"), 2, Some(2)).unwrap();
        assert_eq!(pending_page.len(), 2, "status-filtered offset should work");
    }

    // ── A2A Permission Tests ──

    #[test]
    fn test_open_mode_allows_all() {
        let conn = setup();
        // In open mode, any message should be allowed regardless of agents/projects
        assert!(check_a2a_permission(
            &conn,
            "open",
            "claude-code",
            "cline",
            None,
            None
        ));
        assert!(check_a2a_permission(
            &conn,
            "open",
            "unknown-agent",
            "another-agent",
            Some("proj"),
            Some("proj2")
        ));
    }

    #[test]
    fn test_controlled_mode_denies_without_permission() {
        let conn = setup();
        // In controlled mode with no permissions, all messages should be denied
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            Some("forge"),
            Some("forge")
        ));
    }

    #[test]
    fn test_controlled_mode_allows_with_permission() {
        let conn = setup();
        // Grant permission from claude-code to cline
        let id = grant_a2a_permission(&conn, "claude-code", "cline", None, None).unwrap();
        assert!(!id.is_empty());

        // Should now be allowed
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));

        // Reverse direction should still be denied (permission is directional)
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "cline",
            "claude-code",
            None,
            None
        ));
    }

    #[test]
    fn test_wildcard_permission() {
        let conn = setup();
        // Grant wildcard: any agent can message cline
        grant_a2a_permission(&conn, "*", "cline", None, None).unwrap();

        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "codex",
            "cline",
            None,
            None
        ));
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "unknown",
            "cline",
            None,
            None
        ));

        // But messages TO other agents should still be denied
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "codex",
            None,
            None
        ));
    }

    #[test]
    fn test_wildcard_to_agent() {
        let conn = setup();
        // Grant: claude-code can message ANY agent
        grant_a2a_permission(&conn, "claude-code", "*", None, None).unwrap();

        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "codex",
            None,
            None
        ));
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "anything",
            None,
            None
        ));

        // Other agents still denied
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "cline",
            "codex",
            None,
            None
        ));
    }

    #[test]
    fn test_grant_and_revoke() {
        let conn = setup();
        // Grant permission
        let id = grant_a2a_permission(&conn, "claude-code", "cline", None, None).unwrap();
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));

        // Revoke it
        let found = revoke_a2a_permission(&conn, &id).unwrap();
        assert!(found, "should find and revoke the permission");

        // Should now be denied again
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));

        // Revoking again should return false
        let found = revoke_a2a_permission(&conn, &id).unwrap();
        assert!(!found, "should not find already-revoked permission");
    }

    #[test]
    fn test_list_a2a_permissions() {
        let conn = setup();
        assert!(list_a2a_permissions(&conn).unwrap().is_empty());

        grant_a2a_permission(&conn, "claude-code", "cline", None, None).unwrap();
        grant_a2a_permission(&conn, "*", "*", Some("forge"), Some("forge")).unwrap();

        let perms = list_a2a_permissions(&conn).unwrap();
        assert_eq!(perms.len(), 2);

        // All should be allowed=true
        assert!(perms.iter().all(|p| p.allowed));
    }

    #[test]
    fn test_project_scoped_permission() {
        let conn = setup();
        // Grant permission only for forge project
        grant_a2a_permission(&conn, "claude-code", "cline", Some("forge"), Some("forge")).unwrap();

        // Should be allowed for forge project
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            Some("forge"),
            Some("forge")
        ));

        // Should be allowed when project is NULL (NULL matches anything in the query)
        assert!(check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            None,
            None
        ));

        // Exact project mismatch should be denied
        // The permission has from_project="forge", but caller says from_project="other"
        // Query: (from_project IS NULL OR from_project = ?3 OR ?3 IS NULL)
        // from_project="forge", ?3="other" -> false OR false OR false -> denied
        assert!(!check_a2a_permission(
            &conn,
            "controlled",
            "claude-code",
            "cline",
            Some("other"),
            Some("forge")
        ));
    }

    // ── Count helpers ──

    #[test]
    fn test_count_active_sessions() {
        let conn = setup();
        assert_eq!(count_active_sessions(&conn).unwrap(), 0);

        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();
        assert_eq!(count_active_sessions(&conn).unwrap(), 2);

        end_session(&conn, "s1").unwrap();
        assert_eq!(count_active_sessions(&conn).unwrap(), 1);
    }

    #[test]
    fn test_count_all_messages() {
        let conn = setup();
        assert_eq!(count_all_messages(&conn).unwrap(), 0);

        register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None, None).unwrap();
        register_session(&conn, "s2", "cline", Some("forge"), None, None, None, None).unwrap();

        send_message(
            &conn,
            "s1",
            "s2",
            "notification",
            "t1",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();
        send_message(
            &conn,
            "s1",
            "s2",
            "notification",
            "t2",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(count_all_messages(&conn).unwrap(), 2);
    }

    #[test]
    fn p3_4_w1_13_read_message_caller_scope_filters_foreign_messages() {
        // P3-4 W1.13 (W28 review HIGH-1): read_message_by_id_or_prefix
        // gains an optional `caller_session` parameter. When None, the
        // function preserves the W27 default-open semantics (any caller
        // can read any message). When Some, it scopes the lookup to
        // messages where the caller is a participant
        // (`to_session = ? OR from_session = ?`), so a caller cannot
        // read a message belonging to a different session pair.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Seed three messages: one between s1↔s2, one between s3↔s4
        // (foreign to s1+s2), and one between s1↔s3 (s1 still
        // participates).
        let id_a = send_message(
            &conn,
            "s1",
            "s2",
            "request",
            "topic-a",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();
        let id_b = send_message(
            &conn,
            "s3",
            "s4",
            "request",
            "topic-b",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();
        let id_c = send_message(
            &conn,
            "s1",
            "s3",
            "request",
            "topic-c",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();

        // Default-open (None) — caller can read any message regardless
        // of participation. Backward-compat path.
        let r_a = read_message_by_id_or_prefix(&conn, &id_a, None)
            .unwrap()
            .expect("default-open must find s1↔s2 by id");
        assert_eq!(r_a.id, id_a);
        let r_b = read_message_by_id_or_prefix(&conn, &id_b, None)
            .unwrap()
            .expect("default-open must find s3↔s4 by id");
        assert_eq!(r_b.id, id_b);

        // Scoped (Some("s1")) — caller can read only messages they
        // sent or received. id_a (s1↔s2) and id_c (s1↔s3) are visible;
        // id_b (s3↔s4) is foreign.
        let r_a_scoped = read_message_by_id_or_prefix(&conn, &id_a, Some("s1"))
            .unwrap()
            .expect("scoped: s1 sees its own outgoing message");
        assert_eq!(r_a_scoped.id, id_a);
        let r_c_scoped = read_message_by_id_or_prefix(&conn, &id_c, Some("s1"))
            .unwrap()
            .expect("scoped: s1 sees its other outgoing message");
        assert_eq!(r_c_scoped.id, id_c);
        let r_b_scoped = read_message_by_id_or_prefix(&conn, &id_b, Some("s1")).unwrap();
        assert!(
            r_b_scoped.is_none(),
            "scoped: s1 must NOT see s3↔s4 message (foreign pair)"
        );

        // Prefix lookup also honors the scope. The 3 seeded messages
        // share the leading ULID time component; the 18-char prefix
        // crosses into the random-bytes section so it's unique to id_b.
        // Scoped("s1") returns None even on an unambiguous prefix
        // because s1 is not a participant.
        let prefix_b: &str = &id_b[..18];
        let prefix_b_scoped = read_message_by_id_or_prefix(&conn, prefix_b, Some("s1")).unwrap();
        assert!(
            prefix_b_scoped.is_none(),
            "scoped prefix lookup must filter foreign matches"
        );
        // Same prefix without scope finds the message (backward compat).
        let prefix_b_open = read_message_by_id_or_prefix(&conn, prefix_b, None).unwrap();
        assert!(
            prefix_b_open.is_some(),
            "default-open prefix lookup must still find the foreign message"
        );

        // Recipient-side scope: s2 sees the message it received.
        let r_a_s2 = read_message_by_id_or_prefix(&conn, &id_a, Some("s2"))
            .unwrap()
            .expect("scoped: s2 sees the incoming message it received from s1");
        assert_eq!(r_a_s2.id, id_a);

        // Adversarial: a session entirely uninvolved (s5) gets nothing.
        let r_uninvolved = read_message_by_id_or_prefix(&conn, &id_a, Some("s5")).unwrap();
        assert!(
            r_uninvolved.is_none(),
            "scoped: a session that is neither sender nor recipient sees nothing"
        );
    }

    // P3-4 W1.32 (W28 review LOW-2 + LOW-3 + LOW-9): the four behavioral
    // branches of `read_message_by_id_or_prefix` are now pinned by tests
    // (a) exact, (b) prefix-1, (c) zero-match, (d) ambiguous, plus the
    // (e) invalid-chars guard at the boundary. These were previously
    // verified only by live walk-through; LOW-9 tracked the gap.

    #[test]
    fn w1_32_w28_low9_read_message_branch_a_exact_match_returns_row() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let id = send_message(&conn, "s1", "s2", "request", "t", "[]", None, None, None).unwrap();
        let row = read_message_by_id_or_prefix(&conn, &id, None)
            .expect("exact branch must not error")
            .expect("exact branch must return Some(row)");
        assert_eq!(row.id, id);
        assert_eq!(row.from_session, "s1");
        assert_eq!(row.to_session, "s2");
    }

    #[test]
    fn w1_32_w28_low9_read_message_branch_b_unambiguous_prefix_returns_row() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let id = send_message(&conn, "s1", "s2", "request", "t", "[]", None, None, None).unwrap();
        // Take a 20-char prefix — long enough to be unambiguous against a
        // single seeded row but short enough to exercise the LIKE branch.
        let prefix: &str = &id[..20];
        let row = read_message_by_id_or_prefix(&conn, prefix, None)
            .expect("prefix branch must not error")
            .expect("unambiguous prefix must return Some(row)");
        assert_eq!(row.id, id, "prefix lookup returns the matching row");
    }

    #[test]
    fn w1_32_w28_low9_read_message_branch_c_zero_matches_returns_none() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // No rows seeded. A well-formed-but-unknown ULID returns Ok(None),
        // not Err — the handler renders this as "message not found".
        let r = read_message_by_id_or_prefix(&conn, "01HZZZZZZZZZZZZZZZZZZZZZZZ", None)
            .expect("zero-match must be Ok(None), not Err");
        assert!(r.is_none(), "zero matches yields None");
    }

    #[test]
    fn w1_32_w28_low9_read_message_branch_d_ambiguous_prefix_returns_err() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // Two seeded ULIDs share a leading time prefix (ULIDs are
        // time-ordered; messages sent in rapid succession share many
        // leading chars). A 4-char prefix is virtually guaranteed to
        // match both.
        let _id_1 = send_message(&conn, "s1", "s2", "request", "t1", "[]", None, None, None)
            .unwrap();
        let _id_2 = send_message(&conn, "s1", "s2", "request", "t2", "[]", None, None, None)
            .unwrap();
        let err = read_message_by_id_or_prefix(&conn, "01", None)
            .expect_err("ambiguous prefix must yield Err::Ambiguous");
        match err {
            MessageReadError::Ambiguous { ref prefix, count } => {
                assert_eq!(prefix, "01");
                assert!(count >= 2, "ambiguous count must be ≥2, got {count}");
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn w1_32_w28_low2_read_message_rejects_like_wildcards_at_boundary() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // Seed one row, then attempt lookups with wildcards. The validator
        // rejects each at the boundary BEFORE the LIKE clause sees them —
        // so `%` no longer matches every message in the database.
        let _id = send_message(&conn, "s1", "s2", "request", "t", "[]", None, None, None).unwrap();
        for bad in ["%", "_", "01%", " 01ABCDEF", "01ABCDEF ", "lowercase-l-rejected"] {
            let err = read_message_by_id_or_prefix(&conn, bad, None)
                .expect_err(&format!("input '{bad}' must be rejected as InvalidChars"));
            assert!(
                matches!(err, MessageReadError::InvalidChars(_)),
                "expected InvalidChars for '{bad}', got {err:?}"
            );
        }
    }
}
