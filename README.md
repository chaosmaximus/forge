<p align="center">
  <img src="docs/images/creation-of-adam.jpg" alt="The Creation of Adam — Michelangelo, Sistine Chapel, c. 1512" width="720" />
</p>

<h1 align="center">Forge</h1>

<p align="center">
  <strong>The operating system for AI agents.</strong>
</p>

<p align="center">
  Multi-layered memory. Proactive perception. Identity. Guardrails.<br />
  One daemon — any agent, any domain. Coding is the first vertical.
</p>

<p align="center">
  <a href="https://forge.bhairavi.tech">Website</a> ·
  <a href="https://github.com/chaosmaximus/forge/discussions">Discussions</a> ·
  <a href="https://github.com/sponsors/chaosmaximus">Sponsor</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/tests-378%20passing-brightgreen" alt="Tests" />
  <img src="https://img.shields.io/badge/rust-1.88-orange" alt="Rust" />
  <a href="https://github.com/sponsors/chaosmaximus"><img src="https://img.shields.io/badge/sponsor-GitHub%20Sponsors-ea4aaa" alt="Sponsor" /></a>
  <a href="https://github.com/chaosmaximus/forge/discussions"><img src="https://img.shields.io/badge/community-Discussions-blue" alt="Discussions" /></a>
</p>

<!-- TODO: Replace with actual demo recording
<p align="center">
  <img src="docs/images/demo.gif" alt="Forge demo — forge recall with guardrail warning" width="720" />
</p>
-->

---

## Why Forge?

AI agents are bare metal. Powerful reasoning engines with no nervous system. Every capability must be manually wired. The agent spends half its cognitive budget figuring out *how* to do things instead of *doing* things.

**Forge changes this. The agent thinks. Forge does everything else.**

```
Session 1: Agent learns "Use JWT for auth, RS256 signing, rotating keys"
            → Forge extracts and stores as a Decision in the knowledge graph

Session 2: Agent runs `forge recall "auth"` → gets the decision back in <10ms
            → Guardrail warning: "auth/middleware.rs has 3 linked decisions"
```

No manual tagging. No MEMORY.md flat files. A real knowledge graph with 8 layers.

---

## Install

```bash
curl -fsSL https://forge.bhairavi.tech/install | sh
```

Or via Claude Code plugin:
```bash
claude plugin marketplace add chaosmaximus/forge
```

The daemon auto-starts on first command. No manual setup needed.

### Prerequisites (optional)

- [Ollama](https://ollama.ai) — for local LLM extraction (recommended)
- The daemon works without Ollama but auto-extraction requires a local model

---

## Two Layers, One App

### Layer 1: The Terminal

A Tauri v2 desktop terminal (SolidJS + Rust) built to replace iTerm2/Ghostty:

| Feature | Description |
|---------|-------------|
| **Terminal** | xterm.js with WebGL rendering, GPU-accelerated |
| **Multi-tab** | Shell, tmux, Claude Code, Codex, Gemini, SSH — each with its own PTY |
| **Cmd+K Search** | Spotlight-style memory search across all 8 layers |
| **Brain Map** | Canvas 2D visualization with breathing animation — see what Forge knows |
| **Guardrails** | Inline warnings when agents edit files with linked architectural decisions |
| **SSH** | Built-in SSH with `~/.ssh/config` parsing and key management |
| **Agent Status** | Real-time working/waiting/idle detection per tab |

### Layer 2: The Daemon

An always-on Rust daemon with the Manas 8-layer memory architecture:

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

Plus: Identity system (agent persona), Disposition engine (behavioral tendencies), Guardrails engine (blast radius + decision tracking), and Proactive context compiler.

**7 background workers** running continuously. Auto-extraction. Zero config.

---

## CLI

```bash
# Memory
forge-next recall "auth" --project forge --limit 10
forge-next remember --type decision --title "Use JWT" --content "..."
forge-next forget <id>

# 8-layer health
forge-next health                    # experience layer counts
forge-next manas-health              # all 8 layers
forge-next platform                  # platform layer entries

# Identity
forge-next identity set --facet role --description "Senior Rust dev"

# Guardrails
forge-next check --file src/main.rs
forge-next blast-radius --file src/main.rs

# Memory sync
forge-next sync-pull <host> --project myproject
forge-next sync-push <host> --project myproject

# System
forge-next doctor                    # full diagnostics
forge-next compile-context           # proactive context assembly
```

---

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

**Works with any agent:** Claude Code, Codex, Gemini, Cursor, Cline — thin adapters teach each agent to recall. The daemon extracts. The graph grows.

---

## Security

- **Socket validation** — lstat + is_socket + UID ownership check
- **Shell whitelist** — only `/bin/`, `/usr/bin/`, `/opt/homebrew/bin/` paths
- **Session name sanitization** — prevents shell/flag injection
- **Secret scanning** — SHA256 fingerprints, never stores actual values
- **Parameterized queries** — no SQL injection
- **CSP enabled** — default-src 'self', restricted connect-src
- **IPC permissions** — minimal Tauri capabilities
- **5 adversarial reviews** completed (Codex gpt-5.4)

---

## Tests

| Component | Tests | Framework |
|-----------|-------|-----------|
| forge-core | 35 | Rust |
| forge-daemon | 261 | Rust |
| forge app (Rust) | 47 | Rust |
| forge app (frontend) | 17 | Vitest |
| **Total** | **378** | |

```bash
cargo test --workspace           # full suite
cargo clippy --workspace -- -W clippy::all  # zero warnings
```

---

## Pricing

| | Free | Pro $12/mo | Team $19/seat | Enterprise |
|--|------|----------|--------------|-----------|
| Terminal | Full | Full | Full | Full |
| Memory | 200 | Unlimited | Unlimited | Unlimited |
| Brain map | Preview | Interactive | Interactive | Interactive |
| Agents | 1 | Unlimited | Unlimited | Unlimited |
| SSH | Unlimited | Unlimited | Unlimited | Unlimited |
| Sync | — | 3 machines | Unlimited | Unlimited |

**Zero marginal cost per user** — everything runs locally.

---

## Competitive Position

Nobody else combines: premium terminal + 8-layer knowledge graph + brain map + guardrails + $12/mo + local-first.

| Product | Terminal | Memory Graph | Brain Map | Guardrails | Price |
|---------|----------|-------------|-----------|-----------|-------|
| **Forge** | **Yes** | **8-layer** | **Yes** | **Yes** | **$12/mo** |
| Warp | Yes | No | No | No | $18/mo |
| Mem0 | No | Yes (cloud) | No | No | $249/mo |
| Claude Code | CLI | MEMORY.md | No | No | $20/mo |

---

## The Vision

The same architecture that makes a coding agent powerful can make *any* agent powerful. The daemon is domain-agnostic. The 8-layer memory, identity system, disposition engine, perception pipeline, and guardrails work for any agent in any domain.

| Phase | Vertical | What Ships |
|-------|----------|-----------|
| **Now** | Coding | Terminal + daemon for coding agents |
| **Next** | DevOps | Infrastructure agent adapters |
| **Then** | Research | Knowledge management UI |
| **Later** | Any domain | Marketplace for domain modules |

See [`product/`](product/) for full product documents:
[Vision](product/vision.md) · [Positioning](product/positioning.md) · [Pricing](product/pricing.md) · [Competitive landscape](product/competitive-landscape-2026-04.md) · [User stories](product/user-stories.md)

---

## Acknowledgments

Built on: [Tauri](https://tauri.app/) · [SolidJS](https://solidjs.com/) · [xterm.js](https://xtermjs.org/) · [SQLite](https://sqlite.org/) · [portable-pty](https://github.com/wez/wezterm/tree/main/pty) · [Superpowers](https://github.com/obra/superpowers)

---

<p align="center">
  <sub>Built by <a href="https://bhairavi.tech">Bhairavi Tech</a> · <a href="https://forge.bhairavi.tech">forge.bhairavi.tech</a></sub>
</p>
