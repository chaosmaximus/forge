// workers/consolidator.rs — Memory consolidator (13 phases)
//
// Periodically runs: exact dedup, semantic dedup, link related, confidence decay,
// episodic->semantic promotion, reconsolidation, embedding merge,
// edge strengthening, contradiction detection, activation decay,
// entity detection, contradiction synthesis, and knowledge gap detection.
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
use forge_core::types::memory::{Memory, MemoryType};
use forge_core::types::manas::{Perception, PerceptionKind, Severity};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const CONSOLIDATION_INTERVAL: Duration = Duration::from_secs(30 * 60); // 30 minutes

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
}

/// Run all consolidation phases synchronously. Used by:
/// - The periodic consolidator worker (every 30 min)
/// - The ForceConsolidate handler (on demand)
/// - Daemon startup (once)
pub fn run_all_phases(conn: &Connection) -> ConsolidationStats {
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
    let synthesized = synthesize_contradictions(conn);
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

    stats
}

/// Synthesize contradictions: find pairs of conflicting memories (same tags,
/// opposite valence, both active), create a resolution memory, and mark
/// originals as "superseded". Returns count of resolutions created.
pub fn synthesize_contradictions(conn: &Connection) -> usize {
    // Find pairs of active memories with opposite valence, shared tags, high intensity
    let mut stmt = match conn.prepare(
        "SELECT id, title, content, tags, valence, intensity, confidence, project FROM memory
         WHERE status = 'active' AND valence IN ('positive', 'negative') AND intensity > 0.5"
    ) {
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

            // Count shared tags
            let shared: usize = rows[i].tags.iter().filter(|t| rows[j].tags.contains(t)).count();
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

            if let Err(e) = ops::remember(conn, &resolution) {
                eprintln!("[consolidator] failed to store resolution: {e}");
                continue;
            }

            // Mark originals as superseded
            let _ = conn.execute(
                "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                rusqlite::params![a.id],
            );
            let _ = conn.execute(
                "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                rusqlite::params![b.id],
            );

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

pub async fn run_consolidator(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[consolidator] started, interval = {:?}", CONSOLIDATION_INTERVAL);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(CONSOLIDATION_INTERVAL) => {
                // Clone event sender before any phase
                let event_tx = {
                    let locked = state.lock().await;
                    locked.events.clone()
                };

                // Run all 9 phases (acquires conn from state)
                let stats = {
                    let locked = state.lock().await;
                    run_all_phases(&locked.conn)
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
        let stats = run_all_phases(&conn);
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
}
