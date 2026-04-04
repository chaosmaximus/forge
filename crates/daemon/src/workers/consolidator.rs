// workers/consolidator.rs — Memory consolidator (9 phases)
//
// Periodically runs: exact dedup, semantic dedup, link related, confidence decay,
// episodic->semantic promotion, reconsolidation, embedding merge,
// edge strengthening, and contradiction detection.
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
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
                let _ = conn.execute(
                    "UPDATE memory SET confidence = ?1 WHERE id = ?2",
                    rusqlite::params![new_confidence, mem.id],
                );
            }
            stats.reconsolidated = candidates.len();
            if !candidates.is_empty() {
                eprintln!("[consolidator] reconsolidated {} memories", candidates.len());
            }
        }
        Err(e) => eprintln!("[consolidator] reconsolidation error: {}", e),
    }

    // Phases 7-9 (embedding_merge, strengthen_active_edges, detect_contradictions)
    // are not yet implemented in ops — stats remain 0.

    stats
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

                // Run all phases (acquires conn from state)
                let stats = {
                    let locked = state.lock().await;
                    run_all_phases(&locked.conn)
                };

                // Emit consolidation event
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
                }));
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
