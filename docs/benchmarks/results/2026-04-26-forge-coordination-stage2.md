# forge-coordination Stage 2 — Calibration Results

**Phase:** P3-3 Stage 2 (2A-6 multi-agent coordination bench).
**Date:** 2026-04-26.
**Spec:** `docs/superpowers/specs/2026-04-26-multi-agent-coordination-bench-design.md` (v2.1 LOCKED).
**Bench commit:** `a7d08bd` (T7+T8 close).
**Hardware profile:** local (development host).
**Tooling:** `cargo build --workspace --features bench --bin forge-bench` debug.

## Summary

5/5 calibration seeds + dogfood seed=42 ALL converged at composite 1.0000
on the first run. **0 calibration cycles needed.** All 9 infrastructure
checks pass; all 6 dimensions hit 1.0000.

## Per-seed results

| Seed   | composite | inbox_precision | roundtrip | broadcast | authz | edge_case | pipeline | infra | wall_ms | verdict |
|--------|-----------|-----------------|-----------|-----------|-------|-----------|----------|-------|---------|---------|
| 7      | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 5       | PASS    |
| 13     | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 5       | PASS    |
| 42     | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 5       | PASS    |
| 100    | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 5       | PASS    |
| 1234   | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 6       | PASS    |
| 99999  | 1.0000    | 1.0000          | 1.0000    | 1.0000    | 1.0000| 1.0000    | 1.0000   | 9/9   | 5       | PASS    |

## Why all seeds converge identically

Spec §3.2 mandates that the corpus generator is fully formula-derived
from `(role, project, idx)` triples. The `_rng: &mut ChaCha20Rng`
parameter is taken for signature-consistency with other bench harnesses
but **not consumed**. As a consequence, `generate_corpus(seed=7)` and
`generate_corpus(seed=99999)` produce byte-identical output.

This is intentional and matches the 2A-5 forge-isolation precedent —
removing seed-dependent sampling eliminates one degree of cross-rustc-
version drift risk. The bench's value is in **structural correctness
verification**, not in randomness exploration; the seed parameter is
preserved for telemetry consistency (every `bench_run_completed` event
carries a seed) but plays no role in scoring.

## Per-dimension breakdown

All 6 dimensions hit their max scores on the green system:

| Dim | Probe | Max possible | Observed |
|-----|-------|--------------|----------|
| D1 inbox_precision           | 0 foreign rows / 50 max foreign per inbox × 6 inboxes | 1.0 | 1.0 |
| D2 roundtrip_correctness     | 70 sub-assertions (10 trials × 7) | 1.0 | 1.0 |
| D3 broadcast_project_scoping | 12 sub-assertions (4 trials × 3) | 1.0 | 1.0 |
| D4 authorization_enforcement | 15 sub-assertions (3 ack × 2 + 3 respond × 3) | 1.0 | 1.0 |
| D5 edge_case_resilience      | 7 probes (size/respond/broadcast/ack/sqli/etc) | 1.0 | 1.0 |
| D6 pipeline_chain_correctness | 18 sub-assertions (3 trials × 6) | 1.0 | 1.0 |

## Infrastructure checks (9/9)

| # | Check | Detail (seed=42) |
|---|-------|------------------|
| 1 | session_message_column_count | 14 columns (== SESSION_MESSAGE_COLUMN_COUNT const) |
| 2 | session_message_indexes_present | idx_msg_to + idx_msg_from + idx_msg_reply + idx_msg_meeting all present |
| 3 | session_table_columns_present | id, agent, project, status, started_at, organization_id |
| 4 | seeded_rng_deterministic | seeded_rng(42) reproduces same u64 |
| 5 | corpus_size_matches_spec | corpus has 6 sessions + 60 messages |
| 6 | session_distribution_correct | 3 roles × 2 projects = 6 sessions; 10 incoming each |
| 7 | pre_d1_total_count_60 | post-seed_corpus session_message count = 60 |
| 8 | send_message_returns_ulid | id len = 26 (synthetic from→to; SAVEPOINT-rolled-back) |
| 9 | respond_to_message_inverts_addressing | response row from↔to inverted, in_reply_to set correctly |

## Wall-clock observations

- 5-6 ms per seed (median 5).
- Spec target: ≤ 1500ms on ubuntu-latest. **300x headroom.**
- 7-bench CI matrix wall-clock impact: + ~60s (matches forge-isolation).

## Reproduction

```bash
# Build (debug ok; release recommended for tight wall-clock)
cargo build --workspace --features bench --bin forge-bench

# Single seed dogfood
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"
./target/debug/forge-bench forge-coordination --seed 42 \
    --output /tmp/forge_coord_dogfood --expected-composite 1.0

# 5-seed calibration sweep
for seed in 7 13 100 1234 99999; do
    ./target/debug/forge-bench forge-coordination --seed $seed \
        --output /tmp/forge_coord_seed_$seed --expected-composite 1.0
done

# Library tests (15/15 pass)
cargo test -p forge-daemon --lib --features bench bench::forge_coordination
```

## CI integration

`.github/workflows/ci.yml` `bench-fast` matrix:

```yaml
bench: [forge-consolidation, forge-identity, forge-isolation, forge-coordination]
```

Same `continue-on-error: true` rollout policy as the other in-process
benches. T17 promotion gate (P3-4 W2) covers all 4 entries together
once 14 consecutive green master runs accumulate.

## Stderr `[a2a] WARN` lines are EXPECTED

D4 (authorization_enforcement) deliberately calls `respond_to_message`
with a non-recipient caller. `sessions::respond_to_message` line 455
logs `eprintln!("[a2a] WARN: session ... tried to respond to message ...
addressed to ...")` before returning `Ok(false)`. The warning lines in
stderr CONFIRM that authorization enforcement is firing correctly —
silent rejection would be a regression.

```text
[a2a] WARN: session planner_alpha tried to respond to message <id> addressed to evaluator_alpha
```

CI parsers should not treat this as an error.

## Status

- ✅ Composite ≥ 0.95 on all 5 calibration seeds + dogfood (composite=1.0000)
- ✅ Every dim ≥ its min
- ✅ All 9 infrastructure assertions pass
- ✅ Wall-clock < 10ms (target ≤ 1500ms)
- ✅ Library tests 15/15 pass
- ✅ Clippy clean (workspace + tests + bench feature)
- ⏳ Adversarial impl review (T10) pending
