# Forge Handoff

## Context for Next Session

Forge v0.4.0 is a local-first memory daemon for AI coding agents. The daemon (Rust, ~5500 LOC across 4 crates) is running in production on a GCP server with 331 tests and multiple adversarial audits passed. The Mac app team is building a native macOS app (Notchy-style OS integration) in parallel.

This session completed Phase 2 of Track 1 — the sqlite-vec migration. This replaced hnsw_rs (in-memory HNSW vectors, lost on restart) and petgraph (in-memory graph) with sqlite-vec (persistent vectors in SQLite) and SQL recursive CTEs (graph traversal). The migration removed 870 lines and added 172, simplified DaemonState from 5 fields to 3, and eliminated 3 crate dependencies (hnsw_rs, petgraph, bincode). All 331 workspace tests pass with zero clippy warnings.

**Next up: Phase 3 — Guardrails Engine.** This adds the second product pillar: `guardrails_check` (pre-execution checks querying the knowledge graph for decisions linked to affected files) and `blast_radius` (impact analysis). The plan is at `docs/plans/2026-04-03-track1-daemon-v1.md`.

## Key Files
- `docs/plans/2026-04-03-track1-daemon-v1.md` — full Track 1 plan (6 phases)
- `crates/daemon/src/db/vec.rs` — NEW: sqlite-vec vector operations (store, search, has, count, delete)
- `crates/daemon/src/recall.rs` — REWRITTEN: SQL-only hybrid recall (BM25 + vec + graph via RRF)
- `product/decisions.md` — all product decisions
- `product/daemon-response.md` — Mac app team's API reference

## Active Branch
`master` — everything merged and pushed.

## Critical Context
- sqlite-vec 0.1.9 loaded via `sqlite3_auto_extension` (must be called before opening any Connection)
- `init_sqlite_vec()` uses `std::sync::Once` — safe to call multiple times, all test helpers call it
- store_embedding uses `unchecked_transaction()` for atomic DELETE+INSERT (review fix)
- sql_neighbors limited to 10 per node (review fix for fan-out explosion)
- Edge table FK declarations removed (edges point to `file:xxx` targets, not just memory rows)
- Codex adversarial review was attempted but connection dropped — Superpowers code-reviewer gave APPROVED
- User preference: Opus for implementation, Codex for adversarial reviews, TDD on every layer, no redundant code
