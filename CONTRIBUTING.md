# Contributing to Forge

Thanks for your interest in contributing to Forge! This document outlines the process for contributing and the conventions we follow.

## Code of Conduct

Be kind. Be respectful. Assume good intent. We're all here to build great cognitive infrastructure for AI agents.

## How to Contribute

1. **Search existing issues** before opening a new one — your question or bug might already be tracked.
2. **Discuss large changes first** — open a discussion or issue before sending a large PR.
3. **Fork, branch, PR** — standard GitHub flow. Branch off `master`.
4. **One logical change per PR** — easier to review, faster to merge.

## Development Setup

```bash
git clone https://github.com/chaosmaximus/forge.git
cd forge

# Build workspace
cargo build --workspace

# Run tests (990+ daemon tests)
cargo test --workspace

# Check formatting
cargo fmt --all -- --check

# Check for warnings (required: 0)
cargo clippy --workspace -- -W clippy::all -D warnings
```

### Requirements
- Rust 1.88+
- SQLite 3.40+ (bundled via `rusqlite`)
- macOS, Linux, or WSL

## Before Submitting a PR

Every PR must:
- [ ] Pass `cargo fmt --all -- --check` (no formatting diffs)
- [ ] Pass `cargo clippy --workspace -- -W clippy::all -D warnings` (0 warnings)
- [ ] Pass `cargo test --workspace` (all tests green)
- [ ] Include tests for new behavior
- [ ] Update relevant documentation in `docs/`
- [ ] Have a clear PR title and description explaining the "why"

## Code Conventions

### Rust
- Use `rustfmt` defaults
- Prefer `?` operator over `unwrap()` in library code
- Propagate errors with `anyhow::Result` for application code, typed errors for library code
- Use `tracing` for logging, never `println!` in non-test code
- Format strings: use inlined args (`format!("{x}")`, not `format!("{}", x)`)

### Testing
- Unit tests live in the same file as the code (`#[cfg(test)] mod tests`)
- Integration tests live in `crates/*/tests/`
- Use `tempfile::TempDir` for filesystem tests
- Don't mock SQLite — use an in-memory database

### Documentation
- Public items need `///` doc comments
- Include examples for non-trivial APIs
- Update `docs/api-reference.md` when adding protocol endpoints

## Architecture Overview

```
crates/
├── core/    — Protocol types (Request/Response enums)
├── daemon/  — HTTP server, memory engine, workers, guardrails
├── cli/     — forge-next CLI client
└── hud/     — Status line rendering
```

The daemon exposes a JSON-over-HTTP API at `localhost:8420/api`. The CLI and all integrations talk to this API. There is no shared state beyond the SQLite database.

See [docs/api-reference.md](docs/api-reference.md) for the full protocol.

## Reporting Bugs

Use [GitHub Issues](https://github.com/chaosmaximus/forge/issues) with:
- **What you expected** to happen
- **What actually happened** (including exact error messages)
- **How to reproduce** (minimal example)
- **Your environment**: OS, Rust version (`rustc --version`), Forge version

## Security

For security issues, please see [SECURITY.md](SECURITY.md). Do **not** file public issues for security vulnerabilities.

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
