# API Reference

Forge exposes a single HTTP endpoint (`POST /api`) that accepts JSON-encoded requests. All operations -- reads, writes, and administrative actions -- go through this endpoint. Health probes and metrics are served on separate paths.

## Transport

### HTTP

| Property | Value |
|----------|-------|
| Base URL | `POST /api` |
| Content-Type | `application/json` |
| Auth | `Authorization: Bearer <JWT>` (when `auth.enabled = true`) |
| Body limit | 10 MB |
| Write timeout | 30 seconds |

### Unix Domain Socket

The same JSON protocol works over the Unix domain socket at `~/.forge/daemon.sock`. Socket requests bypass JWT authentication and RBAC (filesystem permissions are the trust boundary).

```bash
echo '{"method":"health"}' | socat - UNIX-CONNECT:~/.forge/daemon.sock
```

### Health Probes

These endpoints require no authentication and accept `GET` requests.

| Endpoint | Purpose |
|----------|---------|
| `GET /healthz` | Liveness -- returns 200 if the daemon process is running |
| `GET /readyz` | Readiness -- returns 200 if the database is queryable |
| `GET /startupz` | Startup -- returns 200 if initial indexing is complete |
| `GET /metrics` | Prometheus text format metrics (when `metrics.enabled = true`) |

## Request Format

Every request is a JSON object with a `method` field and an optional `params` field:

```json
{"method": "<method_name>", "params": { ... }}
```

Methods without parameters omit the `params` field:

```json
{"method": "health"}
```

## Response Format

Success:

```json
{
  "status": "ok",
  "data": {
    "kind": "<response_type>",
    ...
  }
}
```

Error:

```json
{
  "status": "error",
  "message": "<error description>"
}
```

Protocol-level errors (invalid method, missing field) return HTTP 200 with `"status": "error"`. Infrastructure failures return non-200 HTTP status codes.

## HTTP Status Codes

| Code | Meaning |
|------|---------|
| 200 | Success (or protocol-level error in JSON body) |
| 400 | Invalid request body (not valid HTTP) |
| 401 | Missing or invalid JWT (auth enabled) |
| 403 | RBAC permission denied |
| 405 | Wrong HTTP method (e.g., GET on /api) |
| 422 | Invalid JSON or missing required fields |
| 503 | Database unavailable or writer actor down |
| 504 | Write timeout (30 seconds exceeded) |

## Roles

Each endpoint requires a minimum role. The three roles are:

- **Viewer:** Read-only access
- **Member:** Read + write access (default for authenticated users)
- **Admin:** Full access including administrative operations

---

## Memory Operations

### remember

Store a new memory.

**Role:** Member

**Request:**

```json
{
  "method": "remember",
  "params": {
    "memory_type": "decision",
    "title": "Use PostgreSQL for user data",
    "content": "Chose PostgreSQL over MySQL for better JSON support and ACID compliance.",
    "confidence": 0.95,
    "tags": ["database", "architecture"],
    "project": "myapp"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `memory_type` | string | yes | One of: `decision`, `lesson`, `pattern`, `preference`, `protocol` |
| `title` | string | yes | Short title for the memory |
| `content` | string | yes | Full content of the memory |
| `confidence` | float | no | Confidence score (0.0 to 1.0) |
| `tags` | string[] | no | Tags for categorization |
| `project` | string | no | Project scope |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "stored",
    "id": "01JQXYZ123ABC"
  }
}
```

### recall

Search memories by natural language query.

**Role:** Viewer

**Request:**

```json
{
  "method": "recall",
  "params": {
    "query": "database decisions",
    "memory_type": "decision",
    "project": "myapp",
    "limit": 10,
    "layer": "experience"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | Natural language search query |
| `memory_type` | string | no | Filter by type: `decision`, `lesson`, `pattern`, `preference`, `protocol` |
| `project` | string | no | Filter by project |
| `limit` | integer | no | Maximum results |
| `layer` | string | no | Filter by Manas layer: `experience`, `declared`, `domain_dna`, `perception`, `identity` |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "memories",
    "results": [
      {
        "id": "01JQXYZ123ABC",
        "memory_type": "decision",
        "title": "Use PostgreSQL for user data",
        "content": "Chose PostgreSQL over MySQL...",
        "confidence": 0.95,
        "tags": ["database", "architecture"],
        "created_at": "2026-04-05T10:30:00Z"
      }
    ],
    "count": 1
  }
}
```

### forget

Delete a memory by ID.

**Role:** Member

**Request:**

```json
{
  "method": "forget",
  "params": {
    "id": "01JQXYZ123ABC"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Memory ID to delete |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "forgotten",
    "id": "01JQXYZ123ABC"
  }
}
```

### batch_recall

Execute multiple recall queries in a single request. Eliminates N+1 round trips.

**Role:** Viewer

**Request:**

```json
{
  "method": "batch_recall",
  "params": {
    "queries": [
      {"text": "database decisions", "memory_type": "decision", "limit": 5},
      {"text": "auth patterns", "limit": 3}
    ]
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `queries` | RecallQuery[] | yes | Array of recall queries |

Each `RecallQuery`:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `text` | string | yes | Search query text |
| `memory_type` | string | no | Filter by memory type |
| `limit` | integer | no | Maximum results per query |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "batch_recall_results",
    "results": [
      [{"id": "...", "title": "...", "content": "..."}],
      [{"id": "...", "title": "...", "content": "..."}]
    ]
  }
}
```

### health

Get memory counts by type.

**Role:** Viewer

**Request:**

```json
{"method": "health"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "health",
    "decisions": 42,
    "lessons": 18,
    "patterns": 7,
    "preferences": 3,
    "edges": 156
  }
}
```

### doctor

Comprehensive system diagnostics.

**Role:** Viewer

**Request:**

```json
{"method": "doctor"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "doctor",
    "daemon_up": true,
    "db_size_bytes": 52428800,
    "memory_count": 70,
    "embedding_count": 65,
    "file_count": 250,
    "symbol_count": 1200,
    "edge_count": 156,
    "workers": ["extraction", "consolidation", "embedding", "code_indexer", "transcript_watcher", "perception_decay", "notification_engine", "entity_linker"],
    "uptime_secs": 86400,
    "platform_count": 5,
    "tool_count": 12,
    "skill_count": 3,
    "domain_dna_count": 8,
    "perception_count": 2,
    "declared_count": 15,
    "identity_count": 4,
    "disposition_count": 6
  }
}
```

### export

Export all data as JSON.

**Role:** Viewer

**Request:**

```json
{
  "method": "export",
  "params": {
    "format": "json",
    "since": "2026-04-01T00:00:00Z"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `format` | string | no | `json` (default) or `ndjson` |
| `since` | string | no | ISO 8601 timestamp filter |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "export",
    "memories": [...],
    "files": [...],
    "symbols": [...],
    "edges": [...]
  }
}
```

### import

Import data from a JSON export.

**Role:** Admin

**Request:**

```json
{
  "method": "import",
  "params": {
    "data": "{\"memories\":[...],\"files\":[...],\"symbols\":[...],\"edges\":[...]}"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `data` | string | yes | JSON string of exported data |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "import",
    "memories_imported": 42,
    "files_imported": 10,
    "symbols_imported": 50,
    "skipped": 3
  }
}
```

---

## Session Management

### register_session

Register an active agent session. Used by agent adapters to track active sessions.

**Role:** Member

**Request:**

```json
{
  "method": "register_session",
  "params": {
    "id": "session-abc-123",
    "agent": "claude-code",
    "project": "myapp",
    "cwd": "/home/user/projects/myapp",
    "capabilities": ["code_review", "testing"],
    "current_task": "Implementing auth module"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Unique session identifier |
| `agent` | string | yes | Agent type: `claude-code`, `cline`, `codex` |
| `project` | string | no | Project name |
| `cwd` | string | no | Working directory |
| `capabilities` | string[] | no | A2A: capabilities this session advertises |
| `current_task` | string | no | A2A: description of current work |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "session_registered",
    "id": "session-abc-123"
  }
}
```

### end_session

Mark a session as ended.

**Role:** Member

**Request:**

```json
{
  "method": "end_session",
  "params": {
    "id": "session-abc-123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Session ID to end |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "session_ended",
    "id": "session-abc-123",
    "found": true
  }
}
```

### sessions

List active or all sessions.

**Role:** Viewer

**Request:**

```json
{
  "method": "sessions",
  "params": {
    "active_only": true
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `active_only` | boolean | no | If true, only return active sessions |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sessions",
    "sessions": [
      {
        "id": "session-abc-123",
        "agent": "claude-code",
        "project": "myapp",
        "started_at": "2026-04-05T10:00:00Z",
        "active": true
      }
    ],
    "count": 1
  }
}
```

### cleanup_sessions

End all active sessions, optionally filtered by ID prefix.

**Role:** Admin

**Request:**

```json
{
  "method": "cleanup_sessions",
  "params": {
    "prefix": "test-"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `prefix` | string | no | Only end sessions whose ID starts with this prefix. If omitted, ends all sessions. |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sessions_cleaned",
    "ended": 5
  }
}
```

---

## A2A Messaging (FISP)

The Forge Inter-Session Protocol (FISP) enables agent-to-agent communication. Sessions can send notifications, make requests, and broadcast messages.

### session_send

Send a message to another session or broadcast to all sessions.

**Role:** Member

**Request:**

```json
{
  "method": "session_send",
  "params": {
    "to": "session-xyz-456",
    "kind": "notification",
    "topic": "schema_changed",
    "parts": [
      {"kind": "text", "text": "The users table schema was updated."}
    ],
    "project": "myapp",
    "timeout_secs": 300
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `to` | string | yes | Target session ID, or `"*"` for broadcast |
| `kind` | string | yes | `notification` or `request` |
| `topic` | string | yes | Message topic |
| `parts` | MessagePart[] | yes | Message content parts |
| `project` | string | no | Project scope for broadcasts |
| `timeout_secs` | integer | no | Request timeout in seconds |
| `meeting_id` | string | no | If set, auto-records as a meeting response |

Each `MessagePart`:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | yes | `text`, `file`, `data`, or `memory_ref` |
| `text` | string | no | Text content (for `text` kind) |
| `path` | string | no | File path (for `file` kind) |
| `data` | object | no | Structured JSON data (for `data` kind) |
| `memory_id` | string | no | Memory reference (for `memory_ref` kind) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "message_sent",
    "id": "01JQXYZ789DEF",
    "status": "delivered"
  }
}
```

### session_messages

Get pending messages for a session.

**Role:** Viewer

**Request:**

```json
{
  "method": "session_messages",
  "params": {
    "session_id": "session-abc-123",
    "status": "pending",
    "limit": 50
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `session_id` | string | yes | Session to get messages for |
| `status` | string | no | Filter by status: `pending`, `read` |
| `limit` | integer | no | Maximum messages to return |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "session_message_list",
    "messages": [
      {
        "id": "01JQXYZ789DEF",
        "from_session": "session-xyz-456",
        "kind": "notification",
        "topic": "schema_changed",
        "parts": [{"kind": "text", "text": "The users table schema was updated."}],
        "status": "pending",
        "created_at": "2026-04-05T10:30:00Z"
      }
    ],
    "count": 1
  }
}
```

### session_ack

Mark messages as read/consumed.

**Role:** Member

**Request:**

```json
{
  "method": "session_ack",
  "params": {
    "message_ids": ["01JQXYZ789DEF", "01JQXYZ789GHI"],
    "session_id": "session-abc-123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `message_ids` | string[] | yes | Message IDs to acknowledge |
| `session_id` | string | no | If set, only ack messages addressed to this session (ownership check) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "messages_acked",
    "count": 2
  }
}
```

---

## Agent Templates

### create_agent_template

Create a reusable agent template (e.g., CTO, CMO, QA Lead).

**Role:** Member

**Request:**

```json
{
  "method": "create_agent_template",
  "params": {
    "name": "CTO",
    "description": "Chief Technology Officer - architecture and technical decisions",
    "agent_type": "executive",
    "organization_id": "org-123",
    "system_context": "You are the CTO. Focus on architecture, scalability, and technical debt.",
    "identity_facets": "{\"communication_style\": \"direct\", \"risk_tolerance\": \"medium\"}",
    "config_overrides": "{\"extraction.backend\": \"claude_api\"}",
    "knowledge_domains": "architecture,security,infrastructure",
    "decision_style": "analytical"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | yes | Template name (unique within org) |
| `description` | string | yes | What this agent does |
| `agent_type` | string | yes | Category (e.g., `executive`, `specialist`, `reviewer`) |
| `organization_id` | string | no | Organization scope |
| `system_context` | string | no | System prompt fragment |
| `identity_facets` | string | no | JSON string of identity facets |
| `config_overrides` | string | no | JSON string of config overrides |
| `knowledge_domains` | string | no | Comma-separated domains |
| `decision_style` | string | no | How this agent makes decisions |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_template_created",
    "id": "01JQTPL123ABC",
    "name": "CTO"
  }
}
```

### list_agent_templates

List available agent templates.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_agent_templates",
  "params": {
    "organization_id": "org-123",
    "limit": 50
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `organization_id` | string | no | Filter by organization |
| `limit` | integer | no | Maximum results |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_template_list",
    "templates": [...],
    "count": 5
  }
}
```

### get_agent_template

Get a single agent template by ID or name.

**Role:** Viewer

**Request:**

```json
{
  "method": "get_agent_template",
  "params": {
    "name": "CTO"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | no | Template ID (provide id or name) |
| `name` | string | no | Template name (provide id or name) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_template_data",
    "template": {
      "id": "01JQTPL123ABC",
      "name": "CTO",
      "description": "Chief Technology Officer...",
      "agent_type": "executive"
    }
  }
}
```

### update_agent_template

Update fields on an existing agent template.

**Role:** Member

**Request:**

```json
{
  "method": "update_agent_template",
  "params": {
    "id": "01JQTPL123ABC",
    "description": "Updated description",
    "decision_style": "collaborative"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Template ID to update |
| `name` | string | no | New name |
| `description` | string | no | New description |
| `system_context` | string | no | New system context |
| `identity_facets` | string | no | New identity facets JSON |
| `config_overrides` | string | no | New config overrides JSON |
| `knowledge_domains` | string | no | New knowledge domains |
| `decision_style` | string | no | New decision style |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_template_updated",
    "id": "01JQTPL123ABC",
    "updated": true
  }
}
```

### delete_agent_template

Delete an agent template.

**Role:** Member

**Request:**

```json
{
  "method": "delete_agent_template",
  "params": {
    "id": "01JQTPL123ABC"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Template ID to delete |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_template_deleted",
    "id": "01JQTPL123ABC",
    "found": true
  }
}
```

---

## Agent Lifecycle

### spawn_agent

Spawn an agent from a template. Creates a session, sets identity facets, and optionally joins a team.

**Role:** Member

**Request:**

```json
{
  "method": "spawn_agent",
  "params": {
    "template_name": "CTO",
    "session_id": "agent-cto-001",
    "project": "myapp",
    "team": "leadership"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `template_name` | string | yes | Name of the template to spawn from |
| `session_id` | string | yes | Session ID for the spawned agent |
| `project` | string | no | Project to assign |
| `team` | string | no | Team to join |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_spawned",
    "session_id": "agent-cto-001",
    "template_name": "CTO",
    "team": "leadership"
  }
}
```

### list_agents

List active agents (sessions that were spawned from templates).

**Role:** Viewer

**Request:**

```json
{
  "method": "list_agents",
  "params": {
    "team": "leadership",
    "limit": 50
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team` | string | no | Filter by team name |
| `limit` | integer | no | Maximum results |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_list",
    "agents": [...],
    "count": 3
  }
}
```

### update_agent_status

Update an agent's status and optionally its current task.

**Role:** Member

**Request:**

```json
{
  "method": "update_agent_status",
  "params": {
    "session_id": "agent-cto-001",
    "status": "working",
    "current_task": "Reviewing architecture proposal"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `session_id` | string | yes | Agent session ID |
| `status` | string | yes | New status (e.g., `idle`, `working`, `blocked`) |
| `current_task` | string | no | Description of current work |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_status_updated",
    "session_id": "agent-cto-001",
    "status": "working"
  }
}
```

### retire_agent

Retire an agent. Soft delete that preserves all memories created by the agent.

**Role:** Member

**Request:**

```json
{
  "method": "retire_agent",
  "params": {
    "session_id": "agent-cto-001"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `session_id` | string | yes | Agent session ID to retire |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "agent_retired",
    "session_id": "agent-cto-001"
  }
}
```

---

## Teams

### create_team

Create a team of agents and/or humans.

**Role:** Member

**Request:**

```json
{
  "method": "create_team",
  "params": {
    "name": "leadership",
    "team_type": "agent",
    "purpose": "Executive decision-making team",
    "organization_id": "org-123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | yes | Team name |
| `team_type` | string | no | `human`, `agent`, or `mixed` |
| `purpose` | string | no | Team description |
| `organization_id` | string | no | Organization scope |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "team_created",
    "id": "01JQTEAM123",
    "name": "leadership"
  }
}
```

### list_team_members

List members of a team.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_team_members",
  "params": {
    "team_name": "leadership"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team_name` | string | yes | Team name to query |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "team_member_list",
    "members": [...],
    "count": 3
  }
}
```

### set_team_orchestrator

Designate a session as the team orchestrator (coordinator).

**Role:** Member

**Request:**

```json
{
  "method": "set_team_orchestrator",
  "params": {
    "team_name": "leadership",
    "session_id": "agent-cto-001"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team_name` | string | yes | Team name |
| `session_id` | string | yes | Session ID of the orchestrator |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "team_orchestrator_set",
    "team_name": "leadership",
    "session_id": "agent-cto-001"
  }
}
```

### team_status

Get full team status including members, meetings, and decisions.

**Role:** Viewer

**Request:**

```json
{
  "method": "team_status",
  "params": {
    "team_name": "leadership"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team_name` | string | yes | Team name |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "team_status_data",
    "team": { ... }
  }
}
```

---

## Meetings

The meeting protocol enables structured multi-agent deliberation. An orchestrator creates a meeting, participants respond, the orchestrator synthesizes, and then records a decision.

### create_meeting

Create a meeting and send FISP messages to all participants.

**Role:** Member

**Request:**

```json
{
  "method": "create_meeting",
  "params": {
    "team_id": "01JQTEAM123",
    "topic": "Should we migrate to microservices?",
    "context": "Current monolith is hitting scale limits at 10k RPS.",
    "orchestrator_session_id": "agent-cto-001",
    "participant_session_ids": ["agent-cto-001", "agent-sre-001", "agent-arch-001"]
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team_id` | string | yes | Team ID |
| `topic` | string | yes | Meeting topic / question |
| `context` | string | no | Background context |
| `orchestrator_session_id` | string | yes | Session running the meeting |
| `participant_session_ids` | string[] | yes | Sessions invited to participate |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_created",
    "meeting_id": "01JQMEET456",
    "participant_count": 3
  }
}
```

### meeting_status

Get meeting status and participant response statuses.

**Role:** Viewer

**Request:**

```json
{
  "method": "meeting_status",
  "params": {
    "meeting_id": "01JQMEET456"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_status_data",
    "meeting": { "id": "01JQMEET456", "topic": "...", "status": "in_progress" },
    "participants": [
      {"session_id": "agent-cto-001", "responded": true},
      {"session_id": "agent-sre-001", "responded": false}
    ]
  }
}
```

### meeting_responses

Get all participant responses for a meeting.

**Role:** Viewer

**Request:**

```json
{
  "method": "meeting_responses",
  "params": {
    "meeting_id": "01JQMEET456"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_response_list",
    "responses": [...],
    "count": 2
  }
}
```

### record_meeting_response

Directly record a participant's response (alternative to FISP side-effect).

**Role:** Member

**Request:**

```json
{
  "method": "record_meeting_response",
  "params": {
    "meeting_id": "01JQMEET456",
    "session_id": "agent-sre-001",
    "response": "I recommend a phased migration starting with the auth service.",
    "confidence": 0.85
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |
| `session_id` | string | yes | Responding session |
| `response` | string | yes | Response text |
| `confidence` | float | no | Confidence in the response (0.0 to 1.0) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_response_recorded",
    "meeting_id": "01JQMEET456",
    "all_responded": false
  }
}
```

### meeting_synthesize

Store the orchestrator's synthesis of all responses.

**Role:** Member

**Request:**

```json
{
  "method": "meeting_synthesize",
  "params": {
    "meeting_id": "01JQMEET456",
    "synthesis": "Consensus: phased migration. Start with auth (CTO agrees), monitoring first (SRE)."
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |
| `synthesis` | string | yes | Synthesis text |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_synthesized",
    "meeting_id": "01JQMEET456"
  }
}
```

### meeting_decide

Record a decision, store it as a memory, and close the meeting.

**Role:** Member

**Request:**

```json
{
  "method": "meeting_decide",
  "params": {
    "meeting_id": "01JQMEET456",
    "decision": "Proceed with phased microservices migration starting Q3."
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |
| `decision` | string | yes | Decision text (stored as a `decision` memory) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_decided",
    "meeting_id": "01JQMEET456",
    "decision_memory_id": "01JQDEC789"
  }
}
```

### list_meetings

List meetings, optionally filtered by team or status.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_meetings",
  "params": {
    "team_id": "01JQTEAM123",
    "status": "completed",
    "limit": 20
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `team_id` | string | no | Filter by team |
| `status` | string | no | Filter by status |
| `limit` | integer | no | Maximum results |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_list",
    "meetings": [...],
    "count": 5
  }
}
```

### meeting_transcript

Get the full meeting transcript (topic, context, responses, synthesis, decision).

**Role:** Viewer

**Request:**

```json
{
  "method": "meeting_transcript",
  "params": {
    "meeting_id": "01JQMEET456"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `meeting_id` | string | yes | Meeting ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "meeting_transcript_data",
    "transcript": {
      "topic": "Should we migrate to microservices?",
      "context": "Current monolith is hitting scale limits...",
      "responses": [...],
      "synthesis": "...",
      "decision": "Proceed with phased migration..."
    }
  }
}
```

---

## Identity (Ahankara)

The identity system stores per-agent personality facets that influence behavior, communication style, and decision-making.

### store_identity

Store or update an identity facet for an agent.

**Role:** Member

**Request:**

```json
{
  "method": "store_identity",
  "params": {
    "facet": {
      "agent": "claude-code",
      "facet_name": "communication_style",
      "description": "Direct, concise, no filler words. Prefer code examples over prose.",
      "source": "user_declared"
    }
  }
}
```

The `facet` object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent` | string | yes | Agent identifier |
| `facet_name` | string | yes | Facet name (e.g., `communication_style`, `risk_tolerance`) |
| `description` | string | yes | Facet content |
| `source` | string | yes | Origin: `user_declared`, `self_observed`, `peer_feedback` |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "identity_stored",
    "id": "01JQID123"
  }
}
```

### list_identity

List identity facets for an agent.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_identity",
  "params": {
    "agent": "claude-code"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `agent` | string | yes | Agent identifier |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "identity_list",
    "facets": [
      {
        "id": "01JQID123",
        "agent": "claude-code",
        "facet_name": "communication_style",
        "description": "Direct, concise...",
        "source": "user_declared",
        "active": true
      }
    ],
    "count": 1
  }
}
```

### deactivate_identity

Deactivate an identity facet (soft delete).

**Role:** Member

**Request:**

```json
{
  "method": "deactivate_identity",
  "params": {
    "id": "01JQID123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Identity facet ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "identity_deactivated",
    "id": "01JQID123",
    "found": true
  }
}
```

---

## Configuration

### get_config

Get current daemon configuration.

**Role:** Viewer

**Request:**

```json
{"method": "get_config"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "config_data",
    "backend": "ollama",
    "ollama_model": "llama3.2",
    "ollama_endpoint": "http://localhost:11434",
    "claude_cli_model": "claude-sonnet-4-20250514",
    "claude_api_model": "claude-sonnet-4-20250514",
    "claude_api_key_set": false,
    "openai_model": "gpt-4o",
    "openai_endpoint": "https://api.openai.com/v1",
    "openai_key_set": false,
    "gemini_model": "gemini-2.5-flash",
    "gemini_key_set": true,
    "embedding_model": "all-MiniLM-L6-v2"
  }
}
```

### set_config

Update a configuration value by dotted key.

**Role:** Admin

**Request:**

```json
{
  "method": "set_config",
  "params": {
    "key": "extraction.backend",
    "value": "gemini"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `key` | string | yes | Dotted config key (e.g., `extraction.backend`) |
| `value` | string | yes | New value |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "config_updated",
    "key": "extraction.backend",
    "value": "gemini"
  }
}
```

### get_effective_config

Get the effective (resolved) configuration for a scope chain. Scoped configuration cascades: organization -> team -> user -> agent -> session.

**Role:** Viewer

**Request:**

```json
{
  "method": "get_effective_config",
  "params": {
    "session_id": "session-abc-123",
    "agent": "claude-code",
    "organization_id": "org-123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `session_id` | string | no | Session scope |
| `agent` | string | no | Agent scope |
| `reality_id` | string | no | Reality (project) scope |
| `user_id` | string | no | User scope |
| `team_id` | string | no | Team scope |
| `organization_id` | string | no | Organization scope |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "effective_config",
    "config": {
      "extraction.backend": {"value": "gemini", "scope": "organization", "locked": false}
    }
  }
}
```

### set_scoped_config

Set a configuration value scoped to a specific entity.

**Role:** Admin

**Request:**

```json
{
  "method": "set_scoped_config",
  "params": {
    "scope_type": "organization",
    "scope_id": "org-123",
    "key": "extraction.backend",
    "value": "claude_api",
    "locked": true,
    "ceiling": null
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `scope_type` | string | yes | Scope level: `organization`, `team`, `user`, `agent`, `session`, `reality` |
| `scope_id` | string | yes | Entity ID at that scope |
| `key` | string | yes | Config key |
| `value` | string | yes | Config value |
| `locked` | boolean | yes | If true, lower scopes cannot override |
| `ceiling` | float | no | Maximum numeric value for lower scopes |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "scoped_config_set",
    "scope_type": "organization",
    "scope_id": "org-123",
    "key": "extraction.backend"
  }
}
```

### delete_scoped_config

Delete a scoped configuration entry.

**Role:** Admin

**Request:**

```json
{
  "method": "delete_scoped_config",
  "params": {
    "scope_type": "organization",
    "scope_id": "org-123",
    "key": "extraction.backend"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `scope_type` | string | yes | Scope level |
| `scope_id` | string | yes | Entity ID |
| `key` | string | yes | Config key to delete |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "scoped_config_deleted",
    "deleted": true
  }
}
```

### list_scoped_config

List all configuration entries for a scope.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_scoped_config",
  "params": {
    "scope_type": "organization",
    "scope_id": "org-123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `scope_type` | string | yes | Scope level |
| `scope_id` | string | yes | Entity ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "scoped_config_list",
    "entries": [
      {
        "key": "extraction.backend",
        "value": "claude_api",
        "locked": true,
        "ceiling": null
      }
    ]
  }
}
```

---

## Guardrails

### guardrails_check

Pre-execution check: are there decisions, lessons, or patterns linked to this file?

**Role:** Viewer

**Request:**

```json
{
  "method": "guardrails_check",
  "params": {
    "file": "src/auth.rs",
    "action": "edit"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file` | string | yes | File path to check |
| `action` | string | yes | Planned action (e.g., `edit`, `delete`) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "guardrails_check",
    "safe": false,
    "warnings": ["2 decisions reference this file"],
    "decisions_affected": ["Use JWT RS256 for auth"],
    "callers_count": 5,
    "calling_files": ["src/server/http.rs", "src/server/socket.rs"],
    "relevant_lessons": ["Always validate nbf claim"],
    "dangerous_patterns": [],
    "applicable_skills": []
  }
}
```

### blast_radius

Analyze the impact of changing a file.

**Role:** Viewer

**Request:**

```json
{
  "method": "blast_radius",
  "params": {
    "file": "src/auth.rs"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file` | string | yes | File path to analyze |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "blast_radius",
    "decisions": [
      {"title": "Use JWT RS256 for auth", "id": "01JQ..."}
    ],
    "callers": 5,
    "importers": ["src/server/http.rs"],
    "files_affected": ["src/server/http.rs", "src/server/socket.rs"],
    "cluster_name": "auth_module",
    "cluster_files": ["src/auth.rs", "src/rbac.rs", "src/server/http.rs"],
    "calling_files": ["src/server/http.rs", "src/server/socket.rs"]
  }
}
```

### pre_bash_check

Pre-execution check for shell commands. Warns about destructive commands and surfaces relevant skills.

**Role:** Viewer

**Request:**

```json
{
  "method": "pre_bash_check",
  "params": {
    "command": "rm -rf /tmp/data"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `command` | string | yes | Shell command to check |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "pre_bash_checked",
    "safe": true,
    "warnings": [],
    "relevant_skills": []
  }
}
```

### post_bash_check

Post-execution check for failed commands. Surfaces relevant lessons and suggestions.

**Role:** Viewer

**Request:**

```json
{
  "method": "post_bash_check",
  "params": {
    "command": "cargo build",
    "exit_code": 1
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `command` | string | yes | Command that was executed |
| `exit_code` | integer | yes | Exit code (non-zero = failure) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "post_bash_checked",
    "suggestions": ["Check that all dependencies are in Cargo.toml"]
  }
}
```

### post_edit_check

Post-edit check: surface callers, lessons, and patterns after a file edit.

**Role:** Viewer

**Request:**

```json
{
  "method": "post_edit_check",
  "params": {
    "file": "src/auth.rs"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file` | string | yes | File that was edited |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "post_edit_checked",
    "file": "src/auth.rs",
    "callers_count": 5,
    "calling_files": ["src/server/http.rs"],
    "relevant_lessons": [],
    "dangerous_patterns": [],
    "applicable_skills": [],
    "decisions_to_review": ["Use JWT RS256 for auth"],
    "cached_diagnostics": []
  }
}
```

---

## Context

### compile_context

Compile optimized context from all Manas layers. This is what gets injected into agent sessions at startup.

**Role:** Viewer

**Request:**

```json
{
  "method": "compile_context",
  "params": {
    "agent": "claude-code",
    "project": "myapp",
    "static_only": false,
    "excluded_layers": ["perceptions"]
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `agent` | string | no | Agent identifier |
| `project` | string | no | Project scope |
| `static_only` | boolean | no | If true, return only the static prefix (for KV-cache optimization) |
| `excluded_layers` | string[] | no | Layer names to exclude: `decisions`, `lessons`, `skills`, `perceptions`, `working_set`, `active_sessions` |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "compiled_context",
    "context": "<forge-context>...</forge-context>",
    "static_prefix": "<forge-static>...</forge-static>",
    "dynamic_suffix": "<forge-dynamic>...</forge-dynamic>",
    "layers_used": 6,
    "chars": 4200
  }
}
```

### compile_context_trace

Compile context with full trace showing what was considered, included, and excluded, with reasons. Used for debugging context assembly.

**Role:** Viewer

**Request:**

```json
{
  "method": "compile_context_trace",
  "params": {
    "agent": "claude-code",
    "project": "myapp"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `agent` | string | no | Agent identifier |
| `project` | string | no | Project scope |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "context_trace",
    "considered": [{"id": "...", "title": "...", "layer": "experience", "reason": "high relevance"}],
    "included": [...],
    "excluded": [{"id": "...", "title": "...", "layer": "experience", "reason": "budget exceeded"}],
    "budget_total": 8000,
    "budget_used": 6200,
    "layer_chars": {"experience": 3000, "declared": 1500, "domain_dna": 1700}
  }
}
```

### manas_health

Extended health across all 8 Manas memory layers.

**Role:** Viewer

**Request:**

```json
{
  "method": "manas_health",
  "params": {
    "project": "myapp"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `project` | string | no | Project for `is_new_project` check |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "manas_health_data",
    "platform_count": 5,
    "tool_count": 12,
    "skill_count": 3,
    "domain_dna_count": 8,
    "perception_unconsumed": 2,
    "declared_count": 15,
    "identity_facets": 4,
    "disposition_traits": 6,
    "experience_count": 70,
    "embedding_count": 65,
    "trait_names": ["analytical", "direct"],
    "is_new_project": false
  }
}
```

---

## Sync

Forge supports peer-to-peer memory synchronization using Hybrid Logical Clocks (HLC) for conflict-free merging.

### sync_export

Export memories as NDJSON lines with HLC metadata.

**Role:** Viewer

**Request:**

```json
{
  "method": "sync_export",
  "params": {
    "project": "myapp",
    "since": "2026-04-01T00:00:00Z"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `project` | string | no | Filter by project |
| `since` | string | no | Only export memories modified after this timestamp |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sync_exported",
    "lines": ["{\"id\":\"01JQ...\",\"hlc_timestamp\":\"...\", ...}"],
    "count": 42,
    "node_id": "node-abc-123"
  }
}
```

### sync_import

Import NDJSON memory lines from a remote node.

**Role:** Admin

**Request:**

```json
{
  "method": "sync_import",
  "params": {
    "lines": [
      "{\"id\":\"01JQ...\",\"hlc_timestamp\":\"...\", ...}"
    ]
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `lines` | string[] | yes | NDJSON lines from `sync_export` |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sync_imported",
    "imported": 38,
    "conflicts": 2,
    "skipped": 2
  }
}
```

### sync_conflicts

List unresolved sync conflicts.

**Role:** Viewer

**Request:**

```json
{"method": "sync_conflicts"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sync_conflict_list",
    "conflicts": [
      {
        "local_id": "01JQ...",
        "remote_id": "01JR...",
        "local_title": "Use PostgreSQL",
        "remote_title": "Use MySQL",
        "conflict_type": "content_divergence"
      }
    ]
  }
}
```

### sync_resolve

Resolve a sync conflict by keeping the specified memory.

**Role:** Member

**Request:**

```json
{
  "method": "sync_resolve",
  "params": {
    "keep_id": "01JQ..."
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `keep_id` | string | yes | ID of the memory to keep |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "sync_resolved",
    "id": "01JQ...",
    "resolved": true
  }
}
```

---

## Notifications

The notification engine surfaces system events, warnings, and confirmation requests.

### list_notifications

List notifications with optional filters.

**Role:** Viewer

**Request:**

```json
{
  "method": "list_notifications",
  "params": {
    "status": "pending",
    "category": "security",
    "limit": 20
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `status` | string | no | Filter by status: `pending`, `acknowledged`, `dismissed` |
| `category` | string | no | Filter by category |
| `limit` | integer | no | Maximum results |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "notification_list",
    "notifications": [...],
    "count": 3
  }
}
```

### ack_notification

Acknowledge a notification (mark as read).

**Role:** Member

**Request:**

```json
{
  "method": "ack_notification",
  "params": {
    "id": "01JQNOTIF123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Notification ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "notification_acked",
    "id": "01JQNOTIF123"
  }
}
```

### dismiss_notification

Dismiss a notification (hide permanently).

**Role:** Member

**Request:**

```json
{
  "method": "dismiss_notification",
  "params": {
    "id": "01JQNOTIF123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Notification ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "notification_dismissed",
    "id": "01JQNOTIF123"
  }
}
```

### act_on_notification

Act on a confirmation notification (approve or reject).

**Role:** Member

**Request:**

```json
{
  "method": "act_on_notification",
  "params": {
    "id": "01JQNOTIF123",
    "approved": true
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Notification ID |
| `approved` | boolean | yes | `true` to approve, `false` to reject |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "notification_acted",
    "id": "01JQNOTIF123",
    "result": "approved"
  }
}
```

---

## Diagnostics

### get_diagnostics

Get cached diagnostics for a file (from LSP or proactive checks).

**Role:** Viewer

**Request:**

```json
{
  "method": "get_diagnostics",
  "params": {
    "file": "src/auth.rs"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file` | string | yes | File path |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "diagnostic_list",
    "diagnostics": [
      {
        "file": "src/auth.rs",
        "line": 42,
        "severity": "warning",
        "message": "Unused variable `old_key`"
      }
    ],
    "count": 1
  }
}
```

### lsp_status

Query which language servers are available.

**Role:** Viewer

**Request:**

```json
{"method": "lsp_status"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "lsp_status",
    "servers": [
      {"language": "rust", "name": "rust-analyzer", "running": true}
    ]
  }
}
```

### inspect

Phase 2A-4d.2 observability API. Queries `kpi_events` or the per-layer `GaugeSnapshot` through one shape-parameterized RPC. `shape=row_count` is served from the atomic gauge snapshot; all other shapes aggregate `kpi_events` rows over the `window`.

**Role:** Viewer

**Request:**

```json
{
  "method": "inspect",
  "params": {
    "shape": "latency",
    "window": "1h",
    "filter": {
      "layer": null,
      "phase": null,
      "event_type": "phase_completed",
      "project": null
    },
    "group_by": "phase"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `shape` | string | yes | One of `row_count`, `latency`, `error_rate`, `throughput`, `phase_run_summary` |
| `window` | string | no | humantime grammar (`15m`, `1h30m`, `7d`…). Default `1h`. Ceiling 7 days. |
| `filter.layer` | string | no | Manas layer filter (applies to `shape=row_count`) |
| `filter.phase` | string | no | Phase name filter |
| `filter.event_type` | string | no | kpi_event type filter |
| `filter.project` | string | no | Project scope filter |
| `group_by` | string | no | One of `phase`, `event_type`, `project`, `run_id` (validity per shape; see spec) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "inspect",
    "shape": "latency",
    "window": "1h",
    "window_secs": 3600,
    "generated_at_secs": 1745500000,
    "effective_filter": {
      "layer": null,
      "phase": null,
      "event_type": "phase_completed",
      "project": null
    },
    "effective_group_by": "phase",
    "stale": false,
    "truncated": false,
    "data": {
      "kind": "latency",
      "rows": [
        {
          "group_key": "phase_3_semantic_dedup",
          "count": 420,
          "p50_ms": 12.5,
          "p95_ms": 48.0,
          "p99_ms": 102.3,
          "mean_ms": 17.9,
          "truncated_samples": 0
        }
      ]
    }
  }
}
```

`stale=true` indicates `shape=row_count` was served from a stale or empty snapshot. `truncated=true` indicates at least one group hit the per-group sample cap during percentile calculation. See `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier2-design.md` §2 for the full validity matrix.

**Percentile convention.** `p50_ms` / `p95_ms` / `p99_ms` use **ceiling-rank percentiles** on the sorted samples — `sorted[ceil(p * n) - 1]`, clamped to `[0, n-1]`. This means percentiles are always concrete observed sample values (no interpolation between adjacent samples), which keeps the units of the response identical to the underlying `latency_ms` values from `kpi_events`.

Edge cases worth knowing:

- For `n = 1` every percentile (`p50` / `p95` / `p99`) returns the single sample.
- For `n = 2`, `p50` returns `sorted[0]` (the **minimum**, not the average) — `ceil(0.5 * 2) - 1 = 0`. With more samples, `p50` gravitates to the median as expected.
- For empty groups, percentiles return `0.0` and `count = 0`; this is reachable only when filters reduce the group to zero rows after the global cap.

Choose this convention vs interpolated percentiles (PERCENTILE_CONT) deliberately — the consumers of `/inspect` are operator dashboards that benefit from "this is a real observed sample" semantics. If you need interpolated percentiles, compute them client-side from the underlying events.

### get_stats

Get aggregated metrics for a time period.

**Role:** Viewer

**Request:**

```json
{
  "method": "get_stats",
  "params": {
    "hours": 24
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `hours` | integer | no | Time period in hours (default varies) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "stats",
    "period_hours": 24,
    "extractions": 15,
    "extraction_errors": 0,
    "tokens_in": 45000,
    "tokens_out": 12000,
    "total_cost_usd": 0.03,
    "avg_latency_ms": 850,
    "memories_created": 8
  }
}
```

### get_graph_data

Get graph data for visualization (nodes = memories, edges = relationships).

**Role:** Viewer

**Request:**

```json
{
  "method": "get_graph_data",
  "params": {
    "layer": "experience",
    "limit": 100
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `layer` | string | no | Filter by Manas layer |
| `limit` | integer | no | Max nodes per layer (default 50) |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "graph_data",
    "nodes": [
      {"id": "01JQ...", "label": "Use PostgreSQL", "layer": "experience", "type": "decision"}
    ],
    "edges": [
      {"source": "01JQ...", "target": "01JR...", "type": "relates_to"}
    ],
    "total_nodes": 70,
    "total_edges": 156
  }
}
```

---

## A2A Permissions

### grant_permission

Grant permission for inter-session messaging between agents.

**Role:** Admin

**Request:**

```json
{
  "method": "grant_permission",
  "params": {
    "from_agent": "forge-generator",
    "to_agent": "forge-evaluator",
    "from_project": "myapp",
    "to_project": "myapp"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `from_agent` | string | yes | Sending agent |
| `to_agent` | string | yes | Receiving agent |
| `from_project` | string | no | Restrict to project |
| `to_project` | string | no | Restrict to project |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "permission_granted",
    "id": "01JQPERM123"
  }
}
```

### revoke_permission

Revoke an A2A permission.

**Role:** Admin

**Request:**

```json
{
  "method": "revoke_permission",
  "params": {
    "id": "01JQPERM123"
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Permission ID |

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "permission_revoked",
    "id": "01JQPERM123",
    "found": true
  }
}
```

### list_permissions

List all A2A permissions.

**Role:** Viewer

**Request:**

```json
{"method": "list_permissions"}
```

**Response:**

```json
{
  "status": "ok",
  "data": {
    "kind": "permission_list",
    "permissions": [
      {
        "id": "01JQPERM123",
        "from_agent": "forge-generator",
        "to_agent": "forge-evaluator",
        "allowed": true
      }
    ],
    "count": 1
  }
}
```

---

## Error Handling

### Protocol Errors

When the JSON is valid but the operation fails (e.g., memory not found, invalid parameters), the HTTP status is 200 and the error is in the JSON body:

```json
{
  "status": "error",
  "message": "memory not found: 01JQXYZ123ABC"
}
```

### Infrastructure Errors

When the daemon itself is unhealthy, the HTTP status reflects the failure:

| HTTP Code | JSON Body | Cause |
|-----------|-----------|-------|
| 503 | `{"status":"error","message":"database unavailable"}` | Cannot open SQLite connection |
| 503 | `{"status":"error","message":"writer unavailable"}` | Writer actor channel closed |
| 504 | `{"status":"error","message":"write request timed out"}` | Write took longer than 30s |

### Auth Errors

| HTTP Code | JSON Body | Cause |
|-----------|-----------|-------|
| 401 | `{"error":"missing Authorization header"}` | No Bearer token provided |
| 401 | `{"error":"invalid token"}` | JWT validation failed |
| 403 | `{"error":"insufficient permissions"}` | RBAC check denied the operation |

---

## Rate Limiting

Forge does not implement built-in rate limiting. For production deployments, apply rate limiting at the reverse proxy or Ingress level.

## SDKs and Clients

The primary client is `forge-next`, the Rust CLI. For programmatic access, any HTTP client that can POST JSON and parse JSON responses works. The protocol is intentionally simple -- no WebSocket, no streaming (except `subscribe` over Unix socket), no pagination tokens.

```bash
# curl example
curl -X POST http://localhost:8420/api \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $JWT" \
  -d '{"method":"recall","params":{"query":"auth decisions","limit":5}}'
```
