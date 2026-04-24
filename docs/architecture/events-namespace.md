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

## Claiming a new `event` name

Open a PR that:

1. Adds a row to the table above.
2. Adds a `### <event_name> — event_schema_version: 1` section below.
3. Lands the emit call-site in the same PR so registry and emitter can't
   drift.
4. Includes a subscriber test or contract test pinning the payload shape.
