//! Tests for stale socket detection (Bug 6 fix).
//!
//! Validates that `is_daemon_alive()` uses the canonical PID path
//! (~/.forge/forge.pid) rather than deriving from the socket parent dir.

use serial_test::serial;
use std::io::Write;
use tempfile::TempDir;

/// Helper: set HOME to a temp dir so `forge_core::forge_dir()` resolves to
/// `<tmpdir>/.forge`, then create that directory.
fn setup_forge_home(tmp: &TempDir) -> std::path::PathBuf {
    let home = tmp.path().to_path_buf();
    // SAFETY: these tests must not run in parallel with other tests that depend
    // on HOME, but `cargo test` runs each test binary in its own process.
    unsafe { std::env::set_var("HOME", &home) };
    let forge_dir = home.join(".forge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    forge_dir
}

#[cfg(unix)]
#[test]
#[serial]
fn test_is_daemon_alive_no_pid_file() {
    let tmp = TempDir::new().unwrap();
    let _forge_dir = setup_forge_home(&tmp);
    // No forge.pid file exists — should return false
    assert!(
        !forge_daemon::server::is_daemon_alive(),
        "is_daemon_alive should return false when PID file does not exist"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn test_is_daemon_alive_invalid_pid_content() {
    let tmp = TempDir::new().unwrap();
    let forge_dir = setup_forge_home(&tmp);

    // Write garbage to the PID file
    let pid_path = forge_dir.join("forge.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    write!(f, "not-a-number").unwrap();

    assert!(
        !forge_daemon::server::is_daemon_alive(),
        "is_daemon_alive should return false when PID file contains non-numeric content"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn test_is_daemon_alive_dead_pid() {
    let tmp = TempDir::new().unwrap();
    let forge_dir = setup_forge_home(&tmp);

    // Write a PID that almost certainly doesn't exist (max i32).
    // On Linux, PID_MAX is typically 32768 or 4194304 — 2_000_000_000 is safely dead.
    let pid_path = forge_dir.join("forge.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    write!(f, "2000000000").unwrap();

    assert!(
        !forge_daemon::server::is_daemon_alive(),
        "is_daemon_alive should return false for a non-existent PID"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn test_is_daemon_alive_current_process() {
    let tmp = TempDir::new().unwrap();
    let forge_dir = setup_forge_home(&tmp);

    // Write our own PID — we are definitely alive
    let pid_path = forge_dir.join("forge.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    write!(f, "{}", std::process::id()).unwrap();

    assert!(
        forge_daemon::server::is_daemon_alive(),
        "is_daemon_alive should return true for a running process (our own PID)"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn test_is_daemon_alive_uses_canonical_path_not_socket_parent() {
    let tmp = TempDir::new().unwrap();
    let forge_dir = setup_forge_home(&tmp);

    // Write a valid PID at the canonical location
    let pid_path = forge_dir.join("forge.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    write!(f, "{}", std::process::id()).unwrap();

    // Also create a DIFFERENT directory with a DIFFERENT PID file that has a dead PID.
    // If `is_daemon_alive` were still using the socket parent dir, and we pointed it
    // at this other directory, it would incorrectly return false.
    let other_dir = tmp.path().join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other_pid = other_dir.join("forge.pid");
    let mut f2 = std::fs::File::create(&other_pid).unwrap();
    write!(f2, "2000000000").unwrap(); // dead PID

    // The function should use the canonical path (~/.forge/forge.pid) and return true,
    // NOT the socket parent dir which would give false.
    assert!(
        forge_daemon::server::is_daemon_alive(),
        "is_daemon_alive must use canonical PID path, not socket parent dir"
    );
}
