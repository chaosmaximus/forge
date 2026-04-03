# Forge Handoff

## Context for Next Session

Forge v0.4.0 is a local-first memory daemon for AI coding agents. The daemon (Rust, ~6000 LOC across 4 crates) is running in production on a GCP server with 331 tests and multiple Codex adversarial audits passed. The Mac app team is building a native macOS app (Notchy-style OS integration) in parallel — they pull from the same repo.

This session completed Waves 1-3 (daemon core, auto-extraction, code intelligence), the Phase 1 app team unblocks (project-scoped recall, health by project, event stream), and a major repo reorganization. The product pivoted from "Agentic OS" to "Memory Infrastructure + Intelligent Guardrails" — agent orchestration skills were dropped.

**Next up: Phase 2 of Track 1 — sqlite-vec migration.** This replaces hnsw_rs (in-memory HNSW, vectors lost on restart) and petgraph (in-memory graph) with sqlite-vec (vectors persist in SQLite) and SQL recursive CTEs (graph traversal). The plan is at `docs/plans/2026-04-03-track1-daemon-v1.md`. After Phase 2: guardrails engine (Phase 3), LSP client (Phase 4), multi-agent adapters (Phase 5).

## Key Files
- `docs/plans/2026-04-03-track1-daemon-v1.md` — full Track 1 plan (6 phases)
- `product/decisions.md` — all product decisions
- `product/daemon-requests.md` — Mac app team's requests
- `product/daemon-response.md` — our response to their requests
- `app/macos/API.md` — complete daemon API reference
- `DEPRECATED.md` — what's being phased out

## Active Branch
`master` — everything merged and pushed.

## Critical Context
- Codex Phase 1 review is still running (check `/codex:status`)
- Ollama is installed on the server: nomic-embed-text + qwen3:4b
- The daemon auto-starts on first `forge-next` command
- The v0.3.0 `forge` binary at `legacy/forge-core/` is still used by the indexer worker — gets replaced by LSP in Phase 4
- User preference: Opus for implementation, Codex for adversarial reviews, TDD on every layer, no redundant code
