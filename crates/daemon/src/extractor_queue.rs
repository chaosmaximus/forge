//! Global handle to the watcher->extractor channel.
//!
//! `workers::spawn_workers` sets this at startup. Request handlers
//! running on per-connection read-only [`DaemonState`] instances can
//! reach the same queue the file watcher feeds by reading this OnceLock —
//! otherwise each reader state would carry its own extractor_tx field or
//! go through the shared Arc<Mutex<DaemonState>> (defeats the read-mutex-
//! free path).
//!
//! # Single-init contract
//!
//! This static is process-wide. The daemon contract is that `spawn_workers`
//! runs **exactly once per process**; a second call is treated as a
//! misconfiguration and logged at `warn` (see `workers::spawn_workers`).
//! Integration tests that want to spin up multiple daemons in one process
//! cannot use this static — they must share the sender directly or restart
//! the process between runs.

use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::sync::mpsc;

/// Watcher->extractor mpsc sender. `None` before `spawn_workers` runs or
/// in unit-test contexts that don't spawn workers. Consumers `try_send`.
pub static GLOBAL_EXTRACTOR_TX: OnceLock<mpsc::Sender<PathBuf>> = OnceLock::new();
