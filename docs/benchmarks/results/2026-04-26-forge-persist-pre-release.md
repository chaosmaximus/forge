# Forge-Persist — pre-release re-run — 2026-04-26

**Status:** PASS at HEAD `a9fa9af` (v0.6.0-rc.3).
**Bench:** `bash scripts/chaos/restart-drill.sh` (wraps `forge-bench forge-persist`).
**Predecessor:** [`forge-persist-2026-04-15.md`](forge-persist-2026-04-15.md) (11 days stale at re-run time).
**Hardware:** Linux x86_64, GCP `chaosmaximus-instance` (`6.8.0-1053-gcp`).
**Daemon binary:** `target/release/forge-daemon` (release profile, rebuilt at HEAD `a9fa9af`).
**Drill defaults:** seed=42 memories=10 chunks=0 fisp_messages=5 kill_after=0.5s recovery_timeout=5000ms worker_catchup=10000ms.

---

## 1. Summary

| Metric | Value | Threshold | Pass |
|--------|------:|----------:|:----:|
| `recovery_rate`           | **1.0000** | = 1.0  | ✓ |
| `consistency_rate`        | **1.0000** | = 1.0  | ✓ |
| `recovery_time_ms`        | 256        | < 5000 | ✓ |
| `total_ops`               | 15         | — | — |
| `acked_pre_kill`          | 7          | > 0 | ✓ |
| `recovered`               | 7          | = acked_pre_kill | ✓ |
| `matched`                 | 7          | = recovered | ✓ |
| Wall-clock                | 11098 ms   | — | — |
| Daemon version           | 0.6.0-rc.3 | — | — |

`composite = recovery_rate = 1.0` per the persist-bench shape (Tier 3
leaderboard treats forge-persist as a survival-probe with empty
`dimensions[]`; composite = recovery_rate).

## 2. Drill semantics

The chaos drill exercises crash-survivability by:

1. Starting forge-daemon as a subprocess.
2. Submitting `total_ops` writes (10 memories + 5 FISP messages — chunks
   skipped because raw-layer chunks require a MiniLM cold-load that adds
   ~6s wall-time without exercising additional persistence paths).
3. Waiting for `kill_after=0.5s` so a subset (`acked_pre_kill`) lands
   acked + flushed.
4. SIGKILL'ing the daemon (`Child::kill()` at `forge_persist.rs:1507-1532`).
5. Restarting the daemon and waiting `recovery_time_ms` for healthcheck.
6. Re-reading the database to verify `recovered` count matches
   `acked_pre_kill` and content matches (`consistency_rate`).

`recovery_rate=1.0` means every pre-kill ack survived; `consistency_rate=1.0`
means every recovered row's content matched its pre-kill payload.

## 3. Reproduction

```bash
cargo build --release
bash scripts/chaos/restart-drill.sh
# Or directly:
./target/release/forge-bench forge-persist \
    --seed 42 --memories 10 --chunks 0 --fisp-messages 5 \
    --kill-after 0.5 \
    --output /tmp/forge-restart-drill \
    --daemon-bin ./target/release/forge-daemon
```

## 4. Comparison vs 2026-04-15 baseline

| Metric | 2026-04-15 | 2026-04-26 | Δ |
|--------|-----------:|-----------:|--:|
| recovery_rate    | 1.0000 | 1.0000 | 0 |
| consistency_rate | 1.0000 | 1.0000 | 0 |
| recovery_time_ms | <5000  | 256    | well within budget |

No regression detected across 11 days of P3-1 + P3-2 + P3-3 changes,
including the P3-2 W7 SIGTERM handler addition (drill uses SIGKILL, so
SIGTERM path is not exercised here — see SIGTERM/SIGINT chaos drill
modes in deferred backlog).

## 5. Drill report excerpt

```
[forge-persist] === verdict ===
[forge-persist] total_ops=15 acked_pre_kill=7 recovered=7 matched=7
[forge-persist] recovery_rate=1.0000 consistency_rate=1.0000 recovery_time_ms=256
[forge-persist] wall_time_ms=11098 daemon_version=0.6.0-rc.3
[forge-persist] PASS
```

## 6. References

* Plan: `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5 W1)
* Predecessor result: `docs/benchmarks/results/forge-persist-2026-04-15.md`
* Stage 3 result: `docs/benchmarks/results/2026-04-26-restart-drill-stage3.md`
  (P3-3 Stage 3 close — first canonical drill report under the operator
  drill name).
* Implementation: `crates/daemon/src/bench/forge_persist.rs`
* Wrapper script: `scripts/chaos/restart-drill.sh`
* CLI subcommand: `crates/daemon/src/bin/forge-bench.rs::run_forge_persist`
