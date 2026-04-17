//! sync.rs — Hybrid Logical Clock + memory sync protocol
//!
//! HLC format: "{wall_ms}-{counter:010}-{node_id}"
//! - wall_ms: milliseconds since epoch
//! - counter: monotonic counter for same-millisecond events, zero-padded to 10 digits
//!   so that lexicographic ordering matches numeric ordering (e.g. "0000000010" > "0000000009")
//! - node_id: 8-char hex identifier for this daemon instance

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use forge_core::types::{Memory, MemoryStatus, MemoryType};
use rusqlite::{params, Connection, OptionalExtension};

/// Hybrid Logical Clock for causal ordering across machines.
pub struct Hlc {
    node_id: String,
    state: Mutex<HlcState>,
}

struct HlcState {
    last_wall_ms: u64,
    counter: u64,
}

impl Hlc {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            state: Mutex::new(HlcState {
                last_wall_ms: 0,
                counter: 0,
            }),
        }
    }

    /// Generate a new HLC timestamp. Always monotonically increasing.
    pub fn now(&self) -> String {
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut state = self.state.lock().unwrap();
        if wall_ms > state.last_wall_ms {
            state.last_wall_ms = wall_ms;
            state.counter = 0;
        } else {
            state.counter += 1;
        }
        format!(
            "{}-{:010}-{}",
            state.last_wall_ms, state.counter, self.node_id
        )
    }

    /// Merge with a remote HLC timestamp to maintain causal ordering.
    pub fn merge(&self, remote_ts: &str) {
        let parts: Vec<&str> = remote_ts.splitn(3, '-').collect();
        if parts.len() < 2 {
            return;
        }
        let remote_ms: u64 = parts[0].parse().unwrap_or(0);
        let remote_counter: u64 = parts[1].parse().unwrap_or(0);

        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut state = self.state.lock().unwrap();
        if remote_ms > state.last_wall_ms && remote_ms > wall_ms {
            state.last_wall_ms = remote_ms;
            state.counter = remote_counter + 1;
        } else if remote_ms == state.last_wall_ms {
            state.counter = state.counter.max(remote_counter) + 1;
        }
        // If wall_ms > both, next now() call will advance naturally
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

/// Generate a stable 8-char hex node ID from hostname + process info.
pub fn generate_node_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        hostname.trim().hash(&mut hasher);
    } else {
        "unknown".hash(&mut hasher);
    }
    // Include a stable machine identifier
    std::env::consts::OS.hash(&mut hasher);
    std::env::consts::ARCH.hash(&mut hasher);
    if let Ok(home) = std::env::var("HOME") {
        home.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())[..8].to_string()
}

// ── HLC Backfill ──

/// Backfill HLC timestamps on existing memories that have empty hlc_timestamp.
/// Returns the number of memories updated.
pub fn backfill_hlc(conn: &Connection, hlc: &Hlc) -> rusqlite::Result<usize> {
    let mut stmt =
        conn.prepare("SELECT id FROM memory WHERE hlc_timestamp = '' OR hlc_timestamp IS NULL")?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let node_id = hlc.node_id();
    for id in &ids {
        let ts = hlc.now();
        conn.execute(
            "UPDATE memory SET hlc_timestamp = ?1, node_id = ?2 WHERE id = ?3",
            params![ts, node_id, id],
        )?;
    }
    Ok(ids.len())
}

// ── Helper: memory_type string conversion ──

fn type_str(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::Decision => "decision",
        MemoryType::Lesson => "lesson",
        MemoryType::Pattern => "pattern",
        MemoryType::Preference => "preference",
        MemoryType::Protocol => "protocol",
    }
}

fn type_from_str(s: &str) -> MemoryType {
    match s {
        "decision" => MemoryType::Decision,
        "lesson" => MemoryType::Lesson,
        "pattern" => MemoryType::Pattern,
        "preference" => MemoryType::Preference,
        "protocol" => MemoryType::Protocol,
        _ => MemoryType::Decision,
    }
}

// ── Task 3: Sync Export ──

/// Export active memories as NDJSON lines with HLC metadata.
/// Optionally filtered by project and/or since a given HLC timestamp.
pub fn sync_export(
    conn: &Connection,
    project: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let mut lines = Vec::new();

    // Build query based on filters
    let (sql, _param_values) = build_export_query(project, since);

    let mut stmt = conn.prepare(&sql)?;

    let rows: Vec<Memory> = match (project, since) {
        (Some(p), Some(s)) => {
            let mapped = stmt.query_map(params![p, s], row_to_memory)?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        (Some(p), None) => {
            let mapped = stmt.query_map(params![p], row_to_memory)?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        (None, Some(s)) => {
            let mapped = stmt.query_map(params![s], row_to_memory)?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        (None, None) => {
            let mapped = stmt.query_map([], row_to_memory)?;
            mapped.filter_map(|r| r.ok()).collect()
        }
    };

    for mem in &rows {
        if mem.hlc_timestamp.is_empty() {
            eprintln!(
                "[sync] WARN: memory {} has empty HLC — run backfill before syncing",
                mem.id
            );
        }
    }

    // Reject export if ANY memory has empty HLC — forces backfill first
    let empty_hlc_count = rows.iter().filter(|m| m.hlc_timestamp.is_empty()).count();
    if empty_hlc_count > 0 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "{empty_hlc_count} memories have empty HLC timestamps — run HLC backfill before export"
        )));
    }

    for mem in rows {
        if let Ok(json) = serde_json::to_string(&mem) {
            lines.push(json);
        }
    }

    // Export identity facets with _type marker
    if let Ok(facets) = crate::db::manas::list_identity(conn, "claude-code", true) {
        for f in facets {
            if let Ok(mut json) = serde_json::to_value(&f) {
                if let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "_type".to_string(),
                        serde_json::Value::String("identity".to_string()),
                    );
                }
                if let Ok(line) = serde_json::to_string(&json) {
                    lines.push(line);
                }
            }
        }
    }

    Ok(lines)
}

fn build_export_query(project: Option<&str>, since: Option<&str>) -> (String, Vec<String>) {
    let base = "SELECT id, memory_type, title, content, confidence, status, project, tags,
                       created_at, accessed_at, valence, intensity, hlc_timestamp, node_id,
                       session_id, access_count, COALESCE(activation_level, 0.0),
                       COALESCE(alternatives, '[]'), COALESCE(participants, '[]'),
                       organization_id
                FROM memory WHERE status = 'active'";

    let mut clauses = String::from(base);
    let mut param_values = Vec::new();
    let mut param_idx = 1;

    if let Some(p) = project {
        clauses.push_str(&format!(
            " AND (project = ?{param_idx} OR project IS NULL OR project = '')"
        ));
        param_values.push(p.to_string());
        param_idx += 1;
    }

    if let Some(s) = since {
        clauses.push_str(&format!(" AND hlc_timestamp > ?{param_idx}"));
        param_values.push(s.to_string());
    }

    clauses.push_str(" ORDER BY hlc_timestamp");
    (clauses, param_values)
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let type_s: String = row.get(1)?;
    let status_s: String = row.get(5)?;
    let project: Option<String> = row.get(6)?;
    let tags_json: String = row.get(7)?;
    let alternatives_json: String = row
        .get::<_, String>(17)
        .unwrap_or_else(|_| "[]".to_string());
    let participants_json: String = row
        .get::<_, String>(18)
        .unwrap_or_else(|_| "[]".to_string());

    let memory_type = type_from_str(&type_s);
    let status = crate::db::ops::status_from_str(&status_s);
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let alternatives: Vec<String> = serde_json::from_str(&alternatives_json).unwrap_or_default();
    let participants: Vec<String> = serde_json::from_str(&participants_json).unwrap_or_default();

    Ok(Memory {
        id: row.get(0)?,
        memory_type,
        title: row.get(2)?,
        content: row.get(3)?,
        confidence: row.get(4)?,
        status,
        project,
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
        alternatives,
        participants,
        organization_id: row.get::<_, Option<String>>(19).unwrap_or(None),
        superseded_by: None,
        valence_flipped_at: None,
    })
}

// ── Cross-Tier Sync Policies ──

/// Sync direction — defines valid sync flows between tiers.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncDirection {
    /// local → team: individual pushes up to shared team memory
    LocalToTeam,
    /// team → local: team broadcasts down to members
    TeamToLocal,
    /// team → org: team pushes up to organization
    TeamToOrg,
    /// org → team: org broadcasts down to teams
    OrgToTeam,
}

/// Determines which memory types are allowed to sync in a given direction.
/// Returns true if the memory type is allowed for this sync direction.
pub fn is_sync_allowed(direction: &SyncDirection, memory_type: &MemoryType) -> bool {
    match direction {
        // Local → Team: decisions and lessons propagate up, preferences don't
        SyncDirection::LocalToTeam => matches!(
            memory_type,
            MemoryType::Decision | MemoryType::Lesson | MemoryType::Pattern | MemoryType::Protocol
        ),
        // Team → Local: everything propagates down (team shares with members)
        SyncDirection::TeamToLocal => true,
        // Team → Org: only decisions and protocols propagate to org level
        SyncDirection::TeamToOrg => {
            matches!(memory_type, MemoryType::Decision | MemoryType::Protocol)
        }
        // Org → Team: everything propagates down (org-wide policies)
        SyncDirection::OrgToTeam => true,
    }
}

/// Filter memories by sync policy before export.
/// Returns only memories allowed for the given sync direction.
pub fn filter_by_sync_policy(memories: Vec<Memory>, direction: &SyncDirection) -> Vec<Memory> {
    memories
        .into_iter()
        .filter(|m| is_sync_allowed(direction, &m.memory_type))
        .collect()
}

/// Export memories with cross-tier sync policy applied.
/// Filters by direction rules + org_id scoping.
pub fn sync_export_with_policy(
    conn: &Connection,
    project: Option<&str>,
    since: Option<&str>,
    direction: &SyncDirection,
    org_id: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    // Export all active memories (scoped by project/since/org)
    let all_lines = sync_export(conn, project, since)?;

    // Parse, filter by policy, re-serialize
    let mut filtered_lines = Vec::new();
    for line in &all_lines {
        if let Ok(memory) = serde_json::from_str::<Memory>(line) {
            // Check org_id match if specified
            let org_ok = match org_id {
                Some(oid) => {
                    memory.organization_id.as_deref() == Some(oid)
                        || memory.organization_id.is_none()
                }
                None => true,
            };
            if org_ok && is_sync_allowed(direction, &memory.memory_type) {
                filtered_lines.push(line.clone());
            }
        }
    }
    Ok(filtered_lines)
}

// ── Task 4: Sync Import with Conflict Detection ──

pub struct SyncImportResult {
    pub imported: usize,
    pub conflicts: usize,
    pub skipped: usize,
}

/// Import NDJSON memory lines from a remote node with conflict detection.
///
/// For each line:
/// 1. Parse as Memory (or identity facet if `_type: "identity"`)
/// 2. Check if same title+type+project exists locally
/// 3. Same content => skip
/// 4. Different content AND different node_id => CONFLICT (mark both)
/// 5. Same node_id => update if remote HLC is newer
/// 6. Doesn't exist => import directly
pub fn sync_import(
    conn: &Connection,
    lines: &[String],
    local_node_id: &str,
) -> rusqlite::Result<SyncImportResult> {
    let mut imported = 0;
    let mut conflicts = 0;
    let mut skipped = 0;

    // SECURITY: limit total import size to prevent OOM
    const MAX_LINES: usize = 10_000;
    if lines.len() > MAX_LINES {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "sync import exceeds {} line limit ({} lines)",
            MAX_LINES,
            lines.len()
        )));
    }

    // Wrap in transaction for atomicity — all-or-nothing (Codex fix: partial writes on failure)
    let tx = conn.unchecked_transaction()?;

    for line in lines {
        // SECURITY: skip oversized lines (>1MB per line)
        if line.len() > 1_048_576 {
            skipped += 1;
            continue;
        }

        // Check for identity facet lines first
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("_type").and_then(|t| t.as_str()) == Some("identity") {
                // Import identity facet
                if import_identity_facet(conn, &val).is_ok() {
                    imported += 1;
                } else {
                    skipped += 1;
                }
                continue;
            }
        }

        // Parse as Memory
        let remote_mem: Memory = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let mt = type_str(&remote_mem.memory_type);
        let proj = remote_mem.project.as_deref().unwrap_or("");

        // Check for existing memory with same title + type + project
        let existing: Option<(String, String, String, String)> = conn
            .query_row(
                "SELECT id, content, node_id, hlc_timestamp FROM memory
                 WHERE title = ?1 AND memory_type = ?2
                 AND COALESCE(project, '') = ?3
                 AND status = 'active'",
                params![remote_mem.title, mt, proj],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;

        match existing {
            Some((existing_id, existing_content, _existing_node, existing_hlc)) => {
                if existing_content == remote_mem.content {
                    // Same content, no action needed
                    skipped += 1;
                } else if existing_hlc.is_empty() {
                    // SAFETY: local memory was never HLC-backfilled — empty string
                    // comparison is semantically dangerous (any non-empty remote wins).
                    // Treat as conflict to prevent silent overwrites.
                    eprintln!(
                        "[sync] WARN: local memory {existing_id} has empty HLC — treating as conflict for safety"
                    );
                    conn.execute(
                        "UPDATE memory SET status = 'conflict' WHERE id = ?1",
                        params![existing_id],
                    )?;
                    let mut conflict_mem = remote_mem;
                    conflict_mem.status = MemoryStatus::Conflict;
                    conflict_mem.id = format!("conflict-{}", ulid::Ulid::new());
                    crate::db::ops::remember_raw(conn, &conflict_mem)?;
                    conflicts += 1;
                } else if remote_mem.node_id == local_node_id {
                    // Memory originated from THIS node (round-trip sync) => update if HLC newer
                    // SECURITY: compare against local_node_id, NOT existing record's node_id
                    // A malicious peer could spoof node_id in their export to match ours
                    if remote_mem.hlc_timestamp > existing_hlc {
                        conn.execute(
                            "UPDATE memory SET content = ?1, confidence = MAX(confidence, ?2),
                             accessed_at = ?3, hlc_timestamp = ?4, node_id = ?5
                             WHERE id = ?6",
                            params![
                                remote_mem.content,
                                remote_mem.confidence,
                                remote_mem.accessed_at,
                                remote_mem.hlc_timestamp,
                                remote_mem.node_id,
                                existing_id,
                            ],
                        )?;
                        imported += 1;
                    } else {
                        skipped += 1;
                    }
                } else {
                    // Different node, different content => CONFLICT
                    // Mark local version as conflict
                    conn.execute(
                        "UPDATE memory SET status = 'conflict' WHERE id = ?1",
                        params![existing_id],
                    )?;

                    // Store remote version as conflict with a new ID
                    let mut conflict_mem = remote_mem;
                    conflict_mem.status = MemoryStatus::Conflict;
                    conflict_mem.id = format!("conflict-{}", ulid::Ulid::new());
                    crate::db::ops::remember_raw(conn, &conflict_mem)?;
                    conflicts += 1;
                }
            }
            None => {
                // No existing memory => import directly
                crate::db::ops::remember_raw(conn, &remote_mem)?;
                imported += 1;
            }
        }
    }

    tx.commit()?;

    Ok(SyncImportResult {
        imported,
        conflicts,
        skipped,
    })
}

/// Import an identity facet from a sync NDJSON line.
/// Uses highest-strength-wins conflict resolution.
fn import_identity_facet(conn: &Connection, val: &serde_json::Value) -> rusqlite::Result<()> {
    // Extract required fields
    let agent = val
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("claude-code");
    let facet = match val.get("facet").and_then(|v| v.as_str()) {
        Some(f) => f,
        None => return Ok(()),
    };
    let description = val
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let strength = val.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("sync");
    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("");

    // Check for existing facet with same agent + facet name
    let existing_strength: Option<f64> = conn
        .query_row(
            "SELECT strength FROM identity WHERE agent = ?1 AND facet = ?2 AND active = 1",
            params![agent, facet],
            |row| row.get(0),
        )
        .optional()?;

    match existing_strength {
        Some(local_strength) if local_strength >= strength => {
            // Local has higher or equal strength => skip
            Ok(())
        }
        _ => {
            // Remote has higher strength, or facet doesn't exist => upsert
            let facet_obj = forge_core::types::manas::IdentityFacet {
                id: if id.is_empty() {
                    ulid::Ulid::new().to_string()
                } else {
                    id.to_string()
                },
                agent: agent.to_string(),
                facet: facet.to_string(),
                description: description.to_string(),
                strength,
                source: source.to_string(),
                active: true,
                created_at: val
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                user_id: None,
            };
            crate::db::manas::store_identity(conn, &facet_obj)
        }
    }
}

// ── Task 5: Conflict Resolution ──

use forge_core::protocol::response::{ConflictPair, ConflictVersion};

/// List all unresolved sync conflicts, grouped by title+type pairs.
pub fn list_conflicts(conn: &Connection) -> rusqlite::Result<Vec<ConflictPair>> {
    // Fetch all conflict memories
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, title, content, node_id, hlc_timestamp
         FROM memory WHERE status = 'conflict'
         ORDER BY title, memory_type, hlc_timestamp",
    )?;

    let rows: Vec<(String, String, String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Group by title+type into conflict pairs
    let mut pairs: Vec<ConflictPair> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let (ref id1, ref mt1, ref title1, ref content1, ref node1, ref hlc1) = rows[i];

        // Look for a matching pair (same title + type)
        let mut found_pair = false;
        if i + 1 < rows.len() {
            let (ref id2, ref mt2, ref title2, ref content2, ref node2, ref hlc2) = rows[i + 1];
            if title1 == title2 && mt1 == mt2 {
                pairs.push(ConflictPair {
                    title: title1.clone(),
                    memory_type: mt1.clone(),
                    local: ConflictVersion {
                        id: id1.clone(),
                        content: content1.clone(),
                        node_id: node1.clone(),
                        hlc_timestamp: hlc1.clone(),
                    },
                    remote: ConflictVersion {
                        id: id2.clone(),
                        content: content2.clone(),
                        node_id: node2.clone(),
                        hlc_timestamp: hlc2.clone(),
                    },
                });
                i += 2;
                found_pair = true;
            }
        }

        if !found_pair {
            // Orphaned conflict (partner may have been resolved already)
            // Present as a single-sided conflict
            pairs.push(ConflictPair {
                title: title1.clone(),
                memory_type: mt1.clone(),
                local: ConflictVersion {
                    id: id1.clone(),
                    content: content1.clone(),
                    node_id: node1.clone(),
                    hlc_timestamp: hlc1.clone(),
                },
                remote: ConflictVersion {
                    id: String::new(),
                    content: String::new(),
                    node_id: String::new(),
                    hlc_timestamp: String::new(),
                },
            });
            i += 1;
        }
    }

    Ok(pairs)
}

/// Resolve a sync conflict by keeping the given memory ID.
/// Sets the kept version to 'active' and the other version(s) to 'superseded'.
pub fn resolve_conflict(conn: &Connection, keep_id: &str) -> rusqlite::Result<bool> {
    // Find the memory to keep
    let kept: Option<(String, String)> = conn
        .query_row(
            "SELECT title, memory_type FROM memory WHERE id = ?1 AND status = 'conflict'",
            params![keep_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let (title, memory_type) = match kept {
        Some(k) => k,
        None => return Ok(false),
    };

    // Set the chosen version to active
    conn.execute(
        "UPDATE memory SET status = 'active' WHERE id = ?1",
        params![keep_id],
    )?;

    // Set all other conflict versions with same title+type to superseded
    conn.execute(
        "UPDATE memory SET status = 'superseded'
         WHERE title = ?1 AND memory_type = ?2 AND status = 'conflict' AND id != ?3",
        params![title, memory_type, keep_id],
    )?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ops, schema};

    /// Create an in-memory connection with schema initialized for testing.
    fn test_conn() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_backfill_hlc() {
        let conn = test_conn();
        let hlc = Hlc::new("backfill_node");

        // Insert a memory with empty hlc_timestamp directly via SQL
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-old1', 'decision', 'Old Memory', 'content', 0.9, 'active', '', '[]', '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-old2', 'lesson', 'Another Old', 'content2', 0.8, 'active', '', '[]', '2026-01-01', '2026-01-01', 'positive', 0.0, '', '')",
            [],
        ).unwrap();

        // Also insert one that already has HLC
        let mut mem = Memory::new(MemoryType::Decision, "Has HLC", "already stamped");
        mem.set_hlc("1712345678000-0-existing".into(), "existing".into());
        ops::remember(&conn, &mem).unwrap();

        let count = backfill_hlc(&conn, &hlc).unwrap();
        assert_eq!(count, 2, "should backfill exactly the 2 empty memories");

        // Verify they now have HLC timestamps
        let hlc_ts: String = conn
            .query_row(
                "SELECT hlc_timestamp FROM memory WHERE id = 'm-old1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !hlc_ts.is_empty(),
            "backfilled memory should have HLC timestamp"
        );
        assert!(
            hlc_ts.contains("backfill_node"),
            "backfilled HLC should contain node_id"
        );

        let node: String = conn
            .query_row("SELECT node_id FROM memory WHERE id = 'm-old2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(node, "backfill_node");

        // Running again should find 0
        let count2 = backfill_hlc(&conn, &hlc).unwrap();
        assert_eq!(count2, 0, "second backfill should find nothing to update");
    }

    #[test]
    fn test_hlc_new() {
        let hlc = Hlc::new("node1");
        let ts = hlc.now();
        assert!(ts.contains("node1"));
        assert!(ts.len() > 20); // "1712345678000-0-node1"
    }

    #[test]
    fn test_hlc_monotonic() {
        let hlc = Hlc::new("node1");
        let ts1 = hlc.now();
        let ts2 = hlc.now();
        assert!(ts2 > ts1, "HLC should be monotonically increasing");
    }

    #[test]
    fn test_hlc_merge_remote() {
        let hlc = Hlc::new("local");
        let _local_ts = hlc.now();
        // Simulate a remote timestamp from the future using the canonical zero-padded format.
        // Remote peers always produce zero-padded counters, so the comparison is well-defined.
        let future_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 10000;
        let remote_ts = format!("{}-{:010}-remote", future_ms, 5u64);
        hlc.merge(&remote_ts);
        let after_merge = hlc.now();
        assert!(
            after_merge > remote_ts,
            "after merge, HLC should be ahead of remote: {after_merge} vs {remote_ts}"
        );
    }

    #[test]
    fn test_generate_node_id() {
        let id = generate_node_id();
        assert_eq!(id.len(), 8); // 8-char hex
                                 // Same machine should produce same ID
        let id2 = generate_node_id();
        assert_eq!(id, id2);
    }

    #[test]
    fn test_hlc_node_id_accessor() {
        let hlc = Hlc::new("mynode");
        assert_eq!(hlc.node_id(), "mynode");
    }

    #[test]
    fn test_hlc_format() {
        let hlc = Hlc::new("abc12345");
        let ts = hlc.now();
        let parts: Vec<&str> = ts.splitn(3, '-').collect();
        assert_eq!(
            parts.len(),
            3,
            "HLC should have 3 parts: wall_ms-counter-node_id"
        );
        assert!(
            parts[0].parse::<u64>().is_ok(),
            "wall_ms should be a number"
        );
        assert!(
            parts[1].parse::<u64>().is_ok(),
            "counter should be a number"
        );
        assert_eq!(parts[2], "abc12345");
    }

    // ── Task 3 tests: sync_export ──

    #[test]
    fn test_sync_export_returns_ndjson_with_hlc() {
        let conn = test_conn();
        let mut mem = Memory::new(MemoryType::Decision, "Use JWT", "For auth");
        mem.set_hlc("1712345678000-0-abc12345".into(), "abc12345".into());
        ops::remember(&conn, &mem).unwrap();

        let exported = sync_export(&conn, None, None).unwrap();
        assert!(
            !exported.is_empty(),
            "should export at least one memory line"
        );
        assert!(exported[0].contains("Use JWT"));
        assert!(exported[0].contains("abc12345"));
    }

    #[test]
    fn test_sync_export_project_filter() {
        let conn = test_conn();

        let mut mem1 =
            Memory::new(MemoryType::Decision, "Project A", "Content A").with_project("proj_a");
        mem1.set_hlc("1712345678000-0-node1".into(), "node1".into());
        ops::remember(&conn, &mem1).unwrap();

        let mut mem2 =
            Memory::new(MemoryType::Decision, "Project B", "Content B").with_project("proj_b");
        mem2.set_hlc("1712345679000-0-node1".into(), "node1".into());
        ops::remember(&conn, &mem2).unwrap();

        let exported = sync_export(&conn, Some("proj_a"), None).unwrap();
        // Should include proj_a memory (and possibly global ones)
        let has_a = exported.iter().any(|l| l.contains("Project A"));
        assert!(has_a, "should include project A memory");
    }

    #[test]
    fn test_sync_export_since_filter() {
        let conn = test_conn();

        let mut mem1 = Memory::new(MemoryType::Decision, "Old mem", "Content old");
        mem1.set_hlc("1712345678000-0-node1".into(), "node1".into());
        ops::remember(&conn, &mem1).unwrap();

        let mut mem2 = Memory::new(MemoryType::Decision, "New mem", "Content new");
        mem2.set_hlc("1712345699000-0-node1".into(), "node1".into());
        ops::remember(&conn, &mem2).unwrap();

        let exported = sync_export(&conn, None, Some("1712345690000-0-node1")).unwrap();
        let has_new = exported.iter().any(|l| l.contains("New mem"));
        let has_old = exported.iter().any(|l| l.contains("Old mem"));
        assert!(has_new, "should include new memory");
        assert!(!has_old, "should not include old memory");
    }

    // ── Task 4 tests: sync_import ──

    #[test]
    fn test_sync_import_no_conflict() {
        let conn = test_conn();
        let line = serde_json::to_string(&Memory {
            id: "m-remote1".into(),
            memory_type: MemoryType::Decision,
            title: "Remote Decision".into(),
            content: "From remote".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345678000-0-remote01".into(),
            node_id: "remote01".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: vec![],
            participants: vec![],
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        })
        .unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn test_sync_import_skips_same_content() {
        let conn = test_conn();

        // Store a local memory
        let mut mem = Memory::new(MemoryType::Decision, "Use JWT", "For auth");
        mem.set_hlc("1712345678000-0-local123".into(), "local123".into());
        ops::remember(&conn, &mem).unwrap();

        // Import same content from remote
        let remote = Memory {
            id: "m-remote1".into(),
            memory_type: MemoryType::Decision,
            title: "Use JWT".into(),
            content: "For auth".into(), // Same content
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345679000-0-remote01".into(),
            node_id: "remote01".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(result.skipped, 1, "same content should be skipped");
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.imported, 0);
    }

    #[test]
    fn test_sync_import_detects_conflict() {
        let conn = test_conn();

        // Store a local memory
        let mut mem = Memory::new(MemoryType::Decision, "Use JWT", "Local version");
        mem.set_hlc("1712345678000-0-local123".into(), "local123".into());
        ops::remember(&conn, &mem).unwrap();

        // Import remote memory with same title+type but different content + different node
        let remote = Memory {
            id: "m-remote1".into(),
            memory_type: MemoryType::Decision,
            title: "Use JWT".into(),
            content: "Remote version".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345679000-0-remote01".into(),
            node_id: "remote01".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(result.conflicts, 1, "should detect a conflict");
        assert_eq!(result.imported, 0);

        // Both versions should be marked as conflicts
        let conflict_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'conflict'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_count, 2, "both versions should be conflict");
    }

    #[test]
    fn test_sync_import_same_node_updates() {
        let conn = test_conn();

        // Store a local memory
        let mut mem = Memory::new(MemoryType::Decision, "Use JWT", "Original");
        mem.set_hlc("1712345678000-0-local123".into(), "local123".into());
        ops::remember(&conn, &mem).unwrap();

        // Import from same node with newer HLC
        let remote = Memory {
            id: "m-update".into(),
            memory_type: MemoryType::Decision,
            title: "Use JWT".into(),
            content: "Updated content".into(),
            confidence: 0.95,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345679000-0-local123".into(),
            node_id: "local123".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(result.imported, 1, "same node should update, not conflict");
        assert_eq!(result.conflicts, 0);

        // Verify content was updated
        let content: String = conn
            .query_row(
                "SELECT content FROM memory WHERE title = 'Use JWT' AND status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(content, "Updated content");
    }

    #[test]
    fn test_sync_import_skips_malformed_lines() {
        let conn = test_conn();
        let result = sync_import(&conn, &["not valid json".into()], "local123").unwrap();
        assert_eq!(result.skipped, 1);
        assert_eq!(result.imported, 0);
        assert_eq!(result.conflicts, 0);
    }

    // ── Task 5 tests: conflict resolution ──

    #[test]
    fn test_list_conflicts() {
        let conn = test_conn();

        // Create a conflict scenario via import
        let mut local = Memory::new(MemoryType::Decision, "Auth method", "Local: OAuth");
        local.set_hlc("1712345678000-0-local1".into(), "local1".into());
        ops::remember(&conn, &local).unwrap();

        let remote = Memory {
            id: "r1".into(),
            memory_type: MemoryType::Decision,
            title: "Auth method".into(),
            content: "Remote: JWT".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345679000-0-remote1".into(),
            node_id: "remote1".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();
        sync_import(&conn, &[line], "local1").unwrap();

        let conflicts = list_conflicts(&conn).unwrap();
        assert_eq!(conflicts.len(), 1, "should have one conflict pair");
        assert_eq!(conflicts[0].title, "Auth method");
        assert_eq!(conflicts[0].memory_type, "decision");
    }

    #[test]
    fn test_resolve_conflict_keep_local() {
        let conn = test_conn();

        // Create conflict
        let mut local = Memory::new(MemoryType::Decision, "DB choice", "Local: Postgres");
        local.set_hlc("1712345678000-0-local1".into(), "local1".into());
        ops::remember(&conn, &local).unwrap();
        let local_id = local.id.clone();

        let remote = Memory {
            id: "r2".into(),
            memory_type: MemoryType::Decision,
            title: "DB choice".into(),
            content: "Remote: MySQL".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345679000-0-remote1".into(),
            node_id: "remote1".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();
        sync_import(&conn, &[line], "local1").unwrap();

        // Resolve: keep local
        let resolved = resolve_conflict(&conn, &local_id).unwrap();
        assert!(resolved, "should successfully resolve");

        // Verify local is active
        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                params![local_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "active");

        // Verify remote is superseded
        let superseded_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE title = 'DB choice' AND status = 'superseded'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(superseded_count, 1);
    }

    #[test]
    fn test_resolve_conflict_nonexistent_id() {
        let conn = test_conn();
        let resolved = resolve_conflict(&conn, "nonexistent").unwrap();
        assert!(!resolved, "should return false for nonexistent id");
    }

    // ── Task 6 tests: identity sync ──

    #[test]
    fn test_sync_import_identity_facet() {
        let conn = test_conn();

        let identity_line = serde_json::json!({
            "_type": "identity",
            "id": "idf-1",
            "agent": "claude-code",
            "facet": "role",
            "description": "Senior Rust engineer",
            "strength": 0.9,
            "source": "sync",
            "active": true,
            "created_at": "2026-04-03"
        });
        let line = serde_json::to_string(&identity_line).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(result.imported, 1, "identity facet should be imported");

        // Verify it was stored
        let facets = crate::db::manas::list_identity(&conn, "claude-code", true).unwrap();
        assert!(
            facets.iter().any(|f| f.facet == "role"),
            "role facet should exist"
        );
    }

    #[test]
    fn test_sync_export_includes_identity() {
        let conn = test_conn();

        // Store an identity facet
        let facet = forge_core::types::manas::IdentityFacet {
            id: "idf-test".into(),
            agent: "claude-code".into(),
            facet: "expertise".into(),
            description: "Memory systems".into(),
            strength: 0.8,
            source: "user".into(),
            active: true,
            created_at: "2026-04-03".into(),
            user_id: None,
        };
        crate::db::manas::store_identity(&conn, &facet).unwrap();

        // Must also have at least one memory with valid HLC for export to succeed
        let mut mem = Memory::new(MemoryType::Decision, "Placeholder", "content");
        mem.set_hlc("1712345678000-0000000000-node1".into(), "node1".into());
        ops::remember(&conn, &mem).unwrap();

        let exported = sync_export(&conn, None, None).unwrap();
        let has_identity = exported
            .iter()
            .any(|l| l.contains("\"_type\":\"identity\""));
        assert!(
            has_identity,
            "export should include identity facets with _type marker"
        );
    }

    // ── Bug 7 tests: HLC sync safety ──

    #[test]
    fn test_sync_import_empty_local_hlc_creates_conflict() {
        let conn = test_conn();

        // Insert a local memory with empty HLC (un-backfilled) directly via SQL
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags,
             created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-empty-hlc', 'decision', 'Unversioned', 'Local content', 0.9, 'active', '', '[]',
                     '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '')",
            [],
        ).unwrap();

        // Import remote memory with same title but different content
        let remote = Memory {
            id: "m-remote-hlc".into(),
            memory_type: MemoryType::Decision,
            title: "Unversioned".into(),
            content: "Remote content".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345678000-0000000000-remote01".into(),
            node_id: "remote01".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(
            result.conflicts, 1,
            "empty local HLC should create conflict, not silent overwrite"
        );
        assert_eq!(
            result.imported, 0,
            "should NOT import when local HLC is empty"
        );

        // Verify both are marked as conflicts
        let conflict_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'conflict'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            conflict_count, 2,
            "both local and remote should be conflict"
        );
    }

    #[test]
    fn test_sync_import_empty_local_hlc_same_content_skips() {
        let conn = test_conn();

        // Insert a local memory with empty HLC but same content as remote
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags,
             created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-empty-hlc2', 'decision', 'Same Title', 'Same content', 0.9, 'active', '', '[]',
                     '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '')",
            [],
        ).unwrap();

        // Import remote with same content — should skip (content match comes first)
        let remote = Memory {
            id: "m-remote-same".into(),
            memory_type: MemoryType::Decision,
            title: "Same Title".into(),
            content: "Same content".into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: vec![],
            embedding: None,
            created_at: "2026-04-03".into(),
            accessed_at: "2026-04-03".into(),
            valence: "neutral".into(),
            intensity: 0.0,
            hlc_timestamp: "1712345678000-0000000000-remote01".into(),
            node_id: "remote01".into(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        };
        let line = serde_json::to_string(&remote).unwrap();

        let result = sync_import(&conn, &[line], "local123").unwrap();
        assert_eq!(
            result.skipped, 1,
            "same content should skip even with empty local HLC"
        );
        assert_eq!(result.conflicts, 0);
    }

    #[test]
    fn test_sync_export_rejects_empty_hlc() {
        let conn = test_conn();

        // Insert a memory with empty HLC directly via SQL (bypassing backfill)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags,
             created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-no-hlc', 'decision', 'No HLC', 'content', 0.9, 'active', '', '[]',
                     '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '')",
            [],
        ).unwrap();

        // Export should fail because of empty HLC
        let result = sync_export(&conn, None, None);
        assert!(
            result.is_err(),
            "sync_export should reject memories with empty HLC"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("empty HLC"),
            "error should mention empty HLC: {err_msg}"
        );
    }

    #[test]
    fn test_sync_export_succeeds_with_valid_hlc() {
        let conn = test_conn();

        // Insert a memory with valid HLC
        let mut mem = Memory::new(MemoryType::Decision, "Valid HLC", "content");
        mem.set_hlc("1712345678000-0000000000-node1".into(), "node1".into());
        ops::remember(&conn, &mem).unwrap();

        let result = sync_export(&conn, None, None);
        assert!(
            result.is_ok(),
            "sync_export should succeed when all memories have HLC"
        );
    }

    #[test]
    fn test_sync_policy_local_to_team() {
        // Decisions propagate, preferences don't
        assert!(is_sync_allowed(
            &SyncDirection::LocalToTeam,
            &MemoryType::Decision
        ));
        assert!(is_sync_allowed(
            &SyncDirection::LocalToTeam,
            &MemoryType::Lesson
        ));
        assert!(is_sync_allowed(
            &SyncDirection::LocalToTeam,
            &MemoryType::Protocol
        ));
        assert!(!is_sync_allowed(
            &SyncDirection::LocalToTeam,
            &MemoryType::Preference
        ));
    }

    #[test]
    fn test_sync_policy_team_to_org() {
        // Only decisions and protocols propagate to org level
        assert!(is_sync_allowed(
            &SyncDirection::TeamToOrg,
            &MemoryType::Decision
        ));
        assert!(is_sync_allowed(
            &SyncDirection::TeamToOrg,
            &MemoryType::Protocol
        ));
        assert!(!is_sync_allowed(
            &SyncDirection::TeamToOrg,
            &MemoryType::Lesson
        ));
        assert!(!is_sync_allowed(
            &SyncDirection::TeamToOrg,
            &MemoryType::Pattern
        ));
        assert!(!is_sync_allowed(
            &SyncDirection::TeamToOrg,
            &MemoryType::Preference
        ));
    }

    #[test]
    fn test_sync_policy_downward_allows_all() {
        // Team→Local and Org→Team allow everything
        for mt in [
            MemoryType::Decision,
            MemoryType::Lesson,
            MemoryType::Pattern,
            MemoryType::Preference,
            MemoryType::Protocol,
        ] {
            assert!(is_sync_allowed(&SyncDirection::TeamToLocal, &mt));
            assert!(is_sync_allowed(&SyncDirection::OrgToTeam, &mt));
        }
    }

    #[test]
    fn test_filter_by_sync_policy() {
        let memories = vec![
            Memory::new(MemoryType::Decision, "Keep this", "content"),
            Memory::new(MemoryType::Preference, "Drop this", "content"),
            Memory::new(MemoryType::Lesson, "Keep this too", "content"),
        ];
        let filtered = filter_by_sync_policy(memories, &SyncDirection::LocalToTeam);
        assert_eq!(filtered.len(), 2, "preference should be filtered out");
        assert_eq!(filtered[0].title, "Keep this");
        assert_eq!(filtered[1].title, "Keep this too");
    }
}
