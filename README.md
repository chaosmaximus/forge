# Forge

<p align="center">
  <img src="docs/images/creation-of-adam.jpg" alt="The Creation of Adam — Michelangelo, Sistine Chapel, c. 1512" width="720" />
</p>

**The operating system for AI agents.**

Multi-layered memory. Proactive perception. Universal tool abstraction. One daemon — any agent, any domain.

Coding is the first vertical. The architecture is domain-agnostic. The same 8-layer memory system that makes a coding agent remember your architecture can make a medical agent remember patient history, a research agent track hypotheses, or a driving agent learn road patterns. We validate on coding because we eat our own dogfood — then we expand to everything.

[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-378%20passing-brightgreen)]()
[![Rust](https://img.shields.io/badge/rust-1.88-orange)]()

---

## Why Forge?

AI agents are bare metal. They're powerful reasoning engines with no nervous system. Every capability must be manually wired — MCP servers for tools, skills for workflows, hooks for automation, plugins for memory. The agent spends half its cognitive budget figuring out *how* to do things instead of *doing* things.

**Forge changes this. The agent thinks. Forge does everything else.**

- **Auto-extraction** — the daemon silently learns from every session
- **Proactive context** — the right information surfaces *before* the agent asks
- **Ambient intelligence** — memory, indexing, security scanning, all happening continuously
- **Universal abstraction** — one daemon, any agent, any domain

```
Session 1: Agent learns "Use JWT for auth, RS256 signing, rotating keys"
            → Forge extracts and stores as a Decision in the knowledge graph

Session 2: Agent runs `forge recall "auth"` → gets the decision back in <10ms
            → Agent also gets a guardrail warning: "auth/middleware.rs has 3 linked decisions"
```

No manual tagging. No copying. No MEMORY.md flat files. A real knowledge graph with 8 layers.

## The App

A Tauri v2 desktop terminal (SolidJS + Rust) that replaces iTerm2/Ghostty:

| Feature | Description |
|---------|-------------|
| **Terminal** | xterm.js with WebGL rendering, Geist Mono font, GPU-accelerated |
| **Multi-tab** | Shell, tmux, Claude Code agent, and SSH tabs — each with its own PTY |
| **Cmd+K Search** | Spotlight-style memory search across all 8 layers with project/layer filtering |
| **Sidebar** | Agent status (working/waiting/idle), 8-layer memory stats, brain map preview |
| **Brain Map** | Canvas 2D visualization with breathing animation — see what Forge knows |
| **Guardrails** | Inline warnings when agents edit files with linked architectural decisions |
| **Notifications** | Native alerts on task completion and memory extraction |
| **SSH** | Built-in SSH with ~/.ssh/config parsing and key management |
| **Shortcuts** | Cmd+K (search), Cmd+T (new tab), Cmd+W (close), Cmd+1-9 (switch) |

## The Daemon

A Rust daemon with the Manas 8-layer memory architecture:

| Layer | What It Stores | How It Grows |
|-------|---------------|-------------|
| 1. Platform | OS, CPU, shell, hostname | Auto-detected at startup |
| 2. Tool | Available tools, APIs | User/app registered |
| 3. Skill | Learned workflows | Extracted from sessions |
| 4. Domain DNA | Project conventions | Detected from codebase |
| 5. Experience | Decisions, lessons, patterns | LLM extraction (core) |
| 6. Perception | Git state, file changes | Perception worker (30s) |
| 7. Declared | CLAUDE.md, README, docs | Ingested from files |
| 8. Latent | Embedding vectors | Embedder worker (60s) |

Plus: Identity system (agent persona), Disposition engine (behavioral tendencies), Guardrails engine (blast radius, decision tracking), and Proactive context compiler.

**7 background workers** running continuously: extraction, embedding, consolidation, perception, disposition, indexing, watching.

## Quick Start

```bash
# Clone
git clone https://github.com/chaosmaximus/forge.git
cd forge

# Build the daemon
cargo build --release -p forge-daemon -p forge-cli
./target/release/forge-daemon &

# Build and run the app
cd app/forge
npm install
npm run tauri dev
```

### Prerequisites

- Rust 1.88+
- Node.js 18+
- tmux (`brew install tmux`)
- Ollama (for local LLM extraction — optional)

## CLI Reference

```bash
# Memory
forge-next recall "auth" --project forge --limit 10
forge-next recall "database" --layer experience
forge-next remember --type decision --title "Use JWT" --content "..."
forge-next forget <id>

# Manas layers
forge-next health                    # Experience layer counts
forge-next manas-health              # All 8 layer counts
forge-next health-by-project         # Per-project breakdown
forge-next platform                  # Platform layer entries

# Identity
forge-next identity set --facet role --description "Senior Rust dev"
forge-next identity list

# Guardrails
forge-next check --file src/main.rs
forge-next blast-radius --file src/main.rs

# System
forge-next doctor                    # Full diagnostics
forge-next sessions                  # Active agent sessions
forge-next compile-context           # Proactive context assembly
```

## Architecture

```
forge-core (shared types + protocol)
    ↑
forge-daemon (8-layer Manas, 7 workers, socket API)
    ↑
forge-cli (thin socket client)

forge app (Tauri v2 — SolidJS + Rust IPC → daemon socket)
    ├── xterm.js terminal (WebGL)
    ├── Sidebar (agents + memory + brain map)
    ├── Search overlay (Cmd+K)
    └── Guardrail warnings
```

**Communication:** Unix domain socket, NDJSON protocol.

**Shared types:** App and daemon share types via the `forge-core` crate. No duplicate definitions.

## Tests

| Component | Tests | Framework |
|-----------|-------|-----------|
| forge-core | 35 | Rust |
| forge-daemon | 261 | Rust |
| forge app (Rust) | 47 | Rust |
| forge app (frontend) | 17 | Vitest |
| **Total** | **378** | |

All tests pass. 5 adversarial Codex (gpt-5.4) reviews completed.

## Security

- **CSP enabled** — default-src 'self', restricted connect-src
- **Socket validation** — lstat + is_socket + UID ownership check before connect
- **Shell whitelist** — only /bin/, /usr/bin/, /opt/homebrew/bin/ paths allowed
- **Session name sanitization** — prevents shell/flag injection
- **PTY cleanup** — kill → join → wait, no zombie processes
- **Secret scanning** — SHA256 fingerprints, never stores actual values
- **Parameterized queries** — no SQL injection in daemon
- **IPC permissions** — minimal Tauri capabilities

## Pricing

| | Free | Pro $9/mo | Team $19/seat | Enterprise |
|--|------|----------|--------------|-----------|
| Terminal | Full | Full | Full | Full |
| Memory | 200 | Unlimited | Unlimited | Unlimited |
| Brain map | Preview | Interactive | Interactive | Interactive |
| Agents | 1 | Unlimited | Unlimited | Unlimited |
| SSH | Unlimited | Unlimited | Unlimited | Unlimited |
| Sync | — | 3 machines | Unlimited | Unlimited |

**Zero marginal cost per user** — everything runs locally.

## Competitive Position

Nobody else combines: premium terminal + 8-layer knowledge graph + brain map + guardrails + $9/mo + local-first.

| Product | Terminal | Memory Graph | Brain Map | Guardrails | Price |
|---------|----------|-------------|-----------|-----------|-------|
| **Forge** | **Yes** | **8-layer** | **Yes** | **Yes** | **$9/mo** |
| Warp | Yes | No | No | No | $18/mo |
| Mem0 | No | Yes (cloud) | No | No | $249/mo |
| OpenClaw | No (chat) | Plugins | No | No | Free |
| Claude Code | CLI | MEMORY.md | No | No | $20/mo |

## Product Documents

See [`product/`](product/) for:
- [Vision](product/vision.md) — "The terminal that remembers"
- [Positioning](product/positioning.md) — Category creation, sales ammunition
- [Pricing](product/pricing.md) — $9/mo disruption strategy
- [Competitive landscape](product/competitive-landscape-2026-04.md) — Full market analysis
- [User stories](product/user-stories.md) — US-1 through US-13
- [Designs](product/designs/) — Pencil mockups (v3 final)

## Contributing

See [CONTRIBUTING.md](docs/archive/v030/CONTRIBUTING.md) for guidelines.

## Acknowledgments

Built on: [Tauri](https://tauri.app/) · [SolidJS](https://solidjs.com/) · [xterm.js](https://xtermjs.org/) · [Claude Code](https://docs.anthropic.com/en/docs/claude-code) · [Codex](https://github.com/openai/codex) · [portable-pty](https://github.com/wez/wezterm/tree/main/pty) · [SQLite](https://sqlite.org/) · [Superpowers](https://github.com/obra/superpowers)

## License

[MIT](LICENSE)
