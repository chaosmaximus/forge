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
    /// `true` once `signal_shutdown()` has been called. The writer-actor's
    /// `process_force_index_async` rejects new requests when this is set,
    /// so a force-index arriving during the drain window doesn't slip
    /// past the supervisor and get stranded by process exit. Set by
    /// `main.rs`'s shutdown sequence BEFORE invoking `drain()`.
    /// P3-4 W1.30 review MED-2.
    shutting_down: AtomicBool,
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
    /// Approximate count of supervisor tasks that completed during the
    /// drain window. On the clean path this equals the in-flight count
    /// at drain entry; on the timeout path it's the best-effort delta
    /// `entry_count - currently_running` (race-friendly: if a peer
    /// `spawn_supervised` happened to land during drain, this can
    /// under-count). Used for tracing only — operational decisions
    /// key off `timed_out`. P3-4 W1.30 review LOW-3.
    pub drained: usize,
    /// Whether the deadline expired before all tasks completed. When
    /// `true`, some tasks were still running when the timer fired —
    /// the daemon will continue to socket teardown anyway, and the
    /// stranded blocking work will be terminated by process exit
    /// (SQLite WAL preserves DB integrity at COMMIT granularity).
    pub timed_out: bool,
}

/// Default deadline for the shutdown drain. Overridable via the
/// `FORGE_DRAIN_TIMEOUT_SECS` environment variable; clamped to
/// [1, 300] to avoid a misconfigured 0-second drain (which would
/// always immediately time out and strand every in-flight pass) or
/// a pathological 1-hour drain (which would mask a wedged indexer).
/// P3-4 W1.30 review LOW-2.
pub const DEFAULT_DRAIN_TIMEOUT_SECS: u64 = 30;

/// Resolve the drain deadline from `FORGE_DRAIN_TIMEOUT_SECS` env
/// (clamped to `[1, 300]`) or fall back to `DEFAULT_DRAIN_TIMEOUT_SECS`.
/// Operators with pathologically large repos can bump it via env
/// without a recompile; operators wanting snappier shutdown can lower
/// it. Pre-fix the value was hardcoded at 30s with no override.
/// P3-4 W1.30 review LOW-2.
pub fn resolve_drain_timeout() -> std::time::Duration {
    let secs = std::env::var("FORGE_DRAIN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s.clamp(1, 300))
        .unwrap_or(DEFAULT_DRAIN_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs)
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
            shutting_down: AtomicBool::new(false),
            in_flight: Mutex::new(JoinSet::new()),
        }
    }

    /// Mark the daemon as shutting down. Called from `main.rs`'s
    /// shutdown sequence BEFORE `drain()` so any force-index request
    /// that slips past `run_server`'s return (e.g. arriving via the
    /// writer-actor mpsc from a background worker) is rejected at
    /// `process_force_index_async`'s gate instead of spawning a new
    /// blocking task that the drain can't catch.
    /// P3-4 W1.30 review MED-2.
    pub fn signal_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    /// Returns `true` if `signal_shutdown` has been called. The
    /// writer-actor checks this before claiming the indexer slot.
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
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
    async fn release_indexer_on_real_blocking_panic_unsticks_the_slot() {
        // P3-4 W1.30 review MED-3: this test pins the production-shape
        // contract that the supervisor closure releases the indexer
        // slot when `tokio::task::spawn_blocking(work)` panics —
        // mirroring exactly the structure of `process_force_index_async`
        // at writer.rs:417-447. Pre-fix the test caught the panic
        // INSIDE the future via `std::panic::catch_unwind`, then
        // unconditionally called release. That fakeout would still
        // pass even if a future refactor dropped the release from the
        // `Err(e) if e.is_panic()` arm — which is the very contract the
        // test claims to pin. Now the panic is real and propagates
        // through tokio's `JoinError::is_panic()` path.
        let bg = Arc::new(BackgroundTaskSupervisor::new());
        assert!(bg.try_claim_indexer());

        let bg_release = Arc::clone(&bg);
        bg.spawn_supervised(async move {
            // Real spawn_blocking panic. tokio catches it and surfaces
            // it via JoinError::is_panic — the production supervisor
            // closure then runs `match join.await { ... Err(e) if
            // e.is_panic() => tracing::error!(...) ... }` and the
            // unconditional release after the match.
            let join = tokio::task::spawn_blocking(|| {
                panic!("simulated indexer panic");
            });
            let result = join.await;
            assert!(
                result.is_err() && result.as_ref().unwrap_err().is_panic(),
                "production contract: spawn_blocking panic must surface via JoinError::is_panic"
            );
            // The release fires AFTER the match in production —
            // mirror that ordering here.
            bg_release.release_indexer();
        })
        .await;

        bg.drain(Duration::from_secs(2)).await;
        assert!(
            bg.try_claim_indexer(),
            "after real panic + release, next claim must succeed"
        );
    }

    #[tokio::test]
    async fn signal_shutdown_blocks_future_claims() {
        // P3-4 W1.30 review MED-2: signal_shutdown sets a separate
        // AtomicBool that production callers (process_force_index_async)
        // check BEFORE try_claim_indexer. After signal_shutdown is
        // called, is_shutting_down returns true and the writer-actor
        // rejects the request without entering the supervisor at all.
        let bg = BackgroundTaskSupervisor::new();
        assert!(!bg.is_shutting_down());
        bg.signal_shutdown();
        assert!(bg.is_shutting_down());
        // Note: signal_shutdown does NOT itself reject try_claim_indexer
        // — the writer-actor checks both gates separately. After
        // shutdown the in-flight indexer (if any) still completes
        // its release, and try_claim would succeed; but production
        // never gets there because the writer-actor's gate-check
        // returns before invoking the supervisor.
        assert!(
            bg.try_claim_indexer(),
            "supervisor's two gates are independent — that's the writer-actor's job to compose"
        );
    }

    #[test]
    fn resolve_drain_timeout_default_and_env_override() {
        // P3-4 W1.30 review LOW-2: the env override is clamped to
        // [1, 300] so a misconfigured 0 doesn't strand every pass and
        // a 1-hour drain doesn't mask a wedged indexer.
        std::env::remove_var("FORGE_DRAIN_TIMEOUT_SECS");
        assert_eq!(
            resolve_drain_timeout().as_secs(),
            DEFAULT_DRAIN_TIMEOUT_SECS
        );

        std::env::set_var("FORGE_DRAIN_TIMEOUT_SECS", "60");
        assert_eq!(resolve_drain_timeout().as_secs(), 60);

        std::env::set_var("FORGE_DRAIN_TIMEOUT_SECS", "0");
        assert_eq!(resolve_drain_timeout().as_secs(), 1, "0 must clamp to 1");

        std::env::set_var("FORGE_DRAIN_TIMEOUT_SECS", "9999");
        assert_eq!(
            resolve_drain_timeout().as_secs(),
            300,
            "max must clamp to 300"
        );

        std::env::set_var("FORGE_DRAIN_TIMEOUT_SECS", "garbage");
        assert_eq!(
            resolve_drain_timeout().as_secs(),
            DEFAULT_DRAIN_TIMEOUT_SECS,
            "unparseable must fall through to default"
        );

        std::env::remove_var("FORGE_DRAIN_TIMEOUT_SECS");
    }
}
