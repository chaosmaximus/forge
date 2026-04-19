# Forge-Recency-Weighted-Decay (Phase 2A-4b) — Results

**Phase:** 2A-4b of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-19
**Parent design:** [docs/superpowers/specs/2026-04-18-forge-recency-weighted-decay-design.md]
**Parent master:** [docs/benchmarks/forge-identity-master-design.md §5 2A-4b]
**HEAD:** `19926cc` (T0→T15) + follow-up T17 results not committed, see bench artifacts
**Prior phase:** 2A-4a Valence Flipping shipped on 2026-04-18.

## Summary

**SHIPPED.** 2A-4b adds preference-type-dispatched recency decay, the `reaffirmed_at` anchor, the `<preferences>` CompileContext section, the `ReaffirmPreference` / `ComputeRecencyFactor` Request variants, and a structural simplification of the post-RRF recency weighting (`1.0 + recency_boost * 0.5` envelope → direct multiplicative factor).

Tests: **1294 lib + workspace tests passing** (up from 1277 at 2A-4a). Clippy clean. Fmt clean.

Regression-guard benchmarks (T16, T17) against prior phases:
- **Forge-Context (2A-2):** 5/5 seeds at **composite 1.0000** — zero regression from the T8 structural change.
- **Forge-Consolidation (2A-3):** 5/5 seeds at **composite 1.0000**, recall_delta **+0.2667** (identical to post-2A-3 baseline) — zero regression.

Live-daemon dogfood (HTTP):
- Remember(preference) → stored ✓
- ReaffirmPreference → `reaffirmed_at` updated with correct `YYYY-MM-DD HH:MM:SS` format ✓
- CompileContext pre-flip → `<preferences>` contains title ✓
- FlipPreference → `new_id` created, old marked flipped ✓
- CompileContext post-flip → `<preferences-flipped>` surfaces old pref ✓

## What shipped

| Task | Scope | Commit |
|------|-------|--------|
| T0 | `bench` Cargo feature declared across core + daemon | (pre-session) |
| T1 | Schema: `reaffirmed_at TEXT` column + migration | (pre-session) |
| T2 | Memory struct: `reaffirmed_at: Option<String>` with serde defaults, fetch-accessor audit | (pre-session) |
| T3 | `RecallConfig::preference_half_life_days` (default 14, validated 1..=365) | (pre-session) |
| T4 | `ops::recency_factor(memory, half_life, now_secs) -> f64` + `current_epoch_secs` | `3e4ce50` |
| T5 | `Request::ReaffirmPreference`/`ComputeRecencyFactor` variants + routing + contract tests | `c59728b` (+ `475e6fb` doc) |
| T6 | `ops::touch()` SQL exemption `AND memory_type != 'preference'` + 4-layer tests | `8fd0c17` + `4ff86df` |
| T7 | `decay_memories` type-dispatched (prefs half-life, non-prefs exp(-0.03×days)); hard-fade exempt for prefs | `8aa34a3` + `03ac96c` |
| T8 | `recall.rs` post-RRF envelope → direct multiplier via `apply_type_dispatched_recency` | `cdd3d93` |
| T9 | `ReaffirmPreference` handler with atomic `UPDATE ... RETURNING` | `7db4cf8` + `51f8c05` |
| T10 | 9 additional ReaffirmPreference validation + race tests (15 total) | `45eedfe` |
| T11 | `preference_reaffirmed` event emission post-commit | `a04538d` |
| T12 | `ComputeRecencyFactor` bit-exact parity handler (bench-feature-gated) | `f82148e` |
| T13 | `<preferences>` CompileContext section + `list_active_preferences` + L3 touch test re-enabled | `0ce5520` |
| T14 | `recency_decay_flow` integration test (end-to-end Remember→age→decay→Reaffirm→age-anchored→Flip) | `3b0ea52` |
| T15 | Schema rollback recipe test for `reaffirmed_at` | `19926cc` |
| T16 | Forge-Context regression-guard: 5/5 seeds 1.0000 composite | bench_results_context_2a4b_seed{1,2,3,42,100}/ |
| T17 | Forge-Consolidation regression-guard: 5/5 seeds 1.0000, recall_delta +0.2667 | bench_results_consolidation_2a4b_seed{1,2,3,42,100}/ |
| T18 | This results doc + live daemon dogfood | (this commit) |

## Decay formulas (canonical)

```
if memory.memory_type == Preference:
    anchor_ts = coalesce(reaffirmed_at, created_at)
    days      = max(0, now - anchor_ts) / 86400
    effective = confidence * 2^(-days / preference_half_life_days)
    # Hard-fade EXEMPT: status stays 'active' regardless of effective value
else:
    days      = max(0, now - accessed_at) / 86400
    effective = confidence * exp(-0.03 * days)
    if effective < 0.1: UPDATE memory SET status='faded'
```

Recall-side post-RRF (T8 replacement):
```
# Old (pre-2A-4b):
recency_boost = exp(-0.1 * days_since_created)
result.score *= 1.0 + recency_boost * 0.5   # envelope ∈ (1.0, 1.5]

# New (2A-4b):
result.score *= ops::recency_factor(memory, half_life, now)
# for non-prefs: exp(-0.1 * days_since_created) ∈ (0, 1]
# for prefs:     2^(-days_since_anchor / half_life) ∈ (0, 1]
```

## touch() exemption — SQL predicate

`ops::touch()` UPDATE now has `AND memory_type != 'preference'` appended. Preferences never update `accessed_at` via recall; their freshness is user/agent-controlled via `ReaffirmPreference` only.

4-layer regression coverage (T6):
- **L1**: `db::ops::tests::touch_exemption_*` direct unit
- **L2**: `tests/touch_exemption_recall.rs` through Request::Recall
- **L3**: `tests/touch_exemption_compile_context.rs` through Request::CompileContext (re-enabled in T13)
- **L4**: `tests/touch_exemption_batch_recall.rs` through Request::BatchRecall

## Bug found during review cycle (noted)

**T9 v1 (commit `7db4cf8`): cross-org access leak.** The first `ReaffirmPreference` handler had no `organization_id` scoping in its UPDATE SQL. Any caller could reaffirm any active preference across orgs. Post-UPDATE diagnostic SELECT was also unscoped, creating an existence-probing oracle. Fixed in `51f8c05` by mirroring FlipPreference's `get_session_org_id` pattern + 6th test `reaffirm_preference_rejects_cross_org_access`.

Note: FlipPreference carries the same architectural limitation — `caller_org` is derived from the target memory's own session, not authenticated caller context. Flagged as phase-wide follow-up for a future authenticated-session API (out of 2A-4b scope).

## Regression-guard results (reproducible)

### Forge-Context (T16)

```
seed 1:  composite=1.0000  (all 5 dimensions 1.0000)  PASS
seed 2:  composite=1.0000  PASS
seed 3:  composite=1.0000  PASS
seed 42: composite=1.0000  PASS
seed 100: composite=1.0000 PASS
```

Reproduce:
```bash
cargo build --release --bin forge-bench
for seed in 1 2 3 42 100; do
  ./target/release/forge-bench forge-context --seed $seed --output "bench_results_context_2a4b_seed${seed}"
done
```

### Forge-Consolidation (T17)

```
seed 1:  composite=1.0000  recall_delta=0.2667  (all 5 dims 1.0000)  PASS
seed 2:  composite=1.0000  recall_delta=0.2667  PASS
seed 3:  composite=1.0000  recall_delta=0.2667  PASS
seed 42: composite=1.0000  recall_delta=0.2667  PASS
seed 100: composite=1.0000 recall_delta=0.2667  PASS
```

Reproduce:
```bash
for seed in 1 2 3 42 100; do
  ./target/release/forge-bench forge-consolidation --seed $seed --output "bench_results_consolidation_2a4b_seed${seed}" --expected-recall-delta 0.20
done
```

Every consolidator phase runs identical workloads to the 2A-3 baseline (dedup removed 6, semantic-dedup merged 8, 4 valence + 4 content contradictions, 10 reweaves, etc.) — no behavioral divergence introduced.

## Live-daemon dogfood (T18)

Rebuild + restart at HEAD, then exercised the full 2A-4b surface via `POST /api`:

```bash
# Step 1: Remember preference
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"remember","params":{"memory_type":"preference","title":"dogfood-2a4b-pref","content":"testing","tags":["dogfood-2a4b"],"confidence":0.9}}'
# → {"status":"ok","data":{"kind":"stored","id":"01KPK7PP7EVQ6WTCZGCT3W48E7"}}

# Step 2: ReaffirmPreference
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"reaffirm_preference","params":{"memory_id":"01KPK7PP7EVQ6WTCZGCT3W48E7"}}'
# → {"status":"ok","data":{"kind":"preference_reaffirmed","memory_id":"...","reaffirmed_at":"2026-04-19 16:03:14"}}

# Step 3: CompileContext — <preferences> present
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"compile_context","params":{"static_only":false}}'
# → context contains <preferences> + title "dogfood-2a4b-pref"

# Step 4: FlipPreference
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"flip_preference","params":{"memory_id":"01KPK7PP7EVQ6WTCZGCT3W48E7","new_valence":"negative","new_intensity":0.8,"reason":"2A-4b dogfood"}}'
# → {"status":"ok","data":{"kind":"preference_flipped","old_id":"...","new_id":"01KPK7PPN0VVBGXXGS7SMQ665J","flipped_at":"2026-04-19 16:03:15"}}

# Step 5: CompileContext post-flip — <preferences-flipped> surfaces old
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"compile_context","params":{"static_only":false}}'
# → context contains <preferences-flipped> + "dogfood-2a4b-pref"
```

All 5 steps passed. Daemon version `0.4.0`, git_sha `19926cc` confirmed via `{"method":"version"}` endpoint.

## Next

**Phase 2A-4c** (Behavioral Skill Inference) and **Phase 2A-4d** (Forge-Identity Bench) remain. 2A-4c1 (session_tool_call schema) and 2A-4c2 (Phase 23) are the next feature sub-phases. 2A-4d (the bench) depends on 2A-4a + 2A-4b + 2A-4c shipped.

Known phase-wide follow-ups (not blocking):
- Authenticated caller-session parameter in write-path requests (affects FlipPreference + ReaffirmPreference cross-org guard quality)
- `load_config()` hot-path I/O in consolidator Phase 4 (codex v7 `LOW` severity)
- Lock `expected_recall_delta = 0.20` as CLI default for regression CI (noted since 2A-3 handoff)
