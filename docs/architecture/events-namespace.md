# Event bus namespace — `ForgeEvent.event` registry

The daemon's in-process event bus (`crates/daemon/src/events.rs`) multicasts
`ForgeEvent { event: String, data: serde_json::Value, timestamp: String }` via
`tokio::sync::broadcast` (capacity 256). Subscribers filter by exact-match
event name (comma-separated `?events=` query on `/api/subscribe`).

This file registers every event name actually emitted from production code
(verified at HEAD by grep) plus its payload contract, mirroring the pattern of
`kpi_events-namespace.md`. Add a row AND a section when you introduce a new
event name.

## Registry

| event name                     | Emitted by                                   | Purpose                                                              | `event_schema_version` |
|--------------------------------|----------------------------------------------|----------------------------------------------------------------------|------------------------|
| `consolidation`                | `workers/consolidator.rs::run_consolidator`  | End-of-pass summary of `ConsolidationStats` counters.                | _(unversioned — legacy)_ |
| `contradiction_detected`       | `workers/consolidator.rs::run_consolidator`  | One event per new contradiction surfaced by Phase 9.                 | _(unversioned — legacy)_ |
| `agent_status_changed`         | `server/handler.rs`                          | Team-agent state transition (spawned / running / retired).           | _(unversioned — legacy)_ |
| `hud_config_changed`           | `server/handler.rs`                          | HUD section / density config mutation.                               | _(unversioned — legacy)_ |
| `consolidate_pass_completed`   | `workers/consolidator.rs::run_all_phases`    | Phase 2A-4d.2: one event per consolidation pass (with run_id, trace_id, stats). | **1** |
| `bench_run_completed`          | `crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed` | After every forge-bench sub-bench run when FORGE_DIR is set | **1** |
| `session_idled`                | `workers/reaper.rs::reap_stale_sessions`     | Phase 2A-4d.3.1 #7: one event per session transitioned `active → idle` by a reaper pass. | **1** |

New emits that readers will consume (CLI, HUD, external dashboards) should
carry an `event_schema_version: <int>` field so the protocol can evolve. Older
emits don't yet carry versions; they are treated as v0-without-guarantees.

---

## `consolidate_pass_completed` — `event_schema_version: 1`

Emitted once at the tail of `run_all_phases`, inside the `consolidate_pass`
tracing span so `trace_id` reflects the pass. The reader for this event is
the HUD segment (`cons:23✓ 1.2s`) + any external SSE subscriber that wants
a lightweight "pass just completed" signal.

```json
{
  "event_schema_version": 1,
  "run_id": "01HXYZ...",
  "correlation_id": "01HXYZ...",
  "trace_id": null,
  "pass_wall_duration_ms": 1234,
  "phase_count": 23,
  "error_count": 0,
  "stats": { /* serialized ConsolidationStats — 20 usize fields */ }
}
```

Field reference:

- `event_schema_version` (int, required): `1`. Readers must check this.
- `run_id` (string, required): ULID assigned at `run_all_phases` entry.
- `correlation_id` (string, required): same as `run_id` today (Tier 1
  namespace contract); reserved for future cross-pass grouping.
- `trace_id` (string | null, required): hex OTLP trace id when
  `FORGE_OTLP_ENABLED=true`, else `null`. Matches the `trace_id` in every
  `kpi_events.metadata_json` row for this `run_id`.
- `pass_wall_duration_ms` (int, required): `Instant` elapsed from function
  entry to emit; excludes broadcast send overhead.
- `phase_count` (int, required): `PHASE_SPAN_NAMES.len()` — currently 23;
  moves only when a consolidator phase is added or removed.
- `error_count` (int, required): `SUM(COALESCE(json_extract(metadata_json,
  '$.error_count'), 0))` across this pass's `kpi_events` rows. Uses the
  `idx_kpi_events_timestamp` index (no full-table scan).
- `stats` (object, required): serialized `ConsolidationStats` — 20 `usize`
  counter fields (`exact_dedup`, `semantic_dedup`, `linked`, `faded`,
  `promoted`, `reconsolidated`, `embedding_merged`, `strengthened`,
  `contradictions`, `entities_detected`, `synthesized`, `gaps_detected`,
  `reweaved`, `scored`, `protocols_extracted`, `skills_inferred`,
  `antipatterns_tagged`, `healed_superseded`, `healed_faded`,
  `healed_quality_adjusted`). Serde derives added in T5.

### Version bump protocol

Any change a reader depends on bumps `event_schema_version`:

1. Increment to the next integer (e.g. `1 → 2`).
2. Add a new section below documenting the new contract.
3. Readers must branch on `event_schema_version` and handle both versions
   for at least one release cycle.
4. Dropping a version is a breaking change; announce in HANDOFF.

Additive fields inside `stats` are NOT a breaking change if readers parse
`stats` as a free-form object and ignore unknown fields. Renaming or
removing fields inside `stats` IS a breaking change.

---

## `bench_run_completed` — `event_schema_version: 1`

Emitted once at the tail of every `forge-bench` sub-bench run by
`crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed`. The
function is a no-op when `FORGE_DIR` is unset (visible via a one-shot
stderr note); otherwise it INSERTs one row into `kpi_events` with
`event_type = 'bench_run_completed'` keyed by a fresh ULID. Readers are
the Tier 2 `/inspect bench_run_summary` aggregator and the Tier 3
leaderboard reaper.

```json
{
  "event_schema_version": 1,
  "bench_name": "forge-identity",
  "seed": 42,
  "composite": 0.8423,
  "pass": true,
  "dimensions": [
    { "name": "identity_facet_persistence", "score": 0.91, "min": 0.80, "pass": true }
  ],
  "dimension_scores": { "identity_facet_persistence": 0.91 },
  "commit_sha": "e93d84b...",
  "commit_dirty": false,
  "commit_timestamp_secs": 1745452800,
  "hardware_profile": "ubuntu-latest-ci",
  "run_id": "01HXYZ...",
  "bench_specific_stats": { /* opaque, bench-specific JSON */ }
}
```

Field reference:

- `event_schema_version` (int, required): `1`. Readers must check this.
- `bench_name` (string, required): one of the canonical bench names —
  `forge-identity`, `forge-consolidation`, `forge-context`,
  `forge-persist`, `forge-isolation`, `longmemeval-<mode>`,
  `locomo-<mode>` (where `<mode>` is `raw` / `extract` etc., composed
  at the call site).
- `seed` (u64, required): the seed passed to the bench (`0` for the
  recall-probe benches `longmemeval` / `locomo` which don't take a seed).
- `composite` (f64 in `[0.0, 1.0]`, required): bench-specific composite
  score (e.g. weighted dimension sum for forge-identity /
  forge-consolidation / forge-context; `recovery_rate` for
  forge-persist; `mean_recall_at_5` for longmemeval; `mean_recall_at_10`
  for locomo).
- `pass` (bool, required): bench-defined PASS verdict.
- `dimensions` (array, required, possibly empty): each entry is
  `{ name: string, score: f64, min: f64, pass: bool }`. Empty for
  recall-probe benches.
- `dimension_scores` (object, required): convenience flat map
  `{ <dim_name>: score }` derived from `dimensions`. Empty when
  `dimensions` is empty.
- `commit_sha` (string | null, required): `GITHUB_SHA` if set, else
  `git rev-parse HEAD`, else `null`.
- `commit_dirty` (bool, required): `true` iff `git status --porcelain`
  is non-empty (best-effort; `false` when git is unavailable).
- `commit_timestamp_secs` (int | null, required): `git show -s
  --format=%ct HEAD` (best-effort; `null` when git is unavailable).
- `hardware_profile` (string, required): one of `ubuntu-latest-ci`,
  `macos-latest-ci`, `local`. Override via `FORGE_HARDWARE_PROFILE`;
  otherwise auto-detected from `GITHUB_ACTIONS` + `RUNNER_OS`.
- `run_id` (string, required): ULID; also used as the `kpi_events.id`
  primary key for this row.
- `bench_specific_stats` (object, required): opaque JSON — the
  bench-specific summary blob (e.g. forge-persist's full survival
  summary, longmemeval's per-type recall map). Readers MUST treat this
  as passthrough and not depend on its shape.

### Per-bench `dimensions[].name` registry

Ground truth = the actual emit code in
`crates/daemon/src/bin/forge-bench.rs`. Names are pinned per bench so
the Tier 3 leaderboard can index them. Renaming any of these is a v2
break (see "Version bump protocol" below).

| bench               | dim count | `dimensions[].name` values |
|---------------------|-----------|----------------------------|
| `forge-identity`        | 6 | `identity_facet_persistence`, `disposition_drift`, `preference_time_ordering`, `valence_flipping`, `behavioral_skill_inference`, `preference_staleness` |
| `forge-consolidation`   | 5 | `dedup`, `contradictions`, `reweave`, `lifecycle`, `recall_improvement` |
| `forge-context`         | 4 | `context_assembly`, `guardrails`, `completion`, `layer_recall` |
| `forge-isolation`       | 6 | `cross_project_precision`, `self_recall_completeness`, `global_memory_visibility`, `unscoped_query_breadth`, `edge_case_resilience`, `compile_context_isolation` |
| `forge-persist`         | 0 | _(survival probe — empty `dimensions`; composite = `recovery_rate`)_ |
| `longmemeval-<mode>`    | 0 | _(recall@K probe — empty `dimensions`; composite = `mean_recall_at_5`)_ |
| `locomo-<mode>`         | 0 | _(recall@K probe — empty `dimensions`; composite = `mean_recall_at_10`)_ |

The forge-identity names match the master v6 §4 fixed shape exactly.

**Discrepancy vs spec §3.3:** the spec lists `recall_delta` as
forge-consolidation's 5th dimension. The actual emit site
(`forge-bench.rs::run_forge_consolidation`) emits **`recall_improvement`**.
The code is ground truth; readers should use `recall_improvement`. The
spec will be reconciled in a follow-up.

### Version bump protocol

Adding a new bench to the `bench_name` enum (e.g. a future
`forge-coding`) is **additive** and stays at `event_schema_version: 1`
— readers that don't recognise the new bench simply skip it.

Renaming or removing a `dimensions[].name` for an **existing** bench is
a **breaking** change and bumps to `event_schema_version: 2`. The new
contract must be documented in a new section below and readers must
branch on the version field for at least one release cycle.

Adding a new dimension to an existing bench (extending the row above) is
a soft break: leaderboard readers that pivot on dimension name will see
a new column, but the row will still parse. Bump to v2 if any reader
asserts the dimension count.

### Reader compatibility

The Tier 2 `/inspect bench_run_summary` handler aggregates rows where
`event_type = 'bench_run_completed'`. It only consumes the top-level
fields (`bench_name`, `composite`, `pass`, `seed`, `commit_sha`) —
`bench_specific_stats` is treated as passthrough opaque JSON and never
introspected. This means bench authors can evolve their
`bench_specific_stats` shape freely without coordinating a v2 bump, as
long as no other reader has come to depend on its internal fields.

---

## `session_idled` — `event_schema_version: 1`

Emitted from `workers/reaper.rs::reap_stale_sessions` Phase 0, once per
session whose `last_heartbeat_at` has fallen behind the
`heartbeat_idle_secs` threshold (default 600s) but is still ahead of the
`heartbeat_timeout_secs` ended threshold (default 14400s). The reader
for this event is the HUD activity panel + any operator dashboard that
wants a live "session went quiet" signal without polling the `session`
table.

```json
{
  "event_schema_version": 1,
  "session_id": "01HXYZ...",
  "idle_secs": 600
}
```

Field reference:

- `event_schema_version` (int, required): `1`. Readers must check this.
- `session_id` (string, required): the ULID of the session that just
  transitioned `status = 'active' → 'idle'`. Matches `session.id`.
- `idle_secs` (int, required): the idle threshold (seconds) configured
  in `WorkerConfig.heartbeat_idle_secs` at the moment of the reaper
  pass. Surfaced for observers so they don't have to read config
  separately. **Note:** this is the threshold value, not the actual
  observed idle duration — derive that from `now - last_heartbeat_at`
  if needed.

### Lifecycle invariant

The reaper's two phases are sequenced atomically per pass:

1. **Phase 0** transitions `active → idle` via `UPDATE ... RETURNING id`,
   emitting `session_idled` for each returning row.
2. **Phase 1** transitions `(active|idle) → ended` via `UPDATE ...
   RETURNING id` (no event yet — open follow-up).

Because Phase 0 only matches sessions whose heartbeat is between
`idle_secs` and `timeout_secs`, a single reaper pass cannot fire
`session_idled` and immediately end the same session. A subsequent pass
may end it.

### Reviving an idle session

`update_heartbeat` (in `crates/daemon/src/sessions.rs`) atomically
revives an idle session back to `active` in the same UPDATE statement
that refreshes `last_heartbeat_at`. There is **no** corresponding
`session_revived` event today — the absence is intentional: revival is
a routine client heartbeat, not an operator-attention signal. If a
consumer needs revival visibility, file a follow-up.

### Version bump protocol

Same as the registry-wide protocol — increment `event_schema_version`
and add a new section if a reader-visible change ships.

---

## Claiming a new `event` name

Open a PR that:

1. Adds a row to the table above.
2. Adds a `### <event_name> — event_schema_version: 1` section below.
3. Lands the emit call-site in the same PR so registry and emitter can't
   drift.
4. Includes a subscriber test or contract test pinning the payload shape.
