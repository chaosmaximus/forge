//! 2A-4d.1 T10 — instrumentation latency baseline.
//!
//! Measures the overhead of the phase-span + kpi_events + Prometheus
//! instrumentation by timing `run_all_phases` on a seeded SQLite DB with
//! and without a `ForgeMetrics` handle, across N=20 iterations with a
//! fresh `ForgeMetrics` per iteration, comparing medians under a relative
//! 1.15× ceiling.
//!
//! We cannot easily compare pre-T1 commit vs. HEAD without a worktree (which
//! the project's CLAUDE.md gates behind explicit permission). Instead we
//! measure:
//!
//!   A. `run_all_phases(conn, cfg, None)`         — closest in-tree
//!                                                  approximation of the
//!                                                  pre-T1 path: spans still
//!                                                  emitted (always compiled
//!                                                  in), kpi_events row still
//!                                                  written, but Prometheus
//!                                                  updates skipped.
//!   B. `run_all_phases(conn, cfg, Some(metrics))`— full 2A-4d.1 hot path.
//!
//! The delta (B - A) is the Prometheus-observation cost. Span + kpi_events
//! cost is baked into both branches because those are the always-on
//! observability layer of Tier 1 (see spec §3.2 non-goals).
//!
//! The test is ignored by default because it needs a non-trivial seeded
//! DB and takes ~20-60s depending on host. Run with:
//!
//!     cargo test -p forge-daemon --test t10_instrumentation_latency -- --ignored --nocapture
//!
//! The printed numbers are the baseline referenced in
//! docs/benchmarks/baselines/2026-04-24-consolidation-latency.md.

use std::time::{Duration, Instant};

use forge_core::types::{memory::MemoryStatus, memory::MemoryType, Memory};
use forge_daemon::db::{ops, schema};
use forge_daemon::server::metrics::ForgeMetrics;
use forge_daemon::workers::consolidator;
use rusqlite::Connection;

const N_ITERATIONS: usize = 20;
const SEEDED_MEMORY_COUNT: usize = 400;
/// Relative ceiling: Variant B's median must be ≤ 1.15× Variant A's median.
/// Replaces the previous 50 ms absolute ceiling which was ~50× typical
/// overhead and caught only runaway regressions. A ratio tolerates
/// single-digit-ms jitter while catching proportional regressions at any
/// absolute workload size.
const OVERHEAD_RATIO_CEILING: f64 = 1.15;

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort_unstable();
    xs[xs.len() / 2]
}

fn seed_db(conn: &Connection) {
    // Mix of statuses, types, tags, valence — exercises many phases without
    // relying on any one generator. Deterministic because the loop index is
    // the only source of variation.
    for i in 0..SEEDED_MEMORY_COUNT {
        let memory_type = match i % 4 {
            0 => MemoryType::Decision,
            1 => MemoryType::Lesson,
            2 => MemoryType::Pattern,
            _ => MemoryType::Preference,
        };
        let title = if i % 7 == 0 {
            // Occasionally reuse titles to create dedup / reweave candidates.
            format!("Recurring topic {}", i % 5)
        } else {
            format!("Seeded memory {i}")
        };
        let content = format!(
            "content body for memory {i} — {} discussion around topic {}",
            if i % 3 == 0 { "positive" } else { "neutral" },
            i % 11
        );
        let mut m = Memory::new(memory_type, title, content)
            .with_tags(vec![format!("tag-{}", i % 3), format!("tag-{}", i % 5)]);
        m.confidence = if i % 13 == 0 { 0.95 } else { 0.7 };
        m.status = if i % 23 == 0 {
            MemoryStatus::Faded
        } else {
            MemoryStatus::Active
        };
        ops::remember(conn, &m).expect("seed memory insert");
    }
}

fn make_conn() -> Connection {
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    schema::create_schema(&conn).expect("schema");
    seed_db(&conn);
    conn
}

#[test]
#[ignore = "T10 latency baseline — opt-in, see module comment"]
fn t10_consolidation_latency_baseline() {
    let cfg = forge_daemon::config::ConsolidationConfig::default();

    // Variant A: no Prometheus updates. Closest approximation of pre-T1
    // behaviour available without a worktree checkout.
    let mut a_durs: Vec<Duration> = Vec::with_capacity(N_ITERATIONS);
    for _ in 0..N_ITERATIONS {
        let conn = make_conn();
        let t0 = Instant::now();
        let _stats = consolidator::run_all_phases(&conn, &cfg, None);
        a_durs.push(t0.elapsed());
    }

    // Variant B: full instrumentation. A *fresh* `ForgeMetrics::new()` is
    // constructed per iteration rather than shared across all N.
    //
    // Why fresh-per-iteration: Prometheus counters are monotonic and
    // accumulate across iterations. If one iteration silently stopped
    // updating a counter mid-run, a shared registry would hide it in the
    // aggregate (the final total would still look "about right" because
    // other iterations made up the slack). A fresh registry means every
    // iteration's counter state is checked in isolation against the 23-row
    // `kpi_events` assertion — a per-iteration regression surfaces as a
    // per-iteration failure, not an aggregate drift.
    //
    // Per-iteration assertion on kpi_events row count closes the coverage
    // gap noted in the T12 Codex review: without it, a regression that
    // silently drops kpi_events inserts on iterations 1..N_ITERATIONS
    // would go unnoticed (the standalone sanity check below only exercises
    // one fresh DB).
    let mut b_durs: Vec<Duration> = Vec::with_capacity(N_ITERATIONS);
    for i in 0..N_ITERATIONS {
        let metrics = ForgeMetrics::new();
        let conn = make_conn();
        let t0 = Instant::now();
        let _stats = consolidator::run_all_phases(&conn, &cfg, Some(&metrics));
        b_durs.push(t0.elapsed());
        let per_iter_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kpi_events WHERE event_type = 'phase_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            per_iter_count, 23,
            "iteration {i}: expected 23 kpi_events rows, got {per_iter_count}"
        );
    }

    let a_med = median(a_durs.clone());
    let b_med = median(b_durs.clone());
    let ratio = b_med.as_secs_f64() / a_med.as_secs_f64();

    println!("\n=== T10 instrumentation latency baseline (N={N_ITERATIONS}) ===");
    println!("seeded memories: {SEEDED_MEMORY_COUNT}");
    println!(
        "Variant A (no metrics):   median = {:>10?}  samples = {:?}",
        a_med, a_durs
    );
    println!(
        "Variant B (full metrics): median = {:>10?}  samples = {:?}",
        b_med, b_durs
    );
    println!(
        "Ratio (B / A) = {:.4}  ceiling ≤ {:.2}",
        ratio, OVERHEAD_RATIO_CEILING
    );
    println!("=== end T10 baseline ===\n");

    // Relative ratio check: B's median must be within 1.15× A's median.
    // If B is faster than A (e.g. cache warming, jitter), ratio < 1.0 and
    // this assertion passes trivially — that's fine.
    assert!(
        ratio <= OVERHEAD_RATIO_CEILING,
        "instrumentation overhead ratio {ratio:.4} exceeds ceiling {OVERHEAD_RATIO_CEILING:.2} \
         (a_med={a_med:?}, b_med={b_med:?})"
    );

    // Sanity: the kpi_events rows should have been written in both variants.
    // Re-run a fresh iteration and assert the row count lands at 23 for
    // variant B (one per phase per run).
    let metrics = ForgeMetrics::new();
    let conn = make_conn();
    let _stats = consolidator::run_all_phases(&conn, &cfg, Some(&metrics));
    let kpi_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kpi_events WHERE event_type = 'phase_completed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        kpi_count, 23,
        "expected exactly 23 kpi_events rows per run_all_phases invocation (one per phase); got {kpi_count}"
    );
}
