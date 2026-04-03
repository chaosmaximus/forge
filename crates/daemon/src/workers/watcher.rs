// workers/watcher.rs — File watcher using notify crate
//
// Watches directories from all detected agent adapters for transcript files.
// Sends file paths to the extractor via an mpsc channel.
// Debounces events for 2 seconds before sending unique paths.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Watch configuration: (directory, file_extension).
pub type WatchConfig = (PathBuf, String);

/// Watch multiple directories for transcript files matching their extensions.
/// Sends unique file paths to `tx` after a 2-second debounce window.
/// Directories that don't exist are polled every 5 seconds until they appear.
pub async fn run_watcher(
    tx: mpsc::Sender<PathBuf>,
    watch_configs: Vec<WatchConfig>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    if watch_configs.is_empty() {
        eprintln!("[watcher] no watch directories configured");
        return;
    }

    for (dir, ext) in &watch_configs {
        eprintln!("[watcher] will watch: {} (*.{})", dir.display(), ext);
    }

    // Wait for at least one directory to exist
    loop {
        if watch_configs.iter().any(|(dir, _)| dir.exists()) {
            break;
        }
        eprintln!("[watcher] no watch directories exist yet, waiting...");
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[watcher] shutdown received while waiting for dirs");
                    return;
                }
            }
        }
    }

    // Build set of valid extensions for fast lookup
    let valid_extensions: HashSet<String> = watch_configs.iter().map(|(_, ext)| ext.clone()).collect();

    // Channel for notify -> tokio bridge (sync callback -> async receiver)
    let (notify_tx, mut notify_rx) = mpsc::channel::<PathBuf>(256);

    // Create the file watcher
    let watcher_result = {
        let notify_tx = notify_tx.clone();
        let valid_extensions = valid_extensions.clone();
        RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            for path in event.paths {
                                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                    if valid_extensions.contains(ext) {
                                        let _ = notify_tx.blocking_send(path);
                                    }
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
            for (dir, _) in &watch_configs {
                if dir.exists() {
                    if let Err(e) = w.watch(dir, RecursiveMode::Recursive) {
                        eprintln!("[watcher] failed to watch {}: {e}", dir.display());
                    } else {
                        eprintln!("[watcher] watching {}", dir.display());
                    }
                }
            }
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

        for path in pending {
            eprintln!("[watcher] detected: {}", path.display());
            if tx.send(path).await.is_err() {
                eprintln!("[watcher] extractor channel closed, stopping");
                return;
            }
        }
    }
}
