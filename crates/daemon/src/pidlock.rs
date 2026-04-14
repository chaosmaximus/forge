//! Portable PID liveness probe for the daemon's PID lock file.
//!
//! The daemon uses `fs2`-based advisory locks on `<forge_dir>/forge.pid`
//! to prevent two instances from running concurrently. When a stale lock
//! is encountered (the old daemon crashed without releasing), we need to
//! determine whether the PID named in the file is actually alive before
//! cleaning up the lock.
//!
//! The previous implementation at `main.rs` used `/proc/{pid}.exists()`
//! guarded by `#[cfg(unix)]`. That check is *Linux-only*: on macOS
//! there is no `/proc` filesystem, so the check always returned `false`,
//! meaning stale cleanup fired unconditionally whenever the lock-held
//! branch was entered — a latent bug that could allow two live daemons
//! to coexist on macOS if the branch was ever triggered.
//!
//! This module uses `libc::kill(pid, 0)` — the POSIX signal-0 probe —
//! which works on all Unix variants (Linux, macOS, BSD). Sending
//! signal 0 does not actually deliver a signal; it only performs the
//! permission + existence check that the kernel would normally do before
//! a real signal. The return value and `errno` tell us exactly whether
//! the process exists.

/// Returns true if a process with the given PID exists, false otherwise.
///
/// Uses `libc::kill(pid, 0)` — the POSIX signal-0 probe — which works on
/// all Unix platforms. See the module-level docs for rationale.
///
/// Edge cases:
/// - If `kill` returns 0, the process exists and we have permission to
///   signal it → returns `true`.
/// - If `kill` returns -1 with `errno == ESRCH`, no such process exists
///   → returns `false`.
/// - If `kill` returns -1 with `errno == EPERM`, the process exists but
///   we lack permission (e.g., signaling `init` as a non-root user) →
///   returns `true`. The process IS alive; we just can't signal it.
/// - For any other errno, we conservatively return `true` to avoid
///   false-positive stale-cleanup that could steal a live daemon's lock.
// `pub` (not `pub(crate)`) because the `forge-daemon` binary is a SEPARATE
// crate from the library. `pub(crate)` would restrict visibility to the
// library crate, making this function invisible to `main.rs` which imports
// via `forge_daemon::pidlock::is_pid_alive`.
#[cfg(unix)]
pub fn is_pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 does not deliver a signal, queue anything, or
    // touch memory. It only performs the kernel's permission + existence
    // check that would normally precede a real signal. The only observable
    // side effect is updating the caller's thread-local `errno`, which we
    // capture INSIDE the same unsafe block — keeping errno read atomic
    // with the syscall so no intervening Rust code can clobber it.
    let (ret, err) = unsafe {
        let r = libc::kill(pid, 0);
        (r, std::io::Error::last_os_error())
    };
    if ret == 0 {
        return true;
    }
    match err.raw_os_error() {
        // ESRCH — no process with that PID. Confirmed dead.
        Some(libc::ESRCH) => false,
        // EPERM — process exists, we just can't signal it (e.g., non-root
        // trying to signal init). Alive.
        Some(libc::EPERM) => true,
        // Any other errno — conservatively report "alive" to prevent a
        // false-positive stale cleanup that could steal a live daemon's
        // lock. This is the safe failure mode.
        _ => true,
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pid_alive_returns_true_for_self() {
        let own_pid = std::process::id() as i32;
        assert!(
            is_pid_alive(own_pid),
            "is_pid_alive should return true for the current process (pid {own_pid})"
        );
    }

    #[test]
    fn test_is_pid_alive_returns_false_for_nonexistent_pid() {
        // i32::MAX is orders of magnitude beyond the maximum allocatable
        // PID on any Unix system (Linux PID_MAX_LIMIT is 2^22, macOS is
        // similar), so the kernel will return ESRCH from kill(). This
        // specifically exposes the pre-fix macOS bug: the old `/proc/{pid}`
        // check always returned false on macOS, but here we require the
        // function to ALSO return false via a proper syscall — replacing
        // the Linux-only filesystem check with a portable signal probe.
        assert!(
            !is_pid_alive(i32::MAX),
            "is_pid_alive should return false for i32::MAX (never a real PID)"
        );
    }

    #[test]
    fn test_is_pid_alive_returns_true_for_init() {
        // PID 1 is init/launchd on every Unix system and always exists.
        // As a non-root test process we lack permission to signal it, so
        // libc::kill returns -1 with errno == EPERM. This test specifically
        // exercises the EPERM branch of our error handling, which MUST
        // return true — the process IS alive, we just can't touch it.
        //
        // (If the test happens to run as root, kill returns 0 and we
        // reach the success branch instead. Either way, the result is
        // still true — both paths lead to the same correct answer.)
        assert!(
            is_pid_alive(1),
            "is_pid_alive should return true for PID 1 (init/launchd)"
        );
    }
}
