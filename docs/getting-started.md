# Getting Started with Forge

Forge is cognitive infrastructure for AI agents. It provides persistent, multi-layered memory that compounds across sessions, machines, and projects. This guide walks through installation and first use.

## Prerequisites

- **OS:** Linux (x86_64, aarch64) or macOS (Apple Silicon, Intel)
- **Rust toolchain:** Required to build from source. Install via [rustup](https://rustup.rs/).

## Install

### Option A: Cargo install from git (recommended)

```bash
cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli
```

This builds `forge-daemon` and `forge-next` and installs them to `~/.cargo/bin`.

### Option B: Build from source

```bash
git clone https://github.com/chaosmaximus/forge.git
cd forge
cargo build --release -p forge-daemon -p forge-cli
```

Then install manually:

```bash
cp target/release/forge-daemon ~/.local/bin/
cp target/release/forge-next ~/.local/bin/
```

### Option C: Docker

```bash
docker run -d \
  --name forge \
  -p 8420:8420 \
  -v forge-data:/var/lib/forge \
  ghcr.io/chaosmaximus/forge-daemon:latest
```

## Start the Daemon

```bash
forge-daemon &
```

The daemon listens on `localhost:8420` by default and stores state in `~/.forge/`.

## Verify Installation

Run the health check to confirm the daemon is running:

```bash
forge-next health
```

Expected output:

```
Forge daemon is healthy
  socket: /home/you/.forge/forge.sock
  uptime: 12s
  version: 0.4.0
```

Check the 8-layer memory system:

```bash
forge-next manas-health
```

Run the full diagnostic:

```bash
forge-next doctor
```

This checks all workers (extraction, embedding, consolidation, indexer), verifies the database, and reports any issues.

## Store Your First Memory

Forge organizes memories by type: `decision`, `lesson`, `pattern`, `fact`, `entity`, `skill`.

Store a decision:

```bash
forge-next remember \
  --type decision \
  --title "Use PostgreSQL for persistence" \
  --content "Chose PostgreSQL over SQLite for the web app due to concurrent write requirements."
```

Store a lesson learned:

```bash
forge-next remember \
  --type lesson \
  --title "Always pin dependency versions" \
  --content "Unpinned deps caused a production break when a transitive dependency released a breaking change."
```

## Recall Memories

Search across all memory layers:

```bash
forge-next recall "database choice"
```

Filter by project:

```bash
forge-next recall "deployment" --project my-web-app
```

Filter by type:

```bash
forge-next recall "testing" --type lesson --limit 5
```

Filter by memory layer:

```bash
forge-next recall "architecture" --layer domain_dna
```

The 8 memory layers are: `platform`, `tool`, `skill`, `domain_dna`, `experience`, `perception`, `declared`, `latent`.

## Compile Context

Generate a full context summary for an agent:

```bash
forge-next compile-context --agent claude-code
```

This assembles memories from all 8 layers, applies identity facets, and returns structured context sized for the agent's context window.

Scope to a specific project:

```bash
forge-next compile-context --agent claude-code --project my-web-app
```

## Agent Integration

Forge exposes all its capabilities via HTTP at `localhost:8420/api`. Any agent can integrate by making `POST` requests with JSON:

```bash
curl -s http://localhost:8420/api \
  -d '{"method":"recall","params":{"query":"database"}}'
```

See [api-reference.md](api-reference.md) for the full protocol, and [agent-development.md](agent-development.md) for how to build agents on top of Forge.

## Configuration

The config file lives at `~/.forge/config.toml`. Key settings:

```toml
# Auto-extraction backend: auto, ollama, claude, claude_api, openai, gemini
[extraction]
backend = "auto"

# Ollama for local extraction (free, private)
[extraction.ollama]
model = "qwen3:0.6b"
endpoint = "http://localhost:11434"

# Worker intervals
[workers]
extraction_debounce_secs = 15
consolidation_interval_secs = 1800
embedding_interval_secs = 60
indexer_interval_secs = 300

# Context budget for agents
[context]
budget_chars = 3000
decisions_limit = 10
lessons_limit = 5

# Inter-agent communication
[a2a]
enabled = true
trust = "open"
```

## Identity (Ahankara)

Forge supports per-agent identity facets that shape how context is compiled and presented.

List current identity facets:

```bash
forge-next identity list
```

Set a facet:

```bash
forge-next identity set \
  --facet coding_style \
  --description "Prefers functional programming patterns, avoids mutation, writes small composable functions."
```

Remove a facet:

```bash
forge-next identity remove <facet-id>
```

## Security Scanning

Scan a directory for exposed secrets:

```bash
forge scan .
```

Run continuous monitoring:

```bash
forge scan . --watch --interval 30
```

Forge never stores actual secret values -- only SHA256 fingerprints for tracking.

## Common Commands Reference

| Command | Description |
|---------|-------------|
| `forge-next health` | Check daemon health |
| `forge-next manas-health` | Check 8-layer memory system health |
| `forge-next doctor` | Full diagnostic (workers, DB, config) |
| `forge-next remember --type T --title "..." --content "..."` | Store a memory |
| `forge-next recall "query"` | Search memories |
| `forge-next compile-context --agent A` | Generate agent context |
| `forge-next sessions` | List active sessions |
| `forge-next identity list` | List identity facets |
| `forge-next export --format json` | Export all memories |
| `forge-next import --file F` | Import memories from file |
| `forge scan .` | Scan for exposed secrets |

## Multi-Tenant Setup

Forge supports multi-tenant isolation through organization_id scoping. All memory queries, recall results, and sync operations are filtered by organization.

Create an organization:

```bash
forge-next org-create --name "Acme Corp" --description "Main development organization"
```

Or initialize from a template:

```bash
forge-next org-from-template --template startup --name "MyStartup"
```

Initialize the workspace directory structure:

```bash
forge-next org-init --name "MyStartup"
```

View the workspace status:

```bash
forge-next workspace-status
```

Memory sync respects cross-tier sync policies: decisions and lessons propagate from local to team, but only decisions and protocols propagate from team to organization level. Preferences stay local.

## Next Steps

- [Cloud Deployment](cloud-deployment.md) -- deploy Forge to Kubernetes for team-wide shared memory
- [Agent Development](agent-development.md) -- build custom AI agents that connect to Forge

## Installing the Claude Code Plugin

Forge ships a Claude Code plugin (manifest in `.claude-plugin/`) that registers
hooks, skills, and subagents so Claude Code sessions automatically register
with the running daemon, stream memory writes, and surface matching skills in
context.

### Option A: Symlink-install from a local clone (fastest for development)

```bash
git clone https://github.com/chaosmaximus/forge.git
mkdir -p ~/.claude/plugins
ln -snf "$PWD/forge" ~/.claude/plugins/forge
```

### Option B: Marketplace install

From a Claude Code session, invoke the plugin marketplace and install the
`forge` plugin. (Full marketplace publication lands in 2P-1b — until then use
Option A.)

### Verify hooks fire

Start the daemon in one terminal:

```bash
forge-daemon
```

Open a new Claude Code session. You should see the daemon log a
`register_session` entry within a few seconds — this confirms
`scripts/hooks/session-start.sh` executed. Ask Claude any question, then:

```bash
forge-next recall "<any keyword from your prompt>"
```

You should see at least one memory whose `session_id` matches the session you
just opened. If not, run `forge-next doctor` and check the "Hook" health row.
