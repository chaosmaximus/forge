<h1 align="center">Forge</h1>

<p align="center">
  <strong>Cognitive infrastructure for AI agents.</strong>
</p>

<p align="center">
  Persistent memory. Intelligent guardrails. Self-healing knowledge graph.<br/>
  One daemon. Any agent. Install once, remember everything.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-Apache%202.0-blue" alt="License" />
  <img src="https://img.shields.io/badge/tests-1%2C245%2B%20passing-brightgreen" alt="Tests" />
  <img src="https://img.shields.io/badge/endpoints-98-blue" alt="Endpoints" />
  <img src="https://img.shields.io/badge/workers-8-orange" alt="Workers" />
  <img src="https://img.shields.io/badge/memory%20layers-8-purple" alt="Memory Layers" />
  <img src="https://img.shields.io/badge/rust-1.88-orange" alt="Rust" />
</p>

---

## The Problem

AI agents forget everything. Every session starts from zero. You explain your auth strategy, your database schema, your deployment pipeline — and next session, it's gone. You maintain `MEMORY.md` files by hand. You copy-paste context. You repeat yourself endlessly.

Your agent has the reasoning power of a senior engineer and the memory of a goldfish.

## The Solution

Forge is an always-on daemon that gives AI agents persistent memory, intelligent guardrails, and a self-healing knowledge graph. Install it, bootstrap from your existing transcripts, and your agent remembers everything — across sessions, across projects, across machines.

**Install. Bootstrap. 100 memories in 60 seconds. All local. All yours.**

---

## Quick Start

```bash
# 1. Install from source
cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli

# 2. Start the daemon (runs on port 8420)
forge-daemon &

# 3. Bootstrap from your existing Claude Code / Codex / Cursor transcripts
forge-next bootstrap

#    ███████████████████████████░░░  Processing 47 sessions...
#    ✓ 143 memories extracted in 58s

# 4. Search your memory
forge-next recall "auth"

#    ╭──────────────────────────────────────────────────╮
#    │ [decision] Use JWT with RS256 signing keys       │
#    │ project: api-server  confidence: 0.94            │
#    │ Rotating keys stored in Vault, 24h expiry.       │
#    │ 3 linked files · 2 related decisions             │
#    ╰──────────────────────────────────────────────────╯
```

The daemon runs in the background. It extracts memories from every agent session, builds a knowledge graph, and serves context via HTTP at `localhost:8420/api` when agents need it.

---

## Architecture

```
┌──────────────────────┐
│ Your Agent           │
│ (Claude, Codex, etc.)│
└──────────┬───────────┘
           │ HTTP /api
           ▼
┌──────────────────────────────────────────┐
│ forge-daemon  (Rust · port 8420)         │
│                                          │
│  • 98 protocol endpoints                 │
│  • 8-layer Manas memory engine           │
│  • 8 background workers                  │
│  • Guardrails engine (blast radius)      │
│  • Identity · Disposition · Skills       │
│  • SQLite FTS5 + sqlite-vec              │
│  • Self-healing sleep-cycle consolidation│
└──────────────────────────────────────────┘
```

**The agent never writes to memory.** Extraction happens silently in the background. The agent only needs to recall. The graph grows automatically.

---

## Features

<table>
<tr>
<td width="50%" valign="top">

### Memory
- **8-layer Manas memory** (platform · tool · skill · domain_dna · experience · perception · declared · latent) + entity/edge knowledge graph, with SQLite FTS5 + vector search
- **Auto-extraction** from agent transcripts (zero manual tagging)
- **Multi-provider** — Ollama (local), Claude, OpenAI, Gemini
- **Bootstrap** — 100+ memories from existing transcripts in 60s
- **Cross-session** — decisions persist across sessions and projects
- **Semantic search** — BM25 + vector + graph traversal via RRF
- **Multi-tenant isolation** — organization_id scoping on all queries

</td>
<td width="50%" valign="top">

### Guardrails
- **Blast radius analysis** — know what breaks before you edit
- **Decision tracking** — files linked to architectural decisions
- **Secret scanning** — SHA256 fingerprints, never stores values
- **Pre-edit warnings** — inline alerts for high-impact changes
- **Memory-aware** — guardrails query the knowledge graph
- **Cross-file consistency** — detects when edits break callers

</td>
</tr>
<tr>
<td width="50%" valign="top">

### Intelligence & Identity
- **Behavioral pattern learning** — learns HOW you think, not just what you did
- **Agent persona** — role, expertise, values per agent
- **Disposition engine** — slow-changing traits from session evidence
- **Tool intelligence** — discovers 50+ tools, surfaces the right one in context
- **Memory valence** — positive/negative emotional weighting
- **Reconsolidation** — memories evolve when recalled

</td>
<td width="50%" valign="top">

### Infrastructure
- **Persistent daemon** — launchd/systemd, starts at boot
- **8 background workers** — continuous ambient processing
- **Self-healing graph** — sleep-cycle consolidation overnight
- **Predictive prefetch** — zero cold-start context injection
- **Memory sync** — encrypted peer-to-peer with cross-tier sync policies
- **Event stream** — 12 real-time event types for UI integration
- **Session KPIs** — per-session observability
- **A2A message notifications** — pending messages injected into context

</td>
</tr>
</table>

---

## Manas: 8-Layer Memory

| # | Layer | What It Stores | How It Grows |
|---|-------|---------------|-------------|
| 1 | **Platform** | OS, CPU, shell, hostname | Auto-detected at startup |
| 2 | **Tool** | Available tools, APIs, CLIs | Auto-detected, 50+ tools |
| 3 | **Skill** | Workflows + behavioral patterns | Extracted from sessions |
| 4 | **Domain DNA** | Project conventions | Detected from codebase structure |
| 5 | **Experience** | Decisions, lessons, patterns | LLM extraction from transcripts |
| 6 | **Perception** | Git state, file changes | Perception worker (30s cycle) |
| 7 | **Declared** | CLAUDE.md, README, docs | Ingested from project files |
| 8 | **Latent** | Embedding vectors | Embedder worker (60s cycle) |

Plus: **Identity engine** (agent persona), **Disposition engine** (behavioral traits), **Proactive intelligence** (7 hook points with context-budgeted output).

---

## CLI Reference

```bash
# Search your memory
forge-next recall "database schema"
forge-next recall "auth" --project api-server --limit 5
forge-next recall "deployment" --layer skill

# Store a decision
forge-next remember --type decision --title "Use PostgreSQL" \
  --content "Chose Postgres over MySQL for JSON support and pg_vector"

# Bootstrap from existing transcripts
forge-next bootstrap

# Check before editing
forge-next check --file src/auth/middleware.rs
forge-next blast-radius --file src/auth/middleware.rs

# Health & diagnostics
forge-next health
forge-next manas-health              # all 8 layers
forge-next doctor                    # full system diagnostics

# Identity
forge-next identity set --facet role --description "Senior Rust developer"
forge-next identity list

# Sync across machines
forge-next sync-push workstation --project myproject
forge-next sync-pull laptop --project myproject

# System
forge-next sessions                  # active agent sessions
forge-next perceptions               # current git state
forge-next platform                  # system info
forge-next tools                     # detected tools
```

See [docs/cli-reference.md](docs/cli-reference.md) for the complete command reference.

---

## Works With Any Agent

Forge is infrastructure, not a plugin. Thin adapters teach each agent to recall. The daemon extracts from all of them simultaneously.

| Agent | Extraction | Recall | Status |
|-------|-----------|--------|--------|
| **Claude Code** | Automatic | Native | Shipped |
| **Codex CLI** | Automatic | Native | Shipped |
| **Cursor** | Automatic | Via MCP | Shipped |
| **Cline** | Automatic | Via MCP | Shipped |
| **Gemini CLI** | Automatic | Native | Planned |

**One knowledge graph. All your agents. Shared memory.**

---

## Build From Source

```bash
git clone https://github.com/chaosmaximus/forge.git
cd forge

# Build workspace (release mode)
cargo build --release --workspace

# Run the full test suite (990+ daemon tests)
cargo test --workspace

# Check for warnings (required: 0)
cargo clippy --workspace -- -W clippy::all -D warnings

# Install binaries to ~/.cargo/bin
cargo install --path crates/daemon
cargo install --path crates/cli
```

### Requirements
- Rust 1.88+
- SQLite 3.40+ (bundled via `rusqlite`)
- macOS, Linux, or WSL

---

## Documentation

| Doc | Contents |
|-----|----------|
| [Getting Started](docs/getting-started.md) | Install, bootstrap, first queries |
| [API Reference](docs/api-reference.md) | All 98 HTTP endpoints |
| [CLI Reference](docs/cli-reference.md) | `forge-next` commands |
| [Security](docs/security.md) | Threat model, secret handling, audit log |
| [Operations](docs/operations.md) | Daemon ops, diagnostics, healing |
| [Cloud Deployment](docs/cloud-deployment.md) | Docker, Helm, K8s |
| [Agent Development](docs/agent-development.md) | Building agents on Forge |

---

## Deploy

### Docker
```bash
docker build -t forge-daemon .
docker run -d -p 8420:8420 -v forge-data:/data forge-daemon
```

### Docker Compose
```bash
cd deploy && docker-compose up -d
```

### Kubernetes (Helm)
```bash
helm install forge ./deploy/helm/
```

See [docs/cloud-deployment.md](docs/cloud-deployment.md) for production deployment including TLS, JWT/OIDC, RBAC, Prometheus/Grafana observability, and Litestream replication.

---

## Under the Hood

```
98 protocol endpoints · 8 background workers · 8 memory layers
1,245+ Rust tests · 0 warnings (clippy) · Apache-2.0 licensed
Enterprise: Docker · Helm · JWT/OIDC · RBAC · Audit · Prometheus · Multi-tenant
```

| Component | Tests | Framework |
|-----------|-------|-----------|
| forge-daemon (unit) | 990 | Rust |
| forge-daemon (integration) | 123 | Rust |
| forge-core | 56 | Rust |
| forge-cli | 76 | Rust |
| **Total (Rust)** | **1,245** | |

---

## The Architecture is Domain-Agnostic

The 8-layer memory, identity system, disposition engine, perception pipeline, and guardrails are not coding-specific. They are general-purpose cognitive primitives. Today, Forge makes coding agents powerful. The same architecture can make any agent powerful.

---

## Contributing

We welcome contributions. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

```bash
# Before submitting a PR
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

Join the discussion: [GitHub Discussions](https://github.com/chaosmaximus/forge/discussions)

---

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE) for details.

```
Copyright 2026 Forge Contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
```
