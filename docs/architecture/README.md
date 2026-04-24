# Forge architecture — cross-cutting surfaces

This directory documents surfaces shared by multiple internal tiers.
Only add pages here when the shared surface otherwise has no single
owner — a file, crate, or protocol variant. For per-tier design, use
`docs/superpowers/specs/` and per-tier execution details in
`docs/superpowers/plans/`.

## Pages

- [`kpi_events-namespace.md`](./kpi_events-namespace.md) — registered
  `event_type` values in the `kpi_events` SQLite table + per-namespace
  `metadata_json` contracts. Any writer must claim its `event_type`
  namespace here before committing.
