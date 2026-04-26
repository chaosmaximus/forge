# Adversarial review — 2A-6 forge-coordination impl

**Target diff:** `fbda6d8..f70955e` (5 commits)
**Spec:** `docs/superpowers/specs/2026-04-26-multi-agent-coordination-bench-design.md` (v2.1 LOCKED)
**Date:** 2026-04-26
**Reviewer:** claude (general-purpose)
**Verdict:** `lockable-as-is`

## Findings count

| Severity | Count |
|----------|-------|
| BLOCKER  | 0     |
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 0     |
| RESOLVED | 12    |

## Spec checkpoints — all pass

| # | Check | Verdict |
|---|-------|---------|
| 1 | §3.4 check 1: column count `== 14` named const | ✓ |
| 2 | §3.4 check 2: 4 indexes enumerated incl. idx_msg_meeting | ✓ |
| 3 | §3.4 SAVEPOINT for probes 8+9 (synthetic-row rollback) | ✓ |
| 4 | §3.7 single shared DaemonState | ✓ |
| 5 | §3.3 dim execution order D1→D2→D3→D4→D6→D5 | ✓ |
| 6 | §4 D11 sentinel-row pair-disjointness invariant | ✓ |
| 7 | §3.1 D1 runtime denominator (no hardcoded 50) | ✓ |
| 8 | §3.1a probe 1 substring match | ✓ |
| 9 | §3.1a probe 2 65536-byte boundary | ✓ |
| 10 | CLI parity with run_forge_isolation | ✓ |
| 11 | events-namespace.md registry alignment | ✓ |
| 12 | CI matrix entry for forge-coordination | ✓ |

## Implementation cross-references

- `crates/daemon/src/bench/forge_coordination.rs` — ~1100 lines. All 6 dimensions, 9 infrastructure checks, single-shared-DaemonState orchestrator, sentinel-row hash helper, 15 unit tests.
- `crates/daemon/src/bench/mod.rs:31` — `#[cfg(feature = "bench")] pub mod forge_coordination;` registered alongside forge_isolation.
- `crates/daemon/src/bin/forge-bench.rs:919-1006` — `run_forge_coordination` byte-for-byte mirror of `run_forge_isolation` modulo bench-name strings.
- `docs/architecture/events-namespace.md:124, 172` — bench_name list + per-bench dim registry row updated.
- `.github/workflows/ci.yml:186` — bench-fast matrix includes forge-coordination.
- `docs/benchmarks/results/2026-04-26-forge-coordination-stage2.md` — calibration table (5 seeds + dogfood = 6 PASS, composite=1.0000).

## Calibration / determinism (Pass 3)

End-to-end results doc claims 5/5 seeds + dogfood seed=42 ALL converged at composite=1.0000 PASS on first run. Verified by reading the bench's actual output captured in commit `a658811` results doc and matching against the 5-seed sweep at the same SHA.

## Notes (non-actionable)

The expected stderr `[a2a] WARN` lines from `sessions::respond_to_message:455` during D4 trials are EXPECTED (they confirm authorization-rejection is firing). The results doc explicitly documents this; CI parsers must not treat them as errors.

## Resolution

No fix-wave needed. Spec compliance + implementation quality both clean. Stage 2 close ready.
