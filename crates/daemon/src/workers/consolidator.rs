// workers/consolidator.rs — Memory consolidator (15 phases)
//
// Periodically runs: exact dedup, semantic dedup, link related, confidence decay,
// episodic->semantic promotion, reconsolidation, embedding merge,
// edge strengthening, contradiction detection, activation decay,
// entity detection, contradiction synthesis, knowledge gap detection,
// memory reweave, and quality scoring.
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
use forge_core::types::memory::{Memory, MemoryType};
use forge_core::types::manas::{Perception, PerceptionKind, Severity};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.consolidation_interval_secs
// (default: 1800 = 30 minutes)

/// Stats returned by a consolidation run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConsolidationStats {
    pub exact_dedup: usize,
    pub semantic_dedup: usize,
    pub linked: usize,
    pub faded: usize,
    pub promoted: usize,
    pub reconsolidated: usize,
    pub embedding_merged: usize,
    pub strengthened: usize,
    pub contradictions: usize,
    pub entities_detected: usize,
    pub synthesized: usize,
    pub gaps_detected: usize,
    pub reweaved: usize,
    pub scored: usize,
}

/// Run all consolidation phases synchronously. Used by:
/// - The periodic consolidator worker (every 30 min)
/// - The ForceConsolidate handler (on demand)
/// - Daemon startup (once)
pub fn run_all_phases(conn: &Connection, config: &crate::config::ConsolidationConfig) -> ConsolidationStats {
    let mut stats = ConsolidationStats::default();

    // Phase 1: Exact dedup (fast)
    match ops::dedup_memories(conn) {
        Ok(removed) => {
            stats.exact_dedup = removed;
            if removed > 0 {
                eprintln!("[consolidator] dedup removed {} duplicate memories", removed);
            }
        }
        Err(e) => eprintln!("[consolidator] dedup error: {}", e),
    }

    // Phase 2: Semantic dedup (slow O(n^2))
    match ops::semantic_dedup(conn) {
        Ok(merged) => {
            stats.semantic_dedup = merged;
            if merged > 0 {
                eprintln!("[consolidator] semantic dedup merged {} near-duplicates", merged);
            }
        }
        Err(e) => eprintln!("[consolidator] semantic dedup error: {}", e),
    }

    // Phase 3: Link related memories
    match ops::link_related_memories(conn) {
        Ok(linked) => {
            stats.linked = linked;
            if linked > 0 {
                eprintln!("[consolidator] linked {} related memory pairs", linked);
            }
        }
        Err(e) => eprintln!("[consolidator] link error: {}", e),
    }

    // Phase 4: Decay (fast)
    match ops::decay_memories(conn) {
        Ok((_decayed, faded)) => {
            stats.faded = faded;
            if faded > 0 {
                eprintln!("[consolidator] faded {}", faded);
            }
        }
        Err(e) => eprintln!("[consolidator] decay error: {}", e),
    }

    // Phase 5: Episodic -> Semantic promotion
    match ops::promote_recurring_lessons(conn) {
        Ok(promoted) => {
            stats.promoted = promoted;
            if promoted > 0 {
                eprintln!("[consolidator] promoted {} recurring lessons to patterns", promoted);
            }
        }
        Err(e) => eprintln!("[consolidator] promotion error: {}", e),
    }

    // Phase 6: Reconsolidation — boost confidence of heavily-accessed memories
    match ops::find_reconsolidation_candidates(conn) {
        Ok(candidates) => {
            for mem in &candidates {
                let new_confidence = (mem.confidence + 0.05).min(1.0);
                if let Err(e) = conn.execute(
                    "UPDATE memory SET confidence = ?1 WHERE id = ?2",
                    rusqlite::params![new_confidence, mem.id],
                ) {
                    eprintln!("[consolidator] failed to reconsolidate memory {}: {e}", mem.id);
                }
            }
            stats.reconsolidated = candidates.len();
            if !candidates.is_empty() {
                eprintln!("[consolidator] reconsolidated {} memories", candidates.len());
            }
        }
        Err(e) => eprintln!("[consolidator] reconsolidation error: {}", e),
    }

    // Phase 7: Embedding-based merge (sleep cycle — deep structural cleanup)
    match ops::embedding_merge(conn) {
        Ok(merged) => {
            stats.embedding_merged = merged;
            if merged > 0 {
                eprintln!("[consolidator] embedding merge: {} similar memories merged", merged);
            }
        }
        Err(e) => eprintln!("[consolidator] embedding merge error: {}", e),
    }

    // Phase 8: Strengthen active edges
    match ops::strengthen_active_edges(conn) {
        Ok(strengthened) => {
            stats.strengthened = strengthened;
            if strengthened > 0 {
                eprintln!("[consolidator] strengthened {} active edges", strengthened);
            }
        }
        Err(e) => eprintln!("[consolidator] edge strengthening error: {}", e),
    }

    // Phase 9: Contradiction detection
    match ops::detect_contradictions(conn) {
        Ok(found) => {
            stats.contradictions = found;
            if found > 0 {
                eprintln!("[consolidator] detected {} contradictory memory pairs", found);
            }
        }
        Err(e) => eprintln!("[consolidator] contradiction detection error: {}", e),
    }

    // Phase 10: Decay activation levels (fast — single UPDATE)
    match ops::decay_activation_levels(conn) {
        Ok(n) => {
            if n > 0 {
                eprintln!("[consolidator] decayed {} activation levels", n);
            }
        }
        Err(e) => eprintln!("[consolidator] activation decay error: {e}"),
    }

    // Phase 11: Entity detection (Knowledge Intelligence)
    match crate::db::manas::detect_entities(conn) {
        Ok(detected) => {
            stats.entities_detected = detected;
            if detected > 0 {
                eprintln!("[consolidator] detected/updated {} entities from memory titles", detected);
            }
        }
        Err(e) => eprintln!("[consolidator] entity detection error: {e}"),
    }

    // Phase 12: Contradiction synthesis — resolve detected contradictions
    let synthesized = synthesize_contradictions(conn, config.batch_limit);
    stats.synthesized = synthesized;
    if synthesized > 0 {
        eprintln!("[consolidator] synthesized {} contradiction resolutions", synthesized);
    }

    // Phase 13: Knowledge gap detection — surface concepts without entities
    let gaps = detect_and_surface_gaps(conn);
    stats.gaps_detected = gaps;
    if gaps > 0 {
        eprintln!("[consolidator] detected {} knowledge gaps", gaps);
    }

    // Phase 14: Memory reweave — enrich older memories with newer context sharing tags
    let reweaved = reweave_memories(conn, config.batch_limit, config.reweave_limit);
    stats.reweaved = reweaved;
    if reweaved > 0 {
        eprintln!("[consolidator] reweaved {} memory pairs", reweaved);
    }

    // Phase 15: Quality scoring — compute quality scores for active memories
    let scored = score_memory_quality(conn, config.batch_limit);
    stats.scored = scored;
    if scored > 0 {
        eprintln!("[consolidator] scored {} memories", scored);
    }

    stats
}

/// Synthesize contradictions: find pairs of conflicting memories (same tags,
/// opposite valence, both active), create a resolution memory, and mark
/// originals as "superseded". Returns count of resolutions created.
pub fn synthesize_contradictions(conn: &Connection, batch_limit: usize) -> usize {
    // Find pairs of active memories with opposite valence, shared tags, high intensity
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, title, content, tags, valence, intensity, confidence, project FROM memory
         WHERE status = 'active' AND valence IN ('positive', 'negative') AND intensity > 0.5
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] synthesize query error: {e}");
            return 0;
        }
    };

    struct ConflictRow {
        id: String,
        title: String,
        content: String,
        tags: Vec<String>,
        valence: String,
        confidence: f64,
        project: Option<String>,
    }

    let rows: Vec<ConflictRow> = match stmt.query_map([], |row| {
        let tags_json: String = row.get(3)?;
        Ok(ConflictRow {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            valence: row.get(4)?,
            confidence: row.get(6)?,
            project: row.get(7)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] synthesize row error: {e}");
            return 0;
        }
    };

    let mut synthesized = 0usize;
    // Track which memory IDs have already been superseded in this run
    let mut superseded_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for i in 0..rows.len() {
        if superseded_ids.contains(&rows[i].id) {
            continue;
        }
        if rows[i].tags.len() < 2 {
            continue;
        }

        for j in (i + 1)..rows.len() {
            if superseded_ids.contains(&rows[j].id) {
                continue;
            }
            if rows[j].tags.len() < 2 {
                continue;
            }

            // Must have opposite valence
            if rows[i].valence == rows[j].valence {
                continue;
            }

            // Count shared tags (HashSet for O(n) instead of O(n^2))
            let tags_i: std::collections::HashSet<&str> = rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j].tags.iter().filter(|t| tags_i.contains(t.as_str())).count();
            if shared < 2 {
                continue;
            }

            // Reference both conflict rows
            let (a, b) = (&rows[i], &rows[j]);

            // Create resolution memory
            let resolution_title = format!("Resolution: {} vs {}", a.title, b.title);
            let resolution_content = format!(
                "Previously: {}. Later: {}. The later decision supersedes the earlier one.",
                a.content, b.content
            );

            // Tags: union + "resolution"
            let mut union_tags: Vec<String> = a.tags.clone();
            for t in &b.tags {
                if !union_tags.contains(t) {
                    union_tags.push(t.clone());
                }
            }
            union_tags.push("resolution".to_string());

            let conf = a.confidence.max(b.confidence);

            let resolution = Memory::new(MemoryType::Decision, &resolution_title, &resolution_content)
                .with_tags(union_tags);
            // Set confidence manually
            let mut resolution = resolution;
            resolution.confidence = conf;
            resolution.project = a.project.clone();

            // Transaction: resolution insert + supersede originals (atomic)
            if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
                eprintln!("[consolidator] failed to begin transaction: {e}");
                continue;
            }
            if let Err(e) = ops::remember(conn, &resolution) {
                eprintln!("[consolidator] failed to store resolution: {e}");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            if conn.execute("UPDATE memory SET status = 'superseded' WHERE id = ?1", rusqlite::params![a.id]).is_err()
                || conn.execute("UPDATE memory SET status = 'superseded' WHERE id = ?1", rusqlite::params![b.id]).is_err()
            {
                eprintln!("[consolidator] failed to supersede originals — rolling back");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            let _ = conn.execute_batch("COMMIT");

            superseded_ids.insert(a.id.clone());
            superseded_ids.insert(b.id.clone());
            synthesized += 1;
        }
    }

    synthesized
}

/// Detect knowledge gaps and surface them as perceptions.
/// A knowledge gap is a word appearing in 3+ memory titles but with no entity.
/// Returns count of gap perceptions created.
pub fn detect_and_surface_gaps(conn: &Connection) -> usize {
    let gaps = match crate::db::manas::detect_knowledge_gaps(conn, None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[consolidator] knowledge gap detection error: {e}");
            return 0;
        }
    };

    let mut count = 0;
    for word in &gaps {
        // Count how many titles reference this word
        let freq: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active' AND LOWER(title) LIKE ?1",
                rusqlite::params![format!("%{}%", word)],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let perception_id = format!("gap-{}", ulid::Ulid::new());
        let p = Perception {
            id: perception_id,
            kind: PerceptionKind::KnowledgeGap,
            data: format!("Knowledge gap: no entity for '{}' despite {} references", word, freq),
            severity: Severity::Info,
            project: None,
            created_at: forge_core::time::now_iso(),
            expires_at: Some(forge_core::time::now_offset(86400)), // 24 hours
            consumed: false,
        };

        if let Err(e) = crate::db::manas::store_perception(conn, &p) {
            eprintln!("[consolidator] failed to store gap perception: {e}");
            continue;
        }
        count += 1;
    }

    count
}

/// Reweave memories: when a newer memory shares 2+ tags with an older memory
/// and both are active with the same project and memory_type, enrich the older
/// memory by appending the newer content and mark the newer one as "merged".
/// Returns count of reweaves performed.
pub fn reweave_memories(conn: &Connection, batch_limit: usize, reweave_limit: usize) -> usize {
    // Find candidate pairs: newer memory shares 2+ tags with older memory,
    // same project, same memory_type, both active
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, title, content, tags, memory_type, project, created_at FROM memory
         WHERE status = 'active' AND tags != '[]'
         ORDER BY created_at ASC
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] reweave query error: {e}");
            return 0;
        }
    };

    struct ReweaveRow {
        id: String,
        content: String,
        tags: Vec<String>,
        memory_type: String,
        project: Option<String>,
        created_at: String,
    }

    let rows: Vec<ReweaveRow> = match stmt.query_map([], |row| {
        let tags_json: String = row.get(3)?;
        Ok(ReweaveRow {
            id: row.get(0)?,
            content: row.get(2)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            memory_type: row.get(4)?,
            project: row.get(5)?,
            created_at: row.get(6)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] reweave row error: {e}");
            return 0;
        }
    };

    let mut reweaved = 0usize;
    let mut merged_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let limit = reweave_limit;

    for i in 0..rows.len() {
        if reweaved >= limit {
            break;
        }
        if merged_ids.contains(&rows[i].id) {
            continue;
        }
        if rows[i].tags.len() < 2 {
            continue;
        }

        for j in (i + 1)..rows.len() {
            if reweaved >= limit {
                break;
            }
            if merged_ids.contains(&rows[j].id) {
                continue;
            }
            if rows[j].tags.len() < 2 {
                continue;
            }

            // Must have same memory_type
            if rows[i].memory_type != rows[j].memory_type {
                continue;
            }

            // Must have same project (both None or both same value)
            if rows[i].project != rows[j].project {
                continue;
            }

            // Count shared tags
            let tags_i: std::collections::HashSet<&str> = rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j].tags.iter().filter(|t| tags_i.contains(t.as_str())).count();
            if shared < 2 {
                continue;
            }

            // rows[i] is older (ordered by created_at ASC), rows[j] is newer
            // Verify j is indeed newer (or at least not the same)
            if rows[j].created_at <= rows[i].created_at {
                continue;
            }

            // Transaction: read current content, enrich, mark newer as merged (atomic)
            if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
                eprintln!("[consolidator] reweave begin error: {e}");
                continue;
            }
            // Re-read content inside transaction to avoid TOCTOU race
            let current_content: String = match conn.query_row(
                "SELECT content FROM memory WHERE id = ?1 AND status = 'active'",
                rusqlite::params![rows[i].id],
                |row| row.get(0),
            ) {
                Ok(c) => c,
                Err(_) => { let _ = conn.execute_batch("ROLLBACK"); continue; }
            };
            let enriched_content = format!("{}\n\n[Update]: {}", current_content, rows[j].content);

            let update1 = conn.execute(
                "UPDATE memory SET content = ?1 WHERE id = ?2 AND status = 'active'",
                rusqlite::params![enriched_content, rows[i].id],
            );
            let update2 = conn.execute(
                "UPDATE memory SET status = 'merged' WHERE id = ?1 AND status = 'active'",
                rusqlite::params![rows[j].id],
            );
            if update1.is_err() || update2.is_err()
                || update1.unwrap_or(0) != 1 || update2.unwrap_or(0) != 1
            {
                eprintln!("[consolidator] reweave update error — rolling back");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            if let Err(e) = conn.execute_batch("COMMIT") {
                eprintln!("[consolidator] reweave commit error: {e}");
                continue;
            }

            merged_ids.insert(rows[j].id.clone());
            reweaved += 1;
        }
    }

    reweaved
}

/// Score memory quality for active memories. Computes a quality score (0.0 to 1.0)
/// based on freshness, utility (access_count), completeness (content length),
/// and activation_level. Stores the result in the quality_score column.
/// Returns count of memories scored.
pub fn score_memory_quality(conn: &Connection, batch_limit: usize) -> usize {
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, content, access_count, activation_level,
                julianday('now') - julianday(created_at) as age_days
         FROM memory WHERE status = 'active'
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] quality score query error: {e}");
            return 0;
        }
    };

    struct ScoreRow {
        id: String,
        content_len: usize,
        access_count: i64,
        activation_level: f64,
        age_days: f64,
    }

    let rows: Vec<ScoreRow> = match stmt.query_map([], |row| {
        let content: String = row.get(1)?;
        Ok(ScoreRow {
            id: row.get(0)?,
            content_len: content.len(),
            access_count: row.get(2)?,
            activation_level: row.get::<_, f64>(3).unwrap_or(0.0),
            age_days: row.get::<_, f64>(4).unwrap_or(0.0),
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] quality score row error: {e}");
            return 0;
        }
    };

    let mut scored = 0usize;
    for row in &rows {
        // freshness (0-1): 1.0 for today, decays by 0.1 per week, min 0.1
        let weeks = row.age_days / 7.0;
        let freshness = (1.0 - weeks * 0.1).clamp(0.1, 1.0);

        // utility (0-1): min(access_count / 10.0, 1.0)
        let utility = (row.access_count as f64 / 10.0).clamp(0.0, 1.0);

        // completeness (0-1): min(content.len() / 200.0, 1.0)
        let completeness = (row.content_len as f64 / 200.0).min(1.0);

        // activation (0-1): activation_level (already 0-1)
        let activation = row.activation_level.clamp(0.0, 1.0);

        let quality_score = freshness * 0.3 + utility * 0.3 + completeness * 0.2 + activation * 0.2;

        if let Err(e) = conn.execute(
            "UPDATE memory SET quality_score = ?1 WHERE id = ?2",
            rusqlite::params![quality_score, row.id],
        ) {
            eprintln!("[consolidator] quality score update error for {}: {e}", row.id);
            continue;
        }
        scored += 1;
    }

    scored
}

pub async fn run_consolidator(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    eprintln!("[consolidator] started, interval = {:?}", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                // Clone event sender before any phase
                let event_tx = {
                    let locked = state.lock().await;
                    locked.events.clone()
                };

                // Run all 15 phases (acquires conn from state)
                let stats = {
                    let consol_config = crate::config::load_config().consolidation.validated();
                    let locked = state.lock().await;
                    run_all_phases(&locked.conn, &consol_config)
                };

                // Emit consolidation event with stats
                events::emit(&event_tx, "consolidation", serde_json::json!({
                    "exact_dedup": stats.exact_dedup,
                    "semantic_dedup": stats.semantic_dedup,
                    "linked": stats.linked,
                    "faded": stats.faded,
                    "promoted": stats.promoted,
                    "reconsolidated": stats.reconsolidated,
                    "embedding_merged": stats.embedding_merged,
                    "strengthened": stats.strengthened,
                    "contradictions": stats.contradictions,
                    "entities_detected": stats.entities_detected,
                    "synthesized": stats.synthesized,
                    "gaps_detected": stats.gaps_detected,
                    "reweaved": stats.reweaved,
                    "scored": stats.scored,
                }));

                // Emit contradiction_detected event if any contradictions were found
                if stats.contradictions > 0 {
                    events::emit(&event_tx, "contradiction_detected", serde_json::json!({
                        "count": stats.contradictions,
                    }));
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[consolidator] shutting down");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_all_phases_returns_stats() {
        // Initialize sqlite-vec extension before opening connection
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // On an empty DB, all stats should be 0
        let config = crate::config::ConsolidationConfig::default();
        let stats = run_all_phases(&conn, &config);
        assert_eq!(stats.exact_dedup, 0);
        assert_eq!(stats.semantic_dedup, 0);
        assert_eq!(stats.linked, 0);
        assert_eq!(stats.faded, 0);
        assert_eq!(stats.promoted, 0);
        assert_eq!(stats.reconsolidated, 0);
        assert_eq!(stats.embedding_merged, 0);
        assert_eq!(stats.strengthened, 0);
        assert_eq!(stats.contradictions, 0);
    }

    #[test]
    fn test_reweave_memories() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create an older memory with tags
        let older = Memory::new(MemoryType::Decision, "Use JWT auth", "We chose JWT for authentication")
            .with_tags(vec!["auth".to_string(), "security".to_string(), "jwt".to_string()]);
        ops::remember(&conn, &older).unwrap();

        // Create a newer memory with shared tags (same project, same type)
        // Need a slight delay in created_at to ensure ordering
        let newer = Memory::new(MemoryType::Decision, "JWT rotation policy", "Rotate JWT tokens every 24h")
            .with_tags(vec!["auth".to_string(), "security".to_string(), "rotation".to_string()]);
        // Manually set a later created_at
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES (?1, 'decision', 'JWT rotation policy', 'Rotate JWT tokens every 24h', 0.9, 'active',
                     '[\"auth\",\"security\",\"rotation\"]', datetime('now', '+1 second'), datetime('now'), NULL)",
            rusqlite::params![newer.id],
        ).unwrap();

        let count = reweave_memories(&conn, 200, 50);
        assert_eq!(count, 1, "should reweave 1 pair");

        // Verify older memory was enriched
        let content: String = conn.query_row(
            "SELECT content FROM memory WHERE id = ?1",
            rusqlite::params![older.id],
            |row| row.get(0),
        ).unwrap();
        assert!(content.contains("[Update]:"), "older memory should contain [Update] marker");
        assert!(content.contains("Rotate JWT tokens every 24h"), "older memory should contain newer content");

        // Verify newer memory was marked as merged
        let status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![newer.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "merged", "newer memory should be marked as merged");
    }

    #[test]
    fn test_reweave_different_types_skipped() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a decision memory
        let decision = Memory::new(MemoryType::Decision, "Use JWT auth", "JWT for authentication")
            .with_tags(vec!["auth".to_string(), "security".to_string()]);
        ops::remember(&conn, &decision).unwrap();

        // Create a lesson memory with same tags — different type should NOT reweave
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES ('lesson-1', 'lesson', 'Auth lesson', 'Learned about auth', 0.9, 'active',
                     '[\"auth\",\"security\"]', datetime('now', '+1 second'), datetime('now'), NULL)",
            [],
        ).unwrap();

        let count = reweave_memories(&conn, 200, 50);
        assert_eq!(count, 0, "should not reweave memories of different types");
    }

    #[test]
    fn test_quality_score_computation() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a memory with known parameters
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('qs-1', 'decision', 'Test quality', ?1, 0.9, 'active', '[]',
                     datetime('now'), datetime('now'), 5, 0.5, NULL)",
            rusqlite::params!["A".repeat(200)], // content_len = 200 -> completeness = 1.0
        ).unwrap();

        let count = score_memory_quality(&conn, 200);
        assert_eq!(count, 1, "should score 1 memory");

        let score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'qs-1'",
            [],
            |row| row.get(0),
        ).unwrap();

        // freshness: created today = 1.0
        // utility: 5/10 = 0.5
        // completeness: 200/200 = 1.0
        // activation: 0.5
        // expected = 1.0*0.3 + 0.5*0.3 + 1.0*0.2 + 0.5*0.2 = 0.3 + 0.15 + 0.2 + 0.1 = 0.75
        assert!((score - 0.75).abs() < 0.05, "score should be ~0.75, got {}", score);
    }

    #[test]
    fn test_quality_score_fresh_vs_old() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Fresh memory — created now
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('fresh-1', 'decision', 'Fresh memory', 'Some content here', 0.9, 'active', '[]',
                     datetime('now'), datetime('now'), 0, 0.0, NULL)",
            [],
        ).unwrap();

        // Old memory — created 70 days ago (10 weeks = freshness decayed to 0.1)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('old-1', 'decision', 'Old memory', 'Some content here', 0.9, 'active', '[]',
                     datetime('now', '-70 days'), datetime('now'), 0, 0.0, NULL)",
            [],
        ).unwrap();

        score_memory_quality(&conn, 200);

        let fresh_score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'fresh-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        let old_score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'old-1'",
            [],
            |row| row.get(0),
        ).unwrap();

        assert!(fresh_score > old_score, "fresh memory score ({}) should be higher than old ({})", fresh_score, old_score);
    }
}
