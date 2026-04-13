//! PTY session management for browser-based terminal access.
//!
//! Provides `PtyManager` which creates and manages pseudo-terminal sessions,
//! allowing WebSocket handlers to relay terminal I/O to browser clients.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read as IoRead, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::broadcast;

/// Maximum concurrent PTY sessions (prevents resource exhaustion / DoS).
const MAX_PTY_SESSIONS: usize = 8;

/// Idle timeout for PTY sessions without activity (15 minutes).
const IDLE_TIMEOUT_SECS: u64 = 900;

/// A single PTY session with its master FD, writer, reader thread, and child process.
pub struct PtySession {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    reader_handle: Option<std::thread::JoinHandle<()>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    /// Timestamp of last activity (write or read) for idle timeout.
    pub last_activity: std::time::Instant,
}

/// Counter for generating unique PTY session IDs.
static NEXT_PTY_ID: AtomicU32 = AtomicU32::new(1);

/// Manages multiple PTY sessions, keyed by numeric ID.
#[derive(Default)]
pub struct PtyManager {
    pub sessions: HashMap<u32, PtySession>,
}

impl PtyManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Current number of active PTY sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Maximum allowed concurrent PTY sessions.
    pub fn max_sessions(&self) -> usize {
        MAX_PTY_SESSIONS
    }

    /// Reap idle PTY sessions that have exceeded the idle timeout.
    /// Returns the number of sessions reaped.
    pub fn reap_idle(&mut self) -> usize {
        let timeout = std::time::Duration::from_secs(IDLE_TIMEOUT_SECS);
        let now = std::time::Instant::now();
        let idle_ids: Vec<u32> = self
            .sessions
            .iter()
            .filter(|(_, s)| now.duration_since(s.last_activity) > timeout)
            .map(|(id, _)| *id)
            .collect();
        let count = idle_ids.len();
        for id in idle_ids {
            tracing::info!(
                pty_id = id,
                "reaping idle PTY session (exceeded {}s timeout)",
                IDLE_TIMEOUT_SECS
            );
            self.close(id);
        }
        count
    }

    /// Close all PTY sessions (for graceful shutdown).
    pub fn close_all(&mut self) {
        let ids: Vec<u32> = self.sessions.keys().copied().collect();
        for id in ids {
            self.close(id);
        }
    }

    /// Spawn a new PTY session.
    ///
    /// Returns the session ID and a broadcast receiver for PTY output.
    /// The shell is taken from `$SHELL` (falling back to `/bin/sh`).
    pub fn create(
        &mut self,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
    ) -> Result<(u32, broadcast::Receiver<Vec<u8>>), String> {
        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| format!("failed to open pty: {e}"))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(&shell);

        if let Some(ref dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn shell: {e}"))?;

        // Drop the slave side — only the master is needed after spawn.
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take writer: {e}"))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone reader: {e}"))?;

        let (output_tx, output_rx) = broadcast::channel(256);

        let tx_clone = output_tx.clone();
        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // Ignore send errors — means no subscribers.
                        let _ = tx_clone.send(buf[..n].to_vec());
                    }
                    Err(_) => break,
                }
            }
        });

        let id = NEXT_PTY_ID.fetch_add(1, Ordering::Relaxed);

        let session = PtySession {
            master: pair.master,
            writer,
            reader_handle: Some(reader_handle),
            output_tx,
            child,
            last_activity: std::time::Instant::now(),
        };

        self.sessions.insert(id, session);

        Ok((id, output_rx))
    }

    /// Write data to an existing PTY session. Refreshes idle timeout.
    pub fn write(&mut self, id: u32, data: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("pty session {id} not found"))?;

        session.last_activity = std::time::Instant::now();

        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;

        session
            .writer
            .flush()
            .map_err(|e| format!("flush failed: {e}"))?;

        Ok(())
    }

    /// Resize an existing PTY session.
    pub fn resize(&mut self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let session = self
            .sessions
            .get(&id)
            .ok_or_else(|| format!("pty session {id} not found"))?;

        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("resize failed: {e}"))?;

        Ok(())
    }

    /// Get another broadcast receiver for PTY output.
    pub fn subscribe(&self, id: u32) -> Result<broadcast::Receiver<Vec<u8>>, String> {
        let session = self
            .sessions
            .get(&id)
            .ok_or_else(|| format!("pty session {id} not found"))?;

        Ok(session.output_tx.subscribe())
    }

    /// Close a PTY session: kill the child, drop FDs, join the reader thread, and reap.
    pub fn close(&mut self, id: u32) {
        if let Some(mut session) = self.sessions.remove(&id) {
            // Kill the child process (ignore errors — may already be dead).
            let _ = session.child.kill();

            // Drop the writer to signal EOF to the reader thread.
            drop(session.writer);

            // Join the reader thread so it does not leak.
            if let Some(handle) = session.reader_handle.take() {
                let _ = handle.join();
            }

            // Wait/reap the child process.
            let _ = session.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_close_pty() {
        let mut mgr = PtyManager::new();
        let (id, _rx) = mgr.create(80, 24, None).expect("create should succeed");
        assert!(id > 0, "ID should be > 0");
        assert!(mgr.sessions.contains_key(&id), "session should be in map");

        mgr.close(id);
        assert!(
            !mgr.sessions.contains_key(&id),
            "session should be removed after close"
        );
    }

    #[test]
    fn test_write_to_pty() {
        let mut mgr = PtyManager::new();
        let (id, _rx) = mgr.create(80, 24, None).expect("create should succeed");

        let result = mgr.write(id, "echo hello\n");
        assert!(result.is_ok(), "write should succeed: {result:?}");

        mgr.close(id);
    }

    #[test]
    fn test_resize_pty() {
        let mut mgr = PtyManager::new();
        let (id, _rx) = mgr.create(80, 24, None).expect("create should succeed");

        let result = mgr.resize(id, 120, 40);
        assert!(result.is_ok(), "resize should succeed: {result:?}");

        mgr.close(id);
    }

    #[test]
    fn test_close_nonexistent_pty() {
        let mut mgr = PtyManager::new();
        // Should not panic.
        mgr.close(9999);
    }

    #[test]
    fn test_write_to_nonexistent_pty() {
        let mut mgr = PtyManager::new();
        let result = mgr.write(9999, "hello");
        assert!(result.is_err(), "write to nonexistent should return Err");
    }
}
