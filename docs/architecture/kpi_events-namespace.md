# `kpi_events` namespace register

`kpi_events` (`crates/daemon/src/db/schema.rs:255-266`) is a shared
observation log. This page is the coordination surface for every writer.

## Table shape (reference)

```sql
CREATE TABLE IF NOT EXISTS kpi_events (
    id TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    project TEXT,
    latency_ms INTEGER,
    result_count INTEGER,
    success INTEGER NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_kpi_events_timestamp ON kpi_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_kpi_events_type      ON kpi_events(event_type);
```

`metadata_json` is a free-form string at the DB level — each
`event_type` defines its own JSON schema. **Every `metadata_json`
payload MUST include an `event_schema_version` integer so readers can
version-validate.**

> **Field-name compat note (2026-04-25):** Tier 1's `phase_completed`
> shipped with the field name `metadata_schema_version`. From Tier 2
> onward (`consolidate_pass_completed`) and Tier 3 onward
> (`bench_run_completed`), the canonical field is `event_schema_version`.
> Readers MUST accept both names — check `event_schema_version` first,
> fall back to `metadata_schema_version` for legacy `phase_completed`
> rows. New `event_type` registrations use `event_schema_version`.

## Claimed `event_type` values

| `event_type` | Owner | Claimed | Contract version | Metadata schema |
|--------------|-------|---------|------------------|-----------------|
| `phase_completed` | consolidator (Tier 1 of 2A-4d) | 2026-04-24 | `metadata_schema_version: 1` (legacy) | see below |
| `bench_run_completed` | bench telemetry (Tier 3 of 2A-4d) | 2026-04-25 | `event_schema_version: 1` | see below |

### `phase_completed` — metadata_schema_version: 1

Emitted by `crates/daemon/src/workers/instrumentation.rs::record`
after every consolidator phase under `run_all_phases`. One row per
phase per consolidate_pass.

```jsonc
{
  "metadata_schema_version": 1,

  // Span metadata
  "phase_name": "phase_23_infer_skills_from_behavior", // see PHASE_SPAN_NAMES
  "run_id": "01HXYZ...",                                // ULID for the outer consolidate_pass
  "correlation_id": "01HXYZ...",                        // same as run_id today; kept separate for future use

  // Populated only when OTLP is enabled; hex OTLP trace id (32 chars)
  "trace_id": "abc123..." | null,

  // Phase accounting
  "output_count": 1,                                    // phase-specific, see PhaseOutputProjection §3.1a
  "error_count": 0,                                     // sum of recoverable failures observed inside the phase
                                                        // (e.g. per-row insert/rollback); 0 means success.
                                                        // Tier 1 T13.1 landed `_with_errors` variants for 11
                                                        // helpers so swallowed inner errors surface here
                                                        // honestly; there are no remaining `.unwrap_or(0)`
                                                        // sites inside `run_all_phases` body today.
                                                        // **Instrumentation-layer failures** (kpi_events insert
                                                        // blew up, ULID collision absorbed) do NOT contribute
                                                        // here — they land in
                                                        // `forge_phase_persistence_errors_total{kind}` only,
                                                        // so phase-level SLOs stay untainted by persistence
                                                        // hiccups.

  // Phase-specific payload
  "extra": { /* keys vary by phase; stable per phase */ }
}
```

Column mappings:

- `latency_ms` = `duration_ms` of the phase.
- `result_count` = `output_count` (fast roll-up without JSON parsing).
- `success` = `1` if `error_count == 0`, else `0`.
- `project` = `NULL` (phases are project-agnostic; per-project counts
  come from per-table gauges).
- `id` = `phase-<ULID>` (prefix identifies provenance without a join).

### `bench_run_completed` — event_schema_version: 1

Emitted by `crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed`
at the tail of every `forge-bench <subcmd>` invocation, when `FORGE_DIR`
is set. One row per bench invocation. The Tier 3 `/inspect bench_run_summary`
shape aggregates these rows for the leaderboard surface.

```jsonc
{
  "event_schema_version": 1,

  // Bench identity
  "bench_name": "forge-identity" | "forge-consolidation" | "forge-context"
              | "forge-persist" | "longmemeval-<mode>" | "locomo-<mode>",
  "seed": 42,                                 // u64 PRNG seed

  // Score
  "composite": 0.8490,                        // f64 in [0.0, 1.0]
  "pass": false,                              // composite ≥ threshold AND
                                              //   per-dim minimums met
  "dimensions": [                             // per-dim breakdown; may be empty
    { "name": "identity_facet_persistence",
      "score": 1.0000, "min": 0.85, "pass": true }
    // ... see events-namespace.md §"Per-bench dimensions[].name registry"
    //     for the canonical name list per bench
  ],
  "dimension_scores": {                       // flat name→score map (same data,
    "identity_facet_persistence": 1.0000     //   easier for SQL extractors)
    // ...
  },

  // Provenance
  "commit_sha": "abc123..." | null,           // GITHUB_SHA → git rev-parse → null
  "commit_dirty": false,                      // `git status --porcelain` non-empty
  "commit_timestamp_secs": 1714080000 | null,
  "hardware_profile": "ubuntu-latest-ci" | "macos-latest-ci" | "local",
  "run_id": "01HXYZ...",                      // ULID; matches column `id`

  // Bench-specific opaque blob — readers branch on `bench_name` for shape
  "bench_specific_stats": { /* varies per bench */ }
}
```

Column mappings:

- `latency_ms` = `wall_duration_ms` of the bench run.
- `result_count` = `dimensions.len()` as i64 (0 for survival/recall-probe benches).
- `success` = `1` if `pass` is true, else `0`.
- `project` = `NULL` (benches are project-agnostic).
- `id` = the `run_id` ULID (no prefix; mirror of `metadata_json.run_id`).
- `event_type` = `'bench_run_completed'` literal.

**Retention:** the kpi_events reaper applies the per-event-type
override `kpi_events_retention_days_by_type` from `WorkerConfig`,
which defaults to `{"bench_run_completed": 180}`. Other event_types
keep the global 30-day default. See `crates/daemon/src/workers/kpi_reaper.rs`.

**Reader surface:** Tier 2's `/inspect bench_run_summary` shape (180-day
window cap per D8) consumes top-level fields (`bench_name`, `composite`,
`pass`, `seed`, `commit_sha`, `dimension_scores.*`). `bench_specific_stats`
is opaque pass-through.

**Cross-reference:** the broadcast-bus event registry at
`docs/architecture/events-namespace.md` documents the same payload from
the broadcast/SSE-consumer perspective and registers per-bench
`dimensions[].name` values. This SQL register is the authoritative one
for `kpi_events` row writers.

### Version bump protocol

Any change to the `metadata_json` contract that a reader depends on:

1. Bump `event_schema_version` (or `metadata_schema_version` for legacy
   `phase_completed`) to the next integer.
2. Update this register with the new contract section.
3. Readers must branch on the version field and handle both the old and
   new versions for at least one release cycle.
4. Dropping a version is a breaking change; announce in HANDOFF
   §Lifted constraints.

**Additive-only changes** (new optional field, new bench enum value)
do NOT bump the version. Renames or removals do.

## Claiming a new `event_type`

Open a PR that:

1. Adds a row to the table above.
2. Adds a `### <event_type> — metadata_schema_version: 1` section
   below, documenting the full JSON contract.
3. Includes the writer (`INSERT INTO kpi_events`) in the same PR so
   the table and writer land atomically.

Do NOT write a new `event_type` without registering it first; the
register is the coordination mechanism that prevents two writers from
colliding on the same namespace with incompatible payloads.
