# Forge — Agentic OS for Claude Code

## Project Overview

Forge is a Claude Code plugin providing production-grade agent team orchestration with a unified knowledge graph (code + memory + secrets), Rust-powered hot paths, and session channels (Telegram/iMessage).

## Architecture

**Hybrid Rust + Python + TypeScript:**
- **forge-core** (Rust): tree-sitter code indexer, HUD renderer, hook handlers, security scanner. Single binary, <5ms hot paths.
- **forge-graph** (Python): MCP server backed by LadybugDB 0.15.3. Memory tools (8), security scanner, evolution engine. 101+ tests.
- **forge-channel** (TypeScript/Bun): MCP channel servers for Telegram/iMessage.
- **forge-hud** (Rust): StatusLine binary, <2ms render, reads hud-state.json.

## Development

### Running Tests

```bash
# Python tests (ALWAYS use PYTHONPATH=src)
cd forge-graph && PYTHONPATH=src python3 -m pytest tests/ -v --tb=short

# Rust build
cargo build --release -p forge-core
cargo build --release -p forge-hud

# Run forge-core indexer
./target/release/forge-core index forge-graph/src/
```

### Critical: Python Version

This system runs on **Python 3.10**. ALWAYS use `python3`, never `python`.

Type hints in `@mcp.tool()` decorated functions MUST use `Optional[str]` and `Dict[str, Any]` from typing — NOT `str | None` or `dict[str, Any]`. The MCP SDK's `issubclass()` check fails on PEP 604/585 syntax with `from __future__ import annotations`.

Internal (non-MCP-decorated) functions can use modern syntax freely.

### LadybugDB Notes

- `real_ladybug` 0.15.3 — Python bindings. Use `current_timestamp()` not `timestamp()`.
- `kuzu` Rust crate is v0.11.3 — INCOMPATIBLE with v0.15.3 databases (storage format mismatch, C++ simsimd build failure). Do NOT attempt Rust graph DB access until the crate catches up.
- Schema uses `IF NOT EXISTS` for idempotency.
- The Secret table uses `status` column (active/rotated/revoked), NOT `invalid_at` like other memory nodes.

### Hook Scripts

Scripts in `scripts/` are called by Claude Code via `hooks/hooks.json`. They delegate to Python:
```
scripts/forge-graph-start.sh    → python3 -m forge_graph.hooks.session_start
scripts/session-end-graph.sh    → python3 -m forge_graph.hooks.session_end
scripts/post-edit-enhanced.sh   → python3 -m forge_graph.hooks.post_edit + regex secret scan + code formatting
```

### Plugin Cache

The installed plugin cache at `~/.claude/plugins/cache/forge-marketplace/forge/0.2.0/` is a STALE COPY. Changes to repo scripts don't take effect until the plugin is reinstalled. For development, test scripts directly from the repo.

## File Structure

```
forge-core/          Rust binary (tree-sitter indexer + future hot paths)
forge-graph/         Python MCP server (LadybugDB, memory, security, evolution)
forge-hud/           Rust HUD binary (statusLine renderer)
forge-channel/       TypeScript channel bridges (Telegram, iMessage)
hooks/               hooks.json (Claude Code hook configuration)
scripts/             Bash hook scripts (thin wrappers to Python/Rust)
agents/              Agent definitions (.md) for planner/generator/evaluator
skills/              Skill definitions (.md) for forge workflows
```

## Design Documents

- Spec: `docs/superpowers/specs/2026-04-02-forge-v0.2.0-unified-graph-design.md`
- Plan: `docs/superpowers/plans/2026-04-02-forge-v0.2.0-agentic-os.md`

## Known Issues

- `axoniq` listed in pyproject.toml dependencies but doesn't exist on PyPI (replaced by forge-core Rust indexer)
- task-completed-gate.sh fails when run from monorepo root (PYTHONPATH not set)
- servers/forge-graph binary referenced in plugin.json doesn't exist yet (needs PyInstaller or pip install)

## Security

- All Cypher queries use parameterized `$param` syntax — never string interpolation
- `axon_cypher` sandbox blocks memory node labels + write keywords
- Per-agent ACL enforcement via `agent_id`
- Hook scripts validate paths, resolve symlinks, check workspace boundaries
- Secret scanner NEVER stores actual secret values — fingerprint only
- Evolution engine writes to isolated git worktrees, path-restricted to skills/
