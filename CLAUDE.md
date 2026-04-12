# Forge — Cognitive Infrastructure for AI Agents

## Project

Forge gives AI agents persistent memory, proactive context, and self-healing intelligence. One Rust daemon, one SQLite file, zero cloud dependency.

**Stack:** Rust daemon (4 crates in `crates/`) — open-source Apache-2.0
**Port:** Daemon HTTP API on `8420` — `POST /api` with `{method, params}` JSON
**Tests:** `cargo test --workspace` (1,245+ passing)
**Lint:** `cargo clippy --workspace -- -W clippy::all -D warnings` (0 warnings required)

## Architecture

```
crates/daemon/     — HTTP server, 8-layer Manas memory, 8 background workers, guardrails
crates/cli/        — forge-next CLI client (talks to daemon via HTTP)
crates/core/       — Protocol types (Request/Response enums shared between daemon and cli)
crates/hud/        — StatusLine rendering
config/            — Sample daemon configuration
deploy/            — Docker, Docker Compose, Helm charts, Litestream, Grafana dashboards
docs/              — User-facing documentation (getting-started, api-reference, cli-reference, security, operations)
scripts/           — Install scripts, systemd/launchd unit files
tests/             — Integration test scripts
.github/workflows/ — CI (fmt + clippy + tests on macOS + Linux), release (multi-arch binaries)
```

## Conventions

- **Protocol**: Some endpoints are unit variants (no params: `health`, `healing_status`, `healing_run`, `doctor`, `license_status`, `sync_conflicts`, `list_team_templates`). Others require `params: {}` even if empty.
- **Error handling**: `anyhow::Result` in application code. Typed errors in library code. Never `unwrap()` outside tests.
- **Format strings**: Inlined args (`format!("{x}")`, not `format!("{}", x)`) — enforced by clippy.
- **Tracing**: Use `tracing::info!` / `tracing::warn!` / `tracing::error!`. Never `println!` in non-test code.
- **Tests**: `#[cfg(test)] mod tests` in the same file. `tempfile::TempDir` for filesystem tests. In-memory SQLite for unit tests, real file for integration.

## Development Workflow

```bash
# Build + test everything
cargo build --workspace
cargo test --workspace

# Lint (must be 0 warnings)
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings

# Run daemon locally
cargo run --release -p forge-daemon

# Use the CLI against a running daemon
cargo run --release -p forge-cli -- health
cargo run --release -p forge-cli -- recall "architecture"
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide. In short:

- Fork, branch, PR
- One logical change per PR
- Every PR must pass fmt + clippy + tests
- Add tests for new behavior
- Update docs in `docs/` when relevant

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
