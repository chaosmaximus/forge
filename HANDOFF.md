# Forge Handoff

## Context for Next Session

Forge v0.4.0 is ship-ready. The daemon (Rust, ~6500 LOC across 4 crates) runs on a GCP server with 367 tests, zero clippy warnings, and multiple adversarial audits passed. Track 1 is complete except Phase 4 (LSP client, deferred to v0.5.0).

This session completed Phases 2, 3, 5, and 6:
- **Phase 2**: sqlite-vec migration (persistent vectors, removed hnsw_rs/petgraph/bincode)
- **Phase 3**: Guardrails engine (guardrails_check + blast_radius endpoints)
- **Phase 5**: Multi-agent adapters (Claude Code + Cline + Codex CLI)
- **Phase 6**: Final polish (docs, clippy, integration tests)

**Next up:** Ship v0.4.0 or start v0.5.0 work (LSP client for code intelligence, Cursor/Gemini/Windsurf adapters).

## Key Files
- `crates/daemon/src/adapters/` — AgentAdapter trait + 3 adapters
- `crates/daemon/src/guardrails/` — check.rs + blast_radius.rs
- `crates/daemon/src/db/vec.rs` — sqlite-vec vector operations
- `crates/daemon/src/recall.rs` — SQL-only hybrid recall
- `docs/plans/` — all implementation plans
- `product/daemon-response.md` — app team API reference

## Active Branch
`master` — everything merged and pushed.

## Critical Context
- sqlite-vec loaded via `init_sqlite_vec()` (std::sync::Once) before any Connection
- Guardrails `callers_count` is always 0 — no "calls" edges until LSP (Phase 4)
- Codex adversarial reviews keep failing (connection issue, not code) — Superpowers code-reviewer used instead
- User preference: Opus for implementation, Codex for adversarial reviews, TDD, no redundant code
