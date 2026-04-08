// workers/watcher.rs — File watcher using notify crate
//
// Watches directories from all detected agent adapters for transcript files.
// Periodically checks for new directories that appear after startup.
// Sends file paths to the extractor via an mpsc channel.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Watch configuration: (directory, file_extension).
pub type WatchConfig = (PathBuf, String);

/// Watch multiple directories for transcript files matching their extensions.
/// Directories that don't exist yet are polled every 30 seconds and attached
/// when they appear (e.g., user installs Cline after daemon is already running).
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
        eprintln!("[watcher] configured: {} (*.{})", dir.display(), ext);
    }

    // Build set of valid extensions for fast lookup
    let valid_extensions: HashSet<String> =
        watch_configs.iter().map(|(_, ext)| ext.clone()).collect();

    // Channel for notify -> tokio bridge
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
                                        let _ = notify_tx.try_send(path);
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

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[watcher] failed to create watcher: {e}");
            return;
        }
    };

    // Track which directories are currently being watched
    let mut watched: HashSet<PathBuf> = HashSet::new();

    // Attach directories that exist now
    for (dir, _) in &watch_configs {
        if dir.exists() {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                eprintln!("[watcher] failed to watch {}: {e}", dir.display());
            } else {
                eprintln!("[watcher] watching {}", dir.display());
                watched.insert(dir.clone());
            }
        }
    }

    // Main loop: process events + periodically check for new directories
    let mut dir_check_interval = tokio::time::interval(Duration::from_secs(30));
    dir_check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let mut pending: HashSet<PathBuf> = HashSet::new();

        // Wait for an event, directory check tick, or shutdown
        tokio::select! {
            Some(path) = notify_rx.recv() => {
                pending.insert(path);
            }
            _ = dir_check_interval.tick() => {
                // Check for newly appeared directories
                for (dir, _) in &watch_configs {
                    if !watched.contains(dir) && dir.exists() {
                        if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                            eprintln!("[watcher] failed to watch new dir {}: {e}", dir.display());
                        } else {
                            eprintln!("[watcher] now watching {}", dir.display());
                            watched.insert(dir.clone());
                        }
                    }
                }
                continue;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_config_type() {
        // WatchConfig is (PathBuf, String) — verify it can be constructed and destructured
        let config: WatchConfig = (PathBuf::from("/tmp/transcripts"), "jsonl".to_string());
        let (dir, ext) = &config;
        assert_eq!(dir, &PathBuf::from("/tmp/transcripts"));
        assert_eq!(ext, "jsonl");
    }

    #[tokio::test]
    async fn test_run_watcher_empty_configs_returns_immediately() {
        // run_watcher with empty watch_configs should return immediately
        // without blocking, since there's nothing to watch.
        let (tx, _rx) = mpsc::channel::<PathBuf>(16);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_watcher(tx, vec![], shutdown_rx),
        )
        .await;

        assert!(result.is_ok(), "run_watcher with empty configs should return immediately");
    }

    #[tokio::test]
    async fn test_run_watcher_shutdown_signal() {
        // run_watcher should exit when shutdown signal is sent
        let (tx, _rx) = mpsc::channel::<PathBuf>(16);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Use a non-existent directory so the watcher has nothing to watch initially
        let configs: Vec<WatchConfig> = vec![
            (PathBuf::from("/tmp/nonexistent_forge_test_dir_12345"), "jsonl".to_string()),
        ];

        let handle = tokio::spawn(async move {
            run_watcher(tx, configs, shutdown_rx).await;
        });

        // Give the watcher a moment to start, then send shutdown
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = shutdown_tx.send(true);

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        assert!(result.is_ok(), "watcher should exit after shutdown signal");
    }
}
