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
- [`project-domain-lifecycle.md`](./project-domain-lifecycle.md) —
  contract for `reality.domain` values across bind paths
  (`project init`, `project detect`, `compile-context --cwd`
  auto-create) and the `"unknown"`→real-domain upgrade rule.
  Locked at P3-4 Wave X for v0.6.1 implementation.
- [`events-namespace.md`](./events-namespace.md) — registered
  daemon broadcast event types and their payload shapes.
