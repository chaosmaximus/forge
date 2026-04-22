//! Global handle to the watcher->extractor channel.
//!
//! `workers::spawn_workers` sets this at startup. Request handlers
//! running on per-connection read-only [`DaemonState`] instances can
//! reach the same queue the file watcher feeds by reading this OnceLock —
//! otherwise each reader state would carry its own extractor_tx field or
//! go through the shared Arc<Mutex<DaemonState>> (defeats the read-mutex-
//! free path).
//!
//! Set-once semantics: a second `set` call returns Err and is ignored.

use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::sync::mpsc;

/// Watcher->extractor mpsc sender. `None` before `spawn_workers` runs or
/// in unit-test contexts that don't spawn workers. Consumers `try_send`.
pub static GLOBAL_EXTRACTOR_TX: OnceLock<mpsc::Sender<PathBuf>> = OnceLock::new();
