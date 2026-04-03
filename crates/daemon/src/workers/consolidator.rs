// workers/consolidator.rs — Confidence decay consolidator
//
// Periodically applies exponential confidence decay to memories based on time
// since last access. Memories that fall below 0.1 confidence are marked "faded".
// Also runs semantic dedup (word-overlap) and links related memories by shared tags.

use crate::db::ops;
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

                // Phase 1: Exact dedup (fast, keep lock)
                {
                    let locked = state.lock().await;
                    match ops::dedup_memories(&locked.conn) {
                        Ok(removed) => {
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
                        Ok((decayed, faded)) => {
                            if faded > 0 {
                                eprintln!("[consolidator] decayed {}, faded {}", decayed, faded);
                            }
                        }
                        Err(e) => eprintln!("[consolidator] decay error: {}", e),
                    }
                } // lock released
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[consolidator] shutting down");
                return;
            }
        }
    }
}
