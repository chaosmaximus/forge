// workers/consolidator.rs — Confidence decay consolidator
//
// Periodically applies exponential confidence decay to memories based on time
// since last access. Memories that fall below 0.1 confidence are marked "faded".

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
                let locked = state.lock().await;

                // Dedup pass — remove duplicate memories (same title+type)
                match ops::dedup_memories(&locked.conn) {
                    Ok(removed) => {
                        if removed > 0 {
                            eprintln!("[consolidator] dedup removed {} duplicate memories", removed);
                        }
                    }
                    Err(e) => eprintln!("[consolidator] dedup error: {}", e),
                }

                // Decay pass — fade old memories below confidence threshold
                match ops::decay_memories(&locked.conn) {
                    Ok((decayed, faded)) => {
                        if faded > 0 {
                            eprintln!("[consolidator] decayed {}, faded {}", decayed, faded);
                        }
                    }
                    Err(e) => eprintln!("[consolidator] decay error: {}", e),
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[consolidator] shutting down");
                return;
            }
        }
    }
}
