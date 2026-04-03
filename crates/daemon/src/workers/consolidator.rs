// workers/consolidator.rs — Memory consolidator (5 phases)
//
// Periodically runs: exact dedup, semantic dedup, link related, confidence decay,
// and episodic->semantic promotion (recurring lessons become patterns).
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const CONSOLIDATION_INTERVAL: Duration = Duration::from_secs(30 * 60); // 30 minutes

pub async fn run_consolidator(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[consolidator] started, interval = {:?}", CONSOLIDATION_INTERVAL);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(CONSOLIDATION_INTERVAL) => {
                // H-1: Split consolidation into phases, releasing the mutex between each
                // to avoid holding it during slow O(n^2) operations.

                // Track stats for event emission
                let mut exact_dedup_count = 0usize;
                let mut semantic_dedup_count = 0usize;
                let mut linked_count = 0usize;
                let mut faded_count = 0usize;

                // Clone event sender before any phase
                let event_tx = {
                    let locked = state.lock().await;
                    locked.events.clone()
                };

                // Phase 1: Exact dedup (fast, keep lock)
                {
                    let locked = state.lock().await;
                    match ops::dedup_memories(&locked.conn) {
                        Ok(removed) => {
                            exact_dedup_count = removed;
                            if removed > 0 {
                                eprintln!("[consolidator] dedup removed {} duplicate memories", removed);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] dedup error: {}", e),
                    }
                } // lock released

                // Phase 2: Semantic dedup (slow O(n^2), lock released between read and write)
                {
                    let locked = state.lock().await;
                    match ops::semantic_dedup(&locked.conn) {
                        Ok(merged) => {
                            semantic_dedup_count = merged;
                            if merged > 0 {
                                eprintln!("[consolidator] semantic dedup merged {} near-duplicates", merged);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] semantic dedup error: {}", e),
                    }
                } // lock released

                // Phase 3: Link related memories (can be slow with many memories)
                {
                    let locked = state.lock().await;
                    match ops::link_related_memories(&locked.conn) {
                        Ok(linked) => {
                            linked_count = linked;
                            if linked > 0 {
                                eprintln!("[consolidator] linked {} related memory pairs", linked);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] link error: {}", e),
                    }
                } // lock released

                // Phase 4: Decay (fast, keep lock)
                {
                    let locked = state.lock().await;
                    match ops::decay_memories(&locked.conn) {
                        Ok((_decayed, faded)) => {
                            faded_count = faded;
                            if faded > 0 {
                                eprintln!("[consolidator] decayed {}, faded {}", _decayed, faded);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] decay error: {}", e),
                    }
                } // lock released

                // Phase 5: Episodic -> Semantic promotion
                let mut promoted_count = 0usize;
                {
                    let locked = state.lock().await;
                    match ops::promote_recurring_lessons(&locked.conn) {
                        Ok(promoted) => {
                            promoted_count = promoted;
                            if promoted > 0 {
                                eprintln!("[consolidator] promoted {} recurring lessons to patterns", promoted);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] promotion error: {}", e),
                    }
                } // lock released

                // Emit consolidation event
                events::emit(&event_tx, "consolidation", serde_json::json!({
                    "exact_dedup": exact_dedup_count,
                    "semantic_dedup": semantic_dedup_count,
                    "linked": linked_count,
                    "faded": faded_count,
                    "promoted": promoted_count,
                }));
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[consolidator] shutting down");
                return;
            }
        }
    }
}
