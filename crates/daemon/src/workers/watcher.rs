// workers/watcher.rs — File watcher using notify crate
//
// Watches ~/.claude/projects/ for modified/created .jsonl files.
// Sends file paths to the extractor via an mpsc channel.
// Debounces events for 2 seconds before sending unique paths.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Watch `~/.claude/projects/` for modified/created `.jsonl` files.
/// Sends unique file paths to `tx` after a 2-second debounce window.
/// If the watch directory does not exist, polls every 5 seconds until it appears (or shutdown).
pub async fn run_watcher(tx: mpsc::Sender<PathBuf>, mut shutdown_rx: watch::Receiver<bool>) {
    let watch_dir = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".claude").join("projects"),
        Err(_) => {
            eprintln!("[watcher] HOME not set, cannot determine watch directory");
            return;
        }
    };

    eprintln!("[watcher] watching: {}", watch_dir.display());

    // Wait for the directory to exist (poll every 5 seconds)
    while !watch_dir.exists() {
        eprintln!(
            "[watcher] {} does not exist yet, waiting...",
            watch_dir.display()
        );
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[watcher] shutdown received while waiting for dir");
                    return;
                }
            }
        }
    }

    // Channel for notify -> tokio bridge (sync callback -> async receiver)
    let (notify_tx, mut notify_rx) = mpsc::channel::<PathBuf>(256);

    // Create the file watcher
    let watcher_result = {
        let notify_tx = notify_tx.clone();
        RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            for path in event.paths {
                                if path.extension().is_some_and(|ext| ext == "jsonl") {
                                    // blocking_send is correct here: notify callback runs on a sync thread
                                    let _ = notify_tx.blocking_send(path);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        )
    };

    let mut _watcher = match watcher_result {
        Ok(mut w) => {
            if let Err(e) = w.watch(&watch_dir, RecursiveMode::Recursive) {
                eprintln!("[watcher] failed to watch {}: {e}", watch_dir.display());
                return;
            }
            eprintln!("[watcher] started watching {}", watch_dir.display());
            w
        }
        Err(e) => {
            eprintln!("[watcher] failed to create watcher: {e}");
            return;
        }
    };

    // Debounce loop: collect events for 2 seconds, then send unique paths
    loop {
        let mut pending = HashSet::new();

        // Wait for at least one event (or shutdown)
        tokio::select! {
            Some(path) = notify_rx.recv() => {
                pending.insert(path);
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[watcher] shutdown received");
                    return;
                }
            }
        }

        // Debounce: collect additional events for 2 seconds
        let debounce_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            tokio::select! {
                Some(path) = notify_rx.recv() => {
                    pending.insert(path);
                }
                _ = tokio::time::sleep_until(debounce_deadline) => {
                    break;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        eprintln!("[watcher] shutdown received during debounce");
                        return;
                    }
                }
            }
        }

        // Send unique paths to extractor
        for path in pending {
            eprintln!("[watcher] detected: {}", path.display());
            if tx.send(path).await.is_err() {
                eprintln!("[watcher] extractor channel closed, stopping");
                return;
            }
        }
    }
}
