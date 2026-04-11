# CLI Reference

`forge-next` is the Rust CLI client for the forge-daemon. It communicates over a Unix domain socket (local) or HTTP/gRPC (remote). The daemon auto-starts on first invocation if not already running.

## Global Flags

These flags apply to all commands.

| Flag | Env Override | Description |
|------|-------------|-------------|
| `--endpoint <URL>` | `FORGE_ENDPOINT` | Connect to a remote daemon (e.g., `https://forge.company.com`). Omit for local Unix socket. |
| `--token <JWT>` | `FORGE_TOKEN` | JWT auth token for remote connections. Required when `auth.enabled=true` on the remote daemon. |

```bash
# Local (default) -- uses Unix socket at ~/.forge/forge.sock
forge-next recall "authentication"

# Remote
forge-next --endpoint https://forge.company.com --token $JWT recall "authentication"
```

---

## Memory Operations

### recall

Search memories using hybrid BM25 + vector + knowledge-graph ranking.

```
forge-next recall <QUERY> [--type TYPE] [--project PROJECT] [--limit N] [--layer LAYER]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<QUERY>` | *(required)* | Free-text search query |
| `--type TYPE` | *(all)* | Filter by memory type: `decision`, `lesson`, `pattern`, `preference` |
| `--project PROJECT` | *(all + global)* | Filter by project name. Global memories are always included. |
| `--limit N` | `10` | Maximum number of results to return |
| `--layer LAYER` | *(all)* | Filter by Manas layer: `experience`, `declared`, `domain_dna`, `skill`, `perception`, `identity` |

```bash
# Basic search
forge-next recall "authentication flow"

# Filter by type and project
forge-next recall "database schema" --type decision --project my-app --limit 5

# Search a specific Manas layer
forge-next recall "coding style" --layer domain_dna
```

### remember

Store a new memory in the daemon.

```
forge-next remember --type TYPE --title TITLE --content CONTENT [--confidence SCORE] [--tags T1,T2] [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--type TYPE` | *(required)* | Memory type: `decision`, `lesson`, `pattern`, `preference` |
| `--title TITLE` | *(required)* | Short title for the memory |
| `--content CONTENT` | *(required)* | Full content text |
| `--confidence SCORE` | *(none)* | Confidence score, 0.0 to 1.0 |
| `--tags T1,T2` | *(none)* | Comma-separated tags |
| `--project PROJECT` | *(global)* | Associate with a project |

```bash
# Store a decision
forge-next remember --type decision \
  --title "Use SQLite for persistence" \
  --content "SQLite with WAL mode provides sufficient throughput for single-node deployments" \
  --tags "architecture,database" \
  --project forge

# Store a lesson with confidence
forge-next remember --type lesson \
  --title "Always check WAL size" \
  --content "WAL files can grow unbounded under write-heavy workloads. Monitor and checkpoint." \
  --confidence 0.9
```

### forget

Soft-delete a memory by ID. The memory is marked as deleted but retained for sync conflict resolution.

```
forge-next forget <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `<ID>` | *(required)* | Memory ID (ULID) to soft-delete |

```bash
forge-next forget 01JQXYZ1234ABCD5678EFGH
```

### supersede

Mark an old memory as superseded by a newer one. The old memory is kept in history but stops surfacing in recall results.

```
forge-next supersede --old-id <OLD_ID> --new-id <NEW_ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--old-id OLD_ID` | *(required)* | ID of the old memory to supersede |
| `--new-id NEW_ID` | *(required)* | ID of the new memory that replaces it |

```bash
forge-next supersede --old-id 01JQXYZ1234ABCD5678EFGH --new-id 01JQXYZ9999ABCD0000EFGH
```

---

## Session Management

### register-session

Register an active agent session with the daemon.

```
forge-next register-session --id ID --agent AGENT [--project PROJECT] [--cwd DIR]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Session ID (typically a UUID or ULID) |
| `--agent AGENT` | *(required)* | Agent name: `claude-code`, `cline`, `codex`, etc. |
| `--project PROJECT` | *(none)* | Project scope for this session |
| `--cwd DIR` | *(none)* | Working directory for this session |

```bash
forge-next register-session --id sess-abc123 --agent claude-code --project forge --cwd /home/user/forge
```

### end-session

End an active agent session. Returns per-session KPIs for observability.

```
forge-next end-session --id ID
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Session ID to end |

```bash
forge-next end-session --id sess-abc123
```

The response includes `session_kpis` with the following metrics:
- `session_duration_secs` -- total session duration
- `context_injections` -- number of context injections during the session
- `context_chars_injected` -- total characters of context injected
- `a2a_messages_sent` -- A2A messages sent from this session
- `a2a_messages_received` -- A2A messages received by this session

### sessions

List active agent sessions.

```
forge-next sessions [--all]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--all` | `false` | Show all sessions including ended ones |

```bash
# Active sessions only
forge-next sessions

# Include ended sessions
forge-next sessions --all
```

### cleanup-sessions

End all active sessions, optionally filtered by ID prefix.

```
forge-next cleanup-sessions [--prefix PREFIX]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--prefix PREFIX` | *(all)* | Only end sessions whose ID starts with this prefix |

```bash
# End all sessions
forge-next cleanup-sessions

# End only test sessions
forge-next cleanup-sessions --prefix hook-test
```

### set-task

Set the current task on a session. Populates the session card for observability and context injection.

```
forge-next set-task --session <SESSION> --task <TASK>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session SESSION` | *(required)* | Session ID |
| `--task TASK` | *(required)* | Task description |

```bash
forge-next set-task --session sess-abc123 --task "Implementing authentication module"
```

### session-heartbeat

Send a heartbeat to keep a session alive. Prevents the session from being reaped by the daemon's session cleanup logic.

```
forge-next session-heartbeat --session <SESSION>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session SESSION` | *(required)* | Session ID to heartbeat |

```bash
forge-next session-heartbeat --session sess-abc123
```

### subscribe

Subscribe to real-time daemon events. Streams NDJSON event objects to stdout until interrupted.

```
forge-next subscribe [--events EVENTS] [--session SESSION] [--team TEAM]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--events EVENTS` | *(all)* | Comma-separated event type filter |
| `--session SESSION` | *(none)* | Filter events by session ID |
| `--team TEAM` | *(none)* | Filter events by team ID |

```bash
# Subscribe to all events
forge-next subscribe

# Subscribe to specific event types for a session
forge-next subscribe --events "memory_created,session_ended" --session sess-abc123

# Subscribe to team events
forge-next subscribe --team backend
```

---

## A2A Messaging

Inter-session messaging using the FISP (Forge Inter-Session Protocol).

### send

Send a message to another session or broadcast to all sessions.

```
forge-next send --to TARGET --kind KIND --topic TOPIC --text TEXT [--project PROJECT] [--timeout SECS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--to TARGET` | *(required)* | Target session ID, or `"*"` for broadcast |
| `--kind KIND` | *(required)* | Message kind: `notification` or `request` |
| `--topic TOPIC` | *(required)* | Topic string (e.g., `file_changed`, `review_code`, `schema_changed`) |
| `--text TEXT` | *(required)* | Message body |
| `--project PROJECT` | *(none)* | Project scope (required for broadcasts) |
| `--timeout SECS` | *(none)* | Timeout in seconds (for `request` kind) |

```bash
# Notify a specific session
forge-next send --to sess-abc123 --kind notification --topic file_changed --text "src/main.rs updated"

# Broadcast to all sessions in a project
forge-next send --to "*" --kind notification --topic schema_changed --text "migration 003 applied" --project my-app

# Send a request with timeout
forge-next send --to sess-abc123 --kind request --topic review_code --text "Please review PR #42" --timeout 300
```

**Protocol note:** The underlying NDJSON protocol also supports a `from_session` field for sender identification. When using the CLI, the sender is recorded as `"api"` by default. For programmatic use via the Unix socket or HTTP API, include `"from_session": "<session-id>"` in the request payload.

### messages

Retrieve pending messages for a session.

```
forge-next messages --session ID [--status STATUS] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session ID` | *(required)* | Session ID to check inbox for |
| `--status STATUS` | *(all)* | Filter by status: `pending`, `read`, `completed` |
| `--limit N` | *(all)* | Maximum messages to return |

```bash
forge-next messages --session sess-abc123 --status pending --limit 10
```

### ack

Acknowledge (mark as read) one or more messages by ID.

```
forge-next ack <ID1> [ID2 ...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<IDs>` | *(required)* | One or more message IDs to acknowledge |

```bash
forge-next ack msg-001 msg-002 msg-003
```

### A2A Permissions

Control which agents can message each other when `a2a.trust` is set to `controlled`.

#### grant-permission

Grant an A2A messaging permission from one agent to another.

```
forge-next grant-permission --from <FROM> --to <TO> [--from-project PROJECT] [--to-project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--from FROM` | *(required)* | Source agent name |
| `--to TO` | *(required)* | Target agent name |
| `--from-project PROJECT` | *(none)* | Restrict source to a specific project |
| `--to-project PROJECT` | *(none)* | Restrict target to a specific project |

```bash
forge-next grant-permission --from claude-code --to cline
forge-next grant-permission --from claude-code --to cline --from-project forge --to-project forge
```

#### revoke-permission

Revoke an A2A permission by its ID.

```
forge-next revoke-permission --id <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Permission ID to revoke |

```bash
forge-next revoke-permission --id perm-abc123
```

#### list-permissions

List all A2A permissions.

```
forge-next list-permissions
```

### message-read

Read a single FISP message by its ID.

```
forge-next message-read --id <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Message ID to read |

```bash
forge-next message-read --id msg-abc123
```

---

## Identity (Ahankara)

Manage per-agent identity facets that shape behavior and context injection.

### identity list

List identity facets for an agent.

```
forge-next identity list [--agent AGENT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent AGENT` | `claude-code` | Agent name to list facets for |

```bash
forge-next identity list
forge-next identity list --agent cline
```

### identity set

Create or update an identity facet.

```
forge-next identity set --facet FACET --description DESC [--agent AGENT] [--strength SCORE]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--facet FACET` | *(required)* | Facet type: `role`, `expertise`, `values`, `goals`, `constraints` |
| `--description DESC` | *(required)* | Description of this facet |
| `--agent AGENT` | `claude-code` | Agent name |
| `--strength SCORE` | `0.5` | Strength weight from 0.0 to 1.0 |

```bash
forge-next identity set --facet role --description "Senior backend engineer with Rust expertise" --strength 0.8
forge-next identity set --facet constraints --description "Never use unsafe Rust" --agent cline
```

### identity remove

Deactivate an identity facet by ID.

```
forge-next identity remove <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `<ID>` | *(required)* | Facet ID to deactivate |

```bash
forge-next identity remove facet-abc123
```

---

## Configuration

### config show

Display the current daemon configuration.

```
forge-next config show
```

### config set

Update a config value using dotted-key notation. Persists to `~/.forge/config.toml`.

```
forge-next config set <KEY> <VALUE>
```

Supported keys include:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `extraction.backend` | string | `auto` | Extraction backend: `auto`, `ollama`, `claude`, `claude_api`, `openai`, `gemini` |
| `extraction.ollama.model` | string | `gemma3:1b` | Ollama model name |
| `extraction.ollama.endpoint` | string | `http://localhost:11434` | Ollama API endpoint |
| `extraction.claude.model` | string | `haiku` | Claude CLI model name |
| `extraction.claude_api.model` | string | `claude-haiku-4-5-20251001` | Claude API model ID |
| `extraction.openai.model` | string | `gpt-4o-mini` | OpenAI model name |
| `extraction.gemini.model` | string | `gemini-2.0-flash` | Gemini model name |
| `embedding.model` | string | `nomic-embed-text` | Embedding model name |
| `embedding.dimensions` | integer | `768` | Embedding vector dimensions |
| `a2a.enabled` | bool | `true` | Enable A2A inter-session messaging |
| `a2a.trust` | string | `open` | Trust mode: `open` or `controlled` |
| `workers.extraction_debounce_secs` | integer | `15` | Extraction debounce interval |
| `workers.consolidation_interval_secs` | integer | `1800` | Consolidation cycle interval |
| `workers.embedding_interval_secs` | integer | `60` | Embedding batch interval |
| `workers.perception_interval_secs` | integer | `30` | Perception worker interval |
| `workers.disposition_interval_secs` | integer | `900` | Disposition engine interval |
| `workers.indexer_interval_secs` | integer | `300` | Code indexer interval |
| `workers.diagnostics_debounce_secs` | integer | `3` | Diagnostics debounce |
| `context.budget_chars` | integer | `3000` | Context compilation character budget |
| `context.decisions_limit` | integer | `10` | Max decisions in compiled context |
| `context.lessons_limit` | integer | `5` | Max lessons in compiled context |
| `context.entities_limit` | integer | `5` | Max entities in compiled context |
| `context.entities_min_mentions` | integer | `3` | Min mentions for entity inclusion |
| `consolidation.batch_limit` | integer | `200` | Consolidation batch size |
| `consolidation.reweave_limit` | integer | `50` | Reweave batch size |
| `recall.recency_24h_boost` | float | `1.5` | Boost factor for memories < 24h old |
| `recall.recency_7d_boost` | float | `1.2` | Boost factor for memories < 7d old |
| `recall.domain_dna_boost` | float | `1.3` | Boost factor for domain DNA matches |
| `reality.auto_detect` | bool | `true` | Auto-detect project type |
| `reality.code_embeddings` | bool | `false` | Enable code symbol embeddings |
| `reality.max_index_files` | integer | `5000` | Max files to index per project |

```bash
forge-next config set extraction.backend ollama
forge-next config set workers.consolidation_interval_secs 3600
forge-next config set context.budget_chars 5000
```

### config set-scoped

Set a config value at a specific scope level (organization, team, user, reality, agent, session). Scoped config cascades: session > agent > reality > user > team > organization > default.

```
forge-next config set-scoped --scope SCOPE --scope-id ID --key KEY --value VALUE [--locked] [--ceiling N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--scope SCOPE` | *(required)* | Scope type: `organization`, `team`, `user`, `reality`, `agent`, `session` |
| `--scope-id ID` | *(required)* | Entity ID for the scope |
| `--key KEY` | *(required)* | Config key (e.g., `context.budget_chars`) |
| `--value VALUE` | *(required)* | Config value |
| `--locked` | `false` | Lock this value, preventing lower scopes from overriding |
| `--ceiling N` | *(none)* | Set a ceiling for numeric values |

```bash
# Set a team-level budget
forge-next config set-scoped --scope team --scope-id backend --key context.budget_chars --value 8000

# Lock an organization-wide setting
forge-next config set-scoped --scope organization --scope-id acme --key extraction.backend --value claude_api --locked
```

### config get-effective

Resolve the effective config for a session context by cascading through all scope levels.

```
forge-next config get-effective [--session S] [--agent A] [--reality R] [--user U] [--team T] [--organization O]
```

### config list-scoped

List all config entries for a specific scope.

```
forge-next config list-scoped --scope SCOPE --scope-id ID
```

### config delete-scoped

Delete a scoped config entry.

```
forge-next config delete-scoped --scope SCOPE --scope-id ID --key KEY
```

---

## Sync

Peer-to-peer memory synchronization using Hybrid Logical Clocks (HLC) for conflict-free replication.

### sync-export

Export memories as NDJSON with HLC metadata.

```
forge-next sync-export [--project PROJECT] [--since TIMESTAMP]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--project PROJECT` | *(all)* | Filter export by project |
| `--since TIMESTAMP` | *(all)* | Only export memories with HLC timestamp after this value |

```bash
# Export all memories
forge-next sync-export > backup.ndjson

# Export project-specific memories since a timestamp
forge-next sync-export --project forge --since "2026-04-01T00:00:00Z"
```

### sync-import

Import NDJSON memory lines from stdin.

```
forge-next sync-import < backup.ndjson
```

### sync-pull

Pull memories from a remote host via SSH.

```
forge-next sync-pull <HOST> [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<HOST>` | *(required)* | SSH destination (e.g., `user@host`) |
| `--project PROJECT` | *(all)* | Filter by project |

```bash
forge-next sync-pull user@production-server --project my-app
```

### sync-push

Push local memories to a remote host via SSH.

```
forge-next sync-push <HOST> [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<HOST>` | *(required)* | SSH destination (e.g., `user@host`) |
| `--project PROJECT` | *(all)* | Filter by project |

```bash
forge-next sync-push user@production-server --project my-app
```

### sync-conflicts

List unresolved sync conflicts.

```
forge-next sync-conflicts
```

### sync-resolve

Resolve a sync conflict by keeping the specified memory.

```
forge-next sync-resolve <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `<ID>` | *(required)* | Memory ID to keep |

```bash
forge-next sync-resolve 01JQXYZ1234ABCD5678EFGH
```

---

## Diagnostics

### health

Show system health: daemon uptime, memory count, database status.

```
forge-next health
```

### health-by-project

Show memory counts grouped by project.

```
forge-next health-by-project
```

### doctor

Run comprehensive daemon health diagnostics: database integrity, worker status, configuration validation.

```
forge-next doctor
```

### manas-health

Show the 8-layer Manas memory system health: counts per layer, completeness, and recommendations.

```
forge-next manas-health
```

### platform

Show platform information (Manas Layer 1): OS, architecture, available tools, runtime environment.

```
forge-next platform
```

### tools

List discovered tools (Manas Layer 2): available CLIs, language servers, package managers.

```
forge-next tools
```

### perceptions

List unconsumed perceptions (Manas Layer 6): observations the daemon has made but not yet acted on.

```
forge-next perceptions [--project PROJECT] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--project PROJECT` | *(all)* | Filter by project |
| `--limit N` | `20` | Maximum results |

```bash
forge-next perceptions --project forge --limit 10
```

### compile-context

Compile optimized context from all Manas layers for session injection.

```
forge-next compile-context --agent AGENT [--project PROJECT] [--static-only]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent AGENT` | `claude-code` | Agent name |
| `--project PROJECT` | *(none)* | Project scope |
| `--static-only` | `false` | Only return the static prefix (platform, identity, disposition, tools) for KV-cache optimization |

```bash
# Full context
forge-next compile-context --agent claude-code --project forge

# Static prefix only (for KV-cache optimization)
forge-next compile-context --agent claude-code --project forge --static-only
```

### context-trace

Show the context compilation trace: which memories were included, excluded, and why.

```
forge-next context-trace [--agent AGENT] [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent AGENT` | `claude-code` | Agent name |
| `--project PROJECT` | *(none)* | Project scope |

```bash
forge-next context-trace --agent claude-code --project forge
```

### lsp-status

Show available language servers for the current project.

```
forge-next lsp-status
```

### entities

List detected entities (recurring concepts extracted from project memories).

```
forge-next entities [--project PROJECT] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--project PROJECT` | *(all)* | Filter by project |
| `--limit N` | `20` | Maximum results |

```bash
forge-next entities --project forge --limit 50
```

### check

Pre-execution guardrail check. Evaluates whether an action on a file is safe based on knowledge graph data.

```
forge-next check --file PATH [--action ACTION]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--file PATH` | *(required)* | File path to check |
| `--action ACTION` | `edit` | Action type: `edit`, `delete`, `rename` |

```bash
forge-next check --file src/main.rs --action edit
forge-next check --file migrations/001.sql --action delete
```

### post-edit-check

Surface callers, lessons, and warnings after editing a file.

```
forge-next post-edit-check --file PATH
```

### pre-bash-check

Warn about destructive commands and surface relevant skills or lessons before execution.

```
forge-next pre-bash-check --command "rm -rf /tmp/build"
```

### post-bash-check

Surface lessons and skills after a command failure.

```
forge-next post-bash-check --command "cargo test" --exit-code 1
```

| Flag | Default | Description |
|------|---------|-------------|
| `--command CMD` | *(required)* | The command that was run |
| `--exit-code N` | `1` | Exit code of the command |

### blast-radius

Analyze the blast radius of changing a file: dependents, callers, tests affected.

```
forge-next blast-radius --file PATH
```

| Flag | Default | Description |
|------|---------|-------------|
| `--file PATH` | *(required)* | File path to analyze |

```bash
forge-next blast-radius --file crates/daemon/src/recall.rs
```

### verify

Run proactive checks on a file or show all active diagnostics.

```
forge-next verify [--file PATH]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--file PATH` | *(none)* | File to check. Omit to show all active diagnostics. |

### diagnostics

Show cached diagnostics for a file.

```
forge-next diagnostics --file PATH
```

### stats

Show extraction metrics, token usage, and cost tracking.

```
forge-next stats [--hours N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--hours N` | `24` | Time period in hours |

```bash
forge-next stats --hours 48
```

---

## Import / Export

### export

Export all data as JSON for visualization, backup, or migration.

```
forge-next export [--format FORMAT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format FORMAT` | `json` | Output format: `json` or `ndjson` |

```bash
forge-next export > forge-backup.json
forge-next export --format ndjson > forge-backup.ndjson
```

### import

Import data from JSON file or stdin.

```
forge-next import [--file PATH]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--file PATH` | *(stdin)* | File to import. Reads stdin if not specified. |

```bash
forge-next import --file forge-backup.json
cat forge-backup.json | forge-next import
```

### ingest-claude

Ingest Claude Code's `MEMORY.md` files into Forge. Reads from `~/.claude/` directory structure.

```
forge-next ingest-claude
```

### migrate

Import a v1 `cache.json` file into the daemon.

```
forge-next migrate <STATE_DIR>
```

| Flag | Default | Description |
|------|---------|-------------|
| `<STATE_DIR>` | *(required)* | Path to v1 state directory containing `cache.json` |

```bash
forge-next migrate ~/.forge-v1
```

---

## Code Intelligence

### code-search

Search code symbols (functions, classes, files) by name pattern.

```
forge-next code-search <QUERY> [--kind KIND] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<QUERY>` | *(required)* | Symbol name pattern to search for |
| `--kind KIND` | *(all)* | Filter by symbol kind: `function`, `class`, `file` |
| `--limit N` | `20` | Maximum results |

```bash
forge-next code-search "authenticate" --kind function --limit 5
forge-next code-search "MyClass"
```

### force-index

Force-trigger the code indexer and show current index counts.

```
forge-next force-index
```

### detect-reality

Detect the project type (reality) for a path.

```
forge-next detect-reality [--path PATH]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--path PATH` | *(cwd)* | Path to detect. Defaults to current directory. |

```bash
forge-next detect-reality --path /home/user/my-rust-project
```

### realities

List all known realities (projects) in the system.

```
forge-next realities [--organization ORG]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--organization ORG` | `default` | Organization ID |

---

## Agent Teams

### agent-template create

Create a reusable agent template.

```
forge-next agent-template create --name NAME --description DESC --agent-type TYPE \
  [--system-context CTX] [--identity-facets JSON] [--config-overrides JSON] \
  [--knowledge-domains JSON] [--decision-style STYLE]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--name NAME` | *(required)* | Template name (e.g., `CTO`, `backend-engineer`) |
| `--description DESC` | *(required)* | Description of the agent's role |
| `--agent-type TYPE` | *(required)* | Agent type: `claude-code`, `cline`, etc. |
| `--system-context CTX` | *(none)* | System context / prompt for the agent |
| `--identity-facets JSON` | *(none)* | Identity facets as JSON array |
| `--config-overrides JSON` | *(none)* | Config overrides as JSON object |
| `--knowledge-domains JSON` | *(none)* | Knowledge domains as JSON array |
| `--decision-style STYLE` | *(none)* | Decision style: `analytical`, `intuitive`, `consensus`, `directive` |

```bash
forge-next agent-template create \
  --name "backend-engineer" \
  --description "Senior Rust backend engineer" \
  --agent-type claude-code \
  --decision-style analytical \
  --knowledge-domains '["rust", "databases", "distributed-systems"]'
```

### agent-template list

List all agent templates.

```
forge-next agent-template list [--org ORG]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--org ORG` | *(all)* | Filter by organization ID |

### agent-template get

Get a single agent template by name or ID.

```
forge-next agent-template get [--name NAME] [--id ID]
```

### agent-template delete

Delete an agent template.

```
forge-next agent-template delete --id ID
```

### agent spawn

Spawn an agent instance from a template.

```
forge-next agent spawn --template TEMPLATE --session-id ID [--project PROJECT] [--team TEAM]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--template TEMPLATE` | *(required)* | Template name to spawn from |
| `--session-id ID` | *(required)* | Session ID for the new agent |
| `--project PROJECT` | *(none)* | Project scope |
| `--team TEAM` | *(none)* | Team to join |

```bash
forge-next agent spawn --template backend-engineer --session-id agent-001 --project forge --team core
```

### agent retire

Retire an agent (soft delete).

```
forge-next agent retire --session SESSION
```

### agents

List active agents.

```
forge-next agents [--team TEAM]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--team TEAM` | *(all)* | Filter by team name |

### agent-status

Update an agent's status.

```
forge-next agent-status --session SESSION --status STATUS [--task DESCRIPTION]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session SESSION` | *(required)* | Session ID of the agent |
| `--status STATUS` | *(required)* | New status: `idle`, `thinking`, `responding`, `in_meeting`, `error` |
| `--task DESCRIPTION` | *(none)* | Current task description |

```bash
forge-next agent-status --session agent-001 --status thinking --task "Designing auth module"
```

---

## Teams

### team create

Create a team.

```
forge-next team create --name NAME [--type TYPE] [--purpose PURPOSE]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--name NAME` | *(required)* | Team name |
| `--type TYPE` | *(none)* | Team type: `human`, `agent`, `mixed` |
| `--purpose PURPOSE` | *(none)* | Team purpose description |

```bash
forge-next team create --name backend --type agent --purpose "Backend service development"
```

### team members

List members of a team.

```
forge-next team members --name NAME
```

### team set-orchestrator

Set the orchestrator session for a team.

```
forge-next team set-orchestrator --name NAME --session SESSION
```

### team status

Show full team status.

```
forge-next team status --name NAME
```

---

## Meetings

### meeting create

Create a meeting for team deliberation.

```
forge-next meeting create --team TEAM --topic TOPIC --orchestrator SESSION --participants S1,S2 [--context CTX]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--team TEAM` | *(required)* | Team ID |
| `--topic TOPIC` | *(required)* | Meeting topic |
| `--orchestrator SESSION` | *(required)* | Orchestrator session ID |
| `--participants S1,S2` | *(required)* | Comma-separated participant session IDs |
| `--context CTX` | *(none)* | Additional context for the meeting |

### meeting status

Get meeting status.

```
forge-next meeting status --id ID
```

### meeting responses

Get participant responses for a meeting.

```
forge-next meeting responses --id ID
```

### meeting synthesize

Store the orchestrator's synthesis for a meeting.

```
forge-next meeting synthesize --id ID --synthesis TEXT
```

### meeting decide

Record a decision and close the meeting.

```
forge-next meeting decide --id ID --decision TEXT
```

### meeting list

List meetings.

```
forge-next meeting list [--team TEAM] [--status STATUS]
```

### meeting transcript

Show full meeting transcript.

```
forge-next meeting transcript --id ID
```

### meeting vote

Cast a vote in a meeting. The choice must be one of the meeting's predefined voting options.

```
forge-next meeting vote --id <ID> --session <SESSION> --choice <CHOICE>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Meeting ID |
| `--session SESSION` | *(required)* | Session ID casting the vote |
| `--choice CHOICE` | *(required)* | Your choice (must be one of the meeting's voting options) |

```bash
forge-next meeting vote --id meeting-001 --session sess-abc123 --choice "yes"
```

### meeting result

Show vote results for a meeting.

```
forge-next meeting result --id <ID>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Meeting ID |

```bash
forge-next meeting result --id meeting-001
```

---

## Notifications

### notifications

List notifications for the current agent.

```
forge-next notifications [--status STATUS] [--category CATEGORY] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--status STATUS` | *(all)* | Filter: `pending`, `acknowledged`, `dismissed` |
| `--category CATEGORY` | *(all)* | Filter: `alert`, `insight`, `confirmation`, `progress` |
| `--limit N` | `10` | Maximum results |

### ack-notification

Acknowledge a notification.

```
forge-next ack-notification <ID>
```

### dismiss-notification

Dismiss a notification.

```
forge-next dismiss-notification <ID>
```

### act-notification

Act on a confirmation notification.

```
forge-next act-notification --id ID [--approve | --reject]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--id ID` | *(required)* | Notification ID |
| `--approve` | `false` | Approve the action |
| `--reject` | `false` | Reject the action |

---

## Daemon Management

### daemon status

Show daemon status: uptime, memory count, version.

```
forge-next daemon status
```

### daemon stop

Stop the daemon process.

```
forge-next daemon stop
```

---

## Service Management

Manage the daemon as a system service (systemd on Linux, launchd on macOS).

```bash
forge-next service install    # Install as system service
forge-next service start      # Start the service
forge-next service stop       # Stop the service
forge-next service status     # Show service status
forge-next service uninstall  # Remove the service
```

---

## Maintenance

### bootstrap

Scan and process all existing transcript files.

```
forge-next bootstrap [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--project PROJECT` | *(all)* | Only process transcripts for this project |

### backfill

Re-process a single transcript file from scratch.

```
forge-next backfill <PATH>
```

### consolidate

Force-run all consolidation phases: deduplication, decay, promotion, reweave.

```
forge-next consolidate
```

### extract

Trigger extraction on pending transcripts.

```
forge-next extract [--force]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--force` | `false` | Force extraction immediately, skipping debounce |

### hlc-backfill

Backfill HLC (Hybrid Logical Clock) timestamps on existing memories that have empty `hlc_timestamp`.

```
forge-next hlc-backfill
```

---

## Hooks (Prajna)

Proactive context hooks called automatically by Claude Code at various lifecycle points. These commands are typically invoked by hook scripts, not directly by users.

### context-refresh

Per-turn context delta check. Called by the `UserPromptSubmit` hook to inject fresh context (new memories, pending messages, diagnostics) since the last turn.

```
forge-next context-refresh --session-id <SESSION_ID> [--since TIMESTAMP]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session-id SESSION_ID` | *(required)* | Session ID to refresh context for |
| `--since TIMESTAMP` | *(none)* | Only return context changes since this timestamp |

```bash
forge-next context-refresh --session-id sess-abc123
forge-next context-refresh --session-id sess-abc123 --since "2026-04-07T10:00:00Z"
```

### completion-check

Check for premature completion signals. Called by the `Stop` hook to detect when an agent claims to be done but may have missed requirements.

```
forge-next completion-check --session-id <SESSION_ID> [--claimed-done]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session-id SESSION_ID` | *(required)* | Session ID to check |
| `--claimed-done` | `false` | Whether the agent explicitly claimed completion |

```bash
forge-next completion-check --session-id sess-abc123
forge-next completion-check --session-id sess-abc123 --claimed-done
```

### task-completion-check

Verify task completion criteria. Called by the `TaskCompleted` hook to ensure all acceptance criteria are met before a task is marked complete.

```
forge-next task-completion-check --session-id <SESSION_ID> --subject <SUBJECT> [--description DESCRIPTION]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session-id SESSION_ID` | *(required)* | Session ID to check |
| `--subject SUBJECT` | *(required)* | Subject/title of the completed task |
| `--description DESCRIPTION` | *(none)* | Additional task description for verification |

```bash
forge-next task-completion-check --session-id sess-abc123 --subject "Implement auth module"
forge-next task-completion-check --session-id sess-abc123 --subject "Fix login bug" --description "Users could not log in with SSO"
```

### context-stats

Context injection observability. Shows token cost, effectiveness metrics, and per-hook breakdown of context injections.

```
forge-next context-stats [--session-id SESSION_ID]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--session-id SESSION_ID` | *(none)* | Session ID (omit for global stats across all sessions) |

```bash
# Global stats
forge-next context-stats

# Per-session stats
forge-next context-stats --session-id sess-abc123
```

---

## Organization & Workspace

### org-create

Create a new organization.

```
forge-next org-create --name <NAME> [--description DESCRIPTION]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--name NAME` | *(required)* | Organization name |
| `--description DESCRIPTION` | *(none)* | Organization description |

```bash
forge-next org-create --name "Acme Corp" --description "Main development organization"
```

### org-list

List all organizations.

```
forge-next org-list
```

```bash
forge-next org-list
```

### org-from-template

Create an organization from a predefined template. Templates provide pre-configured teams, roles, and workspace structure.

```
forge-next org-from-template --template <TEMPLATE> --name <NAME>
```

| Flag | Default | Description |
|------|---------|-------------|
| `--template TEMPLATE` | *(required)* | Template name: `startup`, `devteam`, `agency` |
| `--name NAME` | *(required)* | Organization name |

```bash
forge-next org-from-template --template startup --name "MyStartup"
forge-next org-from-template --template devteam --name "Backend Team"
```

### org-init

Initialize workspace directories for an organization. Creates the standard directory structure under `~/.forge/workspace/`.

```
forge-next org-init --name <NAME> [--template TEMPLATE]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--name NAME` | *(required)* | Organization name |
| `--template TEMPLATE` | *(none)* | Template name (e.g., `startup`, `devteam`) |

```bash
forge-next org-init --name "MyOrg"
forge-next org-init --name "MyOrg" --template startup
```

### workspace-status

Show workspace status including mode, paths, and organization info.

```
forge-next workspace-status
```

```bash
forge-next workspace-status
```

### team-tree

Show the team hierarchy tree for an organization. Displays teams, sub-teams, and their relationships.

```
forge-next team-tree [--org ORG]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--org ORG` | *(none)* | Organization ID to show tree for |

```bash
forge-next team-tree
forge-next team-tree --org acme
```

### team-send

Send a FISP message to all sessions in a team. Optionally recurse into sub-teams.

```
forge-next team-send --team <TEAM> --kind <KIND> --topic <TOPIC> --text <TEXT> [--from FROM] [--recursive]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--team TEAM` | *(required)* | Team ID to send to |
| `--kind KIND` | *(required)* | Message kind: `notification` or `request` |
| `--topic TOPIC` | *(required)* | Message topic |
| `--text TEXT` | *(required)* | Message body |
| `--from FROM` | *(none)* | Sender session ID |
| `--recursive` | `false` | Also send to all sub-team sessions |

```bash
forge-next team-send --team backend --kind notification --topic deployment --text "v2.1.0 deployed to staging"
forge-next team-send --team engineering --kind notification --topic standup --text "Standup in 5 minutes" --recursive
```

---

## Skills Registry

### skills-list

List skills from the registry. Filter by category or search by keyword.

```
forge-next skills-list [--category CATEGORY] [--search SEARCH] [--limit N]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--category CATEGORY` | *(all)* | Filter by skill category |
| `--search SEARCH` | *(none)* | Search by keyword |
| `--limit N` | `20` | Maximum results |

```bash
forge-next skills-list
forge-next skills-list --category development --limit 10
forge-next skills-list --search "testing"
```

### skills-install

Install a skill for a project.

```
forge-next skills-install <NAME> [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<NAME>` | *(required)* | Skill name to install |
| `--project PROJECT` | `""` | Project to install for |

```bash
forge-next skills-install tdd-workflow
forge-next skills-install code-review --project my-app
```

### skills-uninstall

Uninstall a skill from a project.

```
forge-next skills-uninstall <NAME> [--project PROJECT]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<NAME>` | *(required)* | Skill name to uninstall |
| `--project PROJECT` | `""` | Project to uninstall from |

```bash
forge-next skills-uninstall tdd-workflow
forge-next skills-uninstall code-review --project my-app
```

### skills-info

Get detailed information about a skill.

```
forge-next skills-info <NAME>
```

| Flag | Default | Description |
|------|---------|-------------|
| `<NAME>` | *(required)* | Skill name |

```bash
forge-next skills-info tdd-workflow
```

### skills-refresh

Re-index the skills directory. Forces a rescan of all installed skills.

```
forge-next skills-refresh
```

```bash
forge-next skills-refresh
```

---

## License

### license-status

Show the current license tier and key status.

```
forge-next license-status
```

```bash
forge-next license-status
```

### license-set

Set the license tier and key.

```
forge-next license-set --tier <TIER> [--key KEY]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--tier TIER` | *(required)* | License tier: `free`, `pro`, `team`, `enterprise` |
| `--key KEY` | `""` | License key |

```bash
forge-next license-set --tier pro --key "FORGE-PRO-XXXX-XXXX-XXXX"
forge-next license-set --tier free
```

---

## Memory Quality

### backfill-project

Backfill the project field on memories that have NULL or empty project values. Derives the correct project from the session registry.

```
forge-next backfill-project
```

```bash
forge-next backfill-project
```

### cleanup-memory

Cleanup garbage memories, normalize project names, and purge duplicate perceptions and declared entries.

```
forge-next cleanup-memory
```

```bash
forge-next cleanup-memory
```

### healing-status

Show the current memory healing status: pending repairs, last run timestamp, and health metrics.

```
forge-next healing-status
```

```bash
forge-next healing-status
```

### healing-run

Trigger a manual healing cycle. Runs deduplication, auto-supersession, and decay on stale memories.

```
forge-next healing-run
```

```bash
forge-next healing-run
```

### healing-log

Show the healing history log: past healing actions with timestamps and details.

```
forge-next healing-log [--limit N] [--action ACTION]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit N` | `20` | Maximum entries to show |
| `--action ACTION` | *(all)* | Filter by action type: `auto_superseded`, `auto_faded` |

```bash
forge-next healing-log
forge-next healing-log --limit 50
forge-next healing-log --action auto_superseded
```
