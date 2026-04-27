// supervisor.rs — Tracking + drain for fire-and-forget background
// blocking tasks (currently force-index; future analytical batch
// writes can hook in via the same surface).
//
// Why this exists:
//
// The WriterActor processes `WriteCommand` messages sequentially on a
// single tokio task; a sub-second write doesn't need supervision.
// But `Request::ForceIndex` (W22 / `611169b`) dispatches the heavy
// indexer pass onto `tokio::task::spawn_blocking` from a fresh writer
// connection so the actor's mutex isn't held for tens of seconds.
// Pre-fix the supervisor task was `tokio::spawn(async move {
// join.await ... })` — fire-and-forget, dropping its `JoinHandle`.
// Two failure modes that were called out in the W23 review HIGH-1
// carry-forward (`feedback_writer_actor_spawn_blocking.md`):
//
// 1. **Reject-overlap**: nothing prevented two concurrent
//    `force-index` calls from both spawning blocking workers — both
//    would race on the same writer connection's COMMITs, multiplying
//    the lock-contention pain force-index was supposed to fix.
//
// 2. **SIGTERM mid-pass split-brain**: the daemon's shutdown path
//    (P3-2 W7) signals workers via `shutdown_tx`, joins them, and
//    exits. The fire-and-forget spawn_blocking task was orphaned —
//    process exit aborted it mid-COMMIT. SQLite WAL guarantees
//    atomic-per-COMMIT, so the DB itself stays consistent; but the
//    indexer's per-pass invariants (file rows + import edges + cluster
//    rebuild ordered as a unit) could be left with a half-finished
//    pass observable on the next start.
//
// This module fixes both:
//
// * `try_claim_indexer()` is a CAS on an `AtomicBool`; second concurrent
//   `force-index` returns a structured "already running" error instead
//   of spawning a duplicate.
// * `spawn_supervised(...)` registers the supervisor task in a `JoinSet`
//   that the shutdown path drains (with a deadline). SIGTERM mid-pass
//   gets up to `drain_timeout` (default 30s) for the in-flight indexer
//   pass to complete, then proceeds to socket teardown. The deadline
//   exists because rusqlite doesn't honor tokio cancellation — a
//   pathologically long pass could hang shutdown indefinitely without
//   it.
//
// Future analytical-batch writes (W23 nomenclature) should use the
// same surface: claim a per-resource `AtomicBool`, spawn via
// `spawn_supervised`, release in the supervisor closure.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::timeout;

/// Per-resource flag + global JoinSet for in-flight blocking tasks.
///
/// Held behind `Arc<...>` so the WriterActor (which runs on its own
/// task) and the shutdown path in `main.rs` (which runs on the main
/// runtime task) share the same state. A single instance per daemon.
pub struct BackgroundTaskSupervisor {
    /// `true` while a force-index pass is in flight. Future per-resource
    /// flags can be added as siblings (e.g., `analytics_running`).
    indexer_running: AtomicBool,
    /// JoinSet of in-flight supervisor tasks. `tokio::sync::Mutex`
    /// because (a) `JoinSet::spawn` and `JoinSet::join_next` need
    /// `&mut self`, (b) the writer-actor and shutdown paths both
    /// touch it from async contexts.
    in_flight: Mutex<JoinSet<()>>,
}

/// Outcome of `BackgroundTaskSupervisor::drain` — used by the shutdown
/// path to decide whether to log a clean drain or a timed-out one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Number of supervisor tasks that completed within the deadline.
    pub drained: usize,
    /// Whether the deadline expired before all tasks completed. When
    /// `true`, some tasks were still running when the timer fired —
    /// the daemon will continue to socket teardown anyway, and the
    /// stranded blocking work will be terminated by process exit
    /// (SQLite WAL preserves DB integrity at COMMIT granularity).
    pub timed_out: bool,
}

impl Default for BackgroundTaskSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundTaskSupervisor {
    pub fn new() -> Self {
        Self {
            indexer_running: AtomicBool::new(false),
            in_flight: Mutex::new(JoinSet::new()),
        }
    }

    /// Atomically try to claim the indexer slot. Returns `true` if
    /// the caller claimed it (and MUST call `release_indexer` when
    /// the work is done — typically inside the supervisor closure
    /// passed to `spawn_supervised`). Returns `false` if another
    /// pass is already in flight; the caller should reject the
    /// request rather than spawn a duplicate.
    pub fn try_claim_indexer(&self) -> bool {
        self.indexer_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Release the indexer slot so the next force-index request can
    /// proceed. Called from the supervisor closure on completion of
    /// the blocking work — including panics and cancellations, so
    /// the slot doesn't get stuck.
    pub fn release_indexer(&self) {
        self.indexer_running.store(false, Ordering::Release);
    }

    /// Spawn a supervisor task tracked by the JoinSet. The shutdown
    /// path's `drain()` will wait for `fut` (with a deadline) before
    /// proceeding to socket teardown.
    ///
    /// `fut` is typically `async move { spawn_blocking(work).await; supervisor.release_indexer(); }`
    /// — see `process_force_index_async` for the canonical shape.
    pub async fn spawn_supervised<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut set = self.in_flight.lock().await;
        set.spawn(fut);
    }

    /// Wait for all in-flight supervisor tasks to complete, with a
    /// deadline. Called from `main.rs`'s shutdown path AFTER
    /// `shutdown_tx.send(true)` (so workers stop accepting new work)
    /// and BEFORE socket file removal (so an in-flight indexer pass
    /// has a chance to finish writing its last batch).
    ///
    /// Returns `DrainOutcome` so the shutdown path can log the result.
    /// On timeout, the daemon proceeds — process exit will terminate
    /// any still-running blocking task; SQLite WAL preserves DB
    /// integrity at COMMIT granularity, so the worst case is a
    /// half-finished indexer pass that the next startup re-runs.
    pub async fn drain(&self, deadline: Duration) -> DrainOutcome {
        let mut set = self.in_flight.lock().await;
        let before = set.len();
        let drain_fut = async { while set.join_next().await.is_some() {} };
        match timeout(deadline, drain_fut).await {
            Ok(()) => DrainOutcome {
                drained: before,
                timed_out: false,
            },
            Err(_) => DrainOutcome {
                drained: before.saturating_sub(set.len()),
                timed_out: true,
            },
        }
    }
}

/// Convenience constructor for the production daemon — wraps a fresh
/// supervisor in an `Arc` so callers don't need to think about it.
pub fn new_arc() -> Arc<BackgroundTaskSupervisor> {
    Arc::new(BackgroundTaskSupervisor::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn try_claim_indexer_is_exclusive() {
        let bg = BackgroundTaskSupervisor::new();
        assert!(bg.try_claim_indexer(), "first claim must succeed");
        assert!(
            !bg.try_claim_indexer(),
            "second concurrent claim must be rejected"
        );
        bg.release_indexer();
        assert!(
            bg.try_claim_indexer(),
            "after release, next claim must succeed"
        );
    }

    #[tokio::test]
    async fn drain_completes_when_supervised_tasks_finish() {
        let bg = Arc::new(BackgroundTaskSupervisor::new());
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let c = Arc::clone(&counter);
            bg.spawn_supervised(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                c.fetch_add(1, Ordering::Relaxed);
            })
            .await;
        }

        let outcome = bg.drain(Duration::from_secs(2)).await;
        assert_eq!(outcome.drained, 3);
        assert!(!outcome.timed_out);
        assert_eq!(counter.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn drain_returns_timed_out_when_deadline_expires() {
        let bg = Arc::new(BackgroundTaskSupervisor::new());

        // A task that runs longer than the deadline.
        bg.spawn_supervised(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
        .await;

        let outcome = bg.drain(Duration::from_millis(50)).await;
        assert!(
            outcome.timed_out,
            "deadline-exceeded drain must report timed_out=true"
        );
        // Subsequent claims should still succeed (the supervisor was
        // never gated by the AtomicBool — that was the indexer-specific
        // guard. The drain's timeout doesn't poison the supervisor for
        // future use; it just signals that the daemon is exiting now.)
    }

    #[tokio::test]
    async fn drain_with_zero_in_flight_is_immediate_success() {
        let bg = BackgroundTaskSupervisor::new();
        let outcome = bg.drain(Duration::from_secs(1)).await;
        assert_eq!(outcome.drained, 0);
        assert!(!outcome.timed_out);
    }

    #[tokio::test]
    async fn release_indexer_on_panic_unsticks_the_slot() {
        // Simulates the production pattern where the supervisor
        // closure releases the slot in all completion paths (Ok / Err
        // / panic). Pre-fix, a panic in spawn_blocking would leave
        // indexer_running=true forever; this test pins the contract
        // that callers MUST release-on-panic.
        let bg = Arc::new(BackgroundTaskSupervisor::new());
        assert!(bg.try_claim_indexer());

        let bg_clone = Arc::clone(&bg);
        bg.spawn_supervised(async move {
            // Wrap the panic in catch_unwind so the test runner doesn't
            // fail; the production path uses `tokio::task::spawn_blocking`
            // which catches the panic and surfaces it via `JoinError::is_panic`.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                panic!("simulated indexer panic");
            }));
            // Production supervisor closure ALWAYS calls release, even
            // after a panic (the `match join.await { ... }` arm runs
            // unconditionally before the closure returns).
            bg_clone.release_indexer();
        })
        .await;

        bg.drain(Duration::from_secs(1)).await;
        assert!(
            bg.try_claim_indexer(),
            "after panic + release, next claim must succeed"
        );
    }
}
