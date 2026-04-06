# Agent Development

This guide covers building custom AI agents that connect to Forge for persistent memory, identity, and inter-agent communication. Forge provides the cognitive infrastructure; your agent provides the reasoning.

## What Is an Agent?

In Forge, an agent is any process that:

1. Registers a session with the daemon
2. Uses memory operations (remember, recall, compile-context) during its work
3. Ends the session when done

An agent can be a Claude Code instance, a custom Python script, a Rust binary, or a containerized worker in Kubernetes. The daemon does not care what the agent is -- it cares about the session lifecycle and memory operations.

## Agent Lifecycle

Every agent follows the same four-phase pattern:

```
Register Session  -->  Get Context  -->  Do Work  -->  End Session
```

### Phase 1: Register

```bash
forge-next register-session \
  --id "my-agent-run-001" \
  --agent "code-reviewer" \
  --project "my-web-app" \
  --cwd "/home/dev/projects/my-web-app"
```

This tells the daemon that a new agent session is active. The `--id` must be unique. The `--agent` field identifies the agent type and is used for identity facet lookup.

### Phase 2: Get Context

```bash
forge-next compile-context --agent code-reviewer --project my-web-app
```

This returns a structured context block assembled from all 8 memory layers, filtered by the agent's identity facets and the project scope. Feed this into your agent's system prompt or context window.

### Phase 3: Do Work

During its work, the agent reads and writes memories:

```bash
# Recall relevant knowledge
forge-next recall "authentication patterns" --project my-web-app --limit 10

# Store findings
forge-next remember \
  --type lesson \
  --title "JWT refresh token rotation missing" \
  --content "The auth module does not implement refresh token rotation, which is a security risk for long-lived sessions."

# Send messages to other agents (FISP protocol)
forge-next send \
  --to "build-agent-042" \
  --kind notification \
  --topic "review_finding" \
  --text "Found missing input validation in /api/users endpoint."
```

### Phase 4: End Session

```bash
forge-next end-session --id "my-agent-run-001"
```

This signals the daemon to finalize the session, persist any pending state, and free resources.

## Transport Options

Agents connect to the Forge daemon over one of three transports:

| Transport | Use Case | Connection |
|-----------|----------|------------|
| Unix socket | Local agents on the same machine | `~/.forge/forge.sock` (default, auto-detected) |
| HTTP | Remote agents, cross-machine | `--endpoint https://forge.company.com` |
| gRPC | In-cluster agents, high throughput | `--endpoint grpc://forge.forge.svc:8421` |

### Local (Unix socket)

No configuration needed. `forge-next` auto-detects the socket:

```bash
forge-next health
```

### Remote (HTTP)

Point to the Forge server and provide a JWT token:

```bash
export FORGE_ENDPOINT="https://forge.company.com"
export FORGE_TOKEN="eyJhbGciOiJSUzI1NiIs..."

forge-next --endpoint "$FORGE_ENDPOINT" --token "$FORGE_TOKEN" health
```

### In-cluster (gRPC)

For agents running as pods in the same Kubernetes cluster:

```bash
forge-next --endpoint grpc://forge.forge.svc.cluster.local:8421 health
```

## Memory Operations

### Remember

Store structured knowledge:

```bash
forge-next remember \
  --type decision \
  --title "Use connection pooling" \
  --content "Database connections should use pgbouncer with pool_mode=transaction for serverless workloads."
```

Memory types: `decision`, `lesson`, `pattern`, `fact`, `entity`, `skill`.

### Recall

Search across all memory layers:

```bash
forge-next recall "connection pooling"
```

Filter by project, type, layer, or limit:

```bash
forge-next recall "deployment" --project my-web-app --type decision --layer domain_dna --limit 5
```

### Compile Context

Generate a full context payload for an agent:

```bash
forge-next compile-context --agent code-reviewer --project my-web-app
```

The compiled context includes:

- Relevant decisions and lessons (from experience layer)
- Domain DNA (project-specific architectural knowledge)
- Identity facets (agent personality and constraints)
- Active perceptions (recent file changes, git state)
- Skills (agent-specific capabilities)

### Forget

Remove a specific memory by ID:

```bash
forge-next forget <memory-id>
```

## Inter-Agent Communication (FISP)

FISP (Forge Inter-Session Protocol) enables agents to communicate during active sessions.

### Send a message

```bash
forge-next send \
  --to "build-agent-042" \
  --kind notification \
  --topic "schema_changed" \
  --text "Added new column 'expires_at' to the sessions table."
```

### Broadcast to all agents in a project

```bash
forge-next send \
  --to "*" \
  --kind notification \
  --topic "schema_changed" \
  --text "Migration 005 added expires_at column to sessions table." \
  --project my-web-app
```

### Read messages

```bash
forge-next messages --session "my-agent-run-001" --status pending
```

### Acknowledge messages

```bash
forge-next ack msg-id-1 msg-id-2
```

## Agent Identity

Each agent type can have identity facets that shape how the daemon compiles context and filters memories.

### Set identity for your agent type

```bash
forge-next identity set \
  --agent code-reviewer \
  --facet review_focus \
  --description "Focuses on security vulnerabilities, error handling, and edge cases. Ignores style/formatting issues."
```

```bash
forge-next identity set \
  --agent code-reviewer \
  --facet communication_style \
  --description "Reports findings as structured JSON with severity, file, line, and description fields."
```

### List identity facets

```bash
forge-next identity list --agent code-reviewer
```

### Remove a facet

```bash
forge-next identity remove <facet-id>
```

## Example: Building a Code Reviewer Agent

Here is a complete example of a code reviewer agent implemented as a shell script. In practice, you would use Python or Rust and call an LLM for the actual review logic.

```bash
#!/usr/bin/env bash
set -euo pipefail

# --- Configuration ---
AGENT_TYPE="code-reviewer"
SESSION_ID="review-$(date +%s)"
PROJECT="${1:?Usage: review-agent.sh <project> <diff-file>}"
DIFF_FILE="${2:?Usage: review-agent.sh <project> <diff-file>}"

# --- Phase 1: Register ---
forge-next register-session \
  --id "$SESSION_ID" \
  --agent "$AGENT_TYPE" \
  --project "$PROJECT" \
  --cwd "$(pwd)"

echo "Session registered: $SESSION_ID"

# --- Phase 2: Get Context ---
CONTEXT=$(forge-next compile-context --agent "$AGENT_TYPE" --project "$PROJECT")

echo "Context compiled ($(echo "$CONTEXT" | wc -c) bytes)"

# --- Phase 3: Do Work ---

# Recall past review findings for this project
PAST_FINDINGS=$(forge-next recall "review findings" --project "$PROJECT" --type lesson --limit 5)

# Read the diff
DIFF=$(cat "$DIFF_FILE")

# Here you would send $CONTEXT, $PAST_FINDINGS, and $DIFF to an LLM
# and get back structured review findings. For this example, we simulate it:
echo "Reviewing diff..."

# Store a finding as a memory
forge-next remember \
  --type lesson \
  --title "Missing null check in user handler" \
  --content "The PATCH /users/:id endpoint does not validate that the request body is non-null before destructuring."

# Notify the build agent about the finding
forge-next send \
  --to "*" \
  --kind notification \
  --topic "review_complete" \
  --text "Code review found 1 issue. See memories for details." \
  --project "$PROJECT"

# --- Phase 4: End Session ---
forge-next end-session --id "$SESSION_ID"

echo "Review complete. Session ended."
```

Run it:

```bash
chmod +x review-agent.sh
git diff HEAD~1 > /tmp/latest.diff
./review-agent.sh my-web-app /tmp/latest.diff
```

## Deploying Agent Workers on Kubernetes

For agents that run continuously or on-demand in a cluster, deploy them as Kubernetes Jobs or Deployments.

### Agent worker pod manifest

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: code-reviewer-job
spec:
  template:
    spec:
      containers:
        - name: code-reviewer
          image: my-registry/code-reviewer:latest
          env:
            - name: FORGE_ENDPOINT
              value: "http://forge.forge.svc:8420"
            - name: FORGE_TOKEN
              valueFrom:
                secretKeyRef:
                  name: forge-agent-token
                  key: token
            - name: PROJECT
              value: "my-web-app"
            - name: DIFF_REF
              value: "HEAD~1"
          command:
            - /bin/sh
            - -c
            - |
              SESSION_ID="review-$(date +%s)"
              forge-next --endpoint "$FORGE_ENDPOINT" --token "$FORGE_TOKEN" \
                register-session --id "$SESSION_ID" --agent code-reviewer --project "$PROJECT"

              CONTEXT=$(forge-next --endpoint "$FORGE_ENDPOINT" --token "$FORGE_TOKEN" \
                compile-context --agent code-reviewer --project "$PROJECT")

              # ... run review logic ...

              forge-next --endpoint "$FORGE_ENDPOINT" --token "$FORGE_TOKEN" \
                end-session --id "$SESSION_ID"
      restartPolicy: Never
  backoffLimit: 2
```

### Long-running agent Deployment

For agents that listen for events and respond continuously:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: watcher-agent
spec:
  replicas: 1
  selector:
    matchLabels:
      app: watcher-agent
  template:
    spec:
      containers:
        - name: watcher
          image: my-registry/watcher-agent:latest
          env:
            - name: FORGE_ENDPOINT
              value: "http://forge.forge.svc:8420"
            - name: FORGE_TOKEN
              valueFrom:
                secretKeyRef:
                  name: forge-agent-token
                  key: token
```

## Memory Sync Across Machines

Agents running on different machines can synchronize memories:

### Export memories

```bash
forge-next sync-export --project my-web-app --since "2026-04-01T00:00:00Z"
```

### Push to a remote Forge instance

```bash
forge-next sync-push remote-host --project my-web-app
```

### Pull from a remote Forge instance

```bash
forge-next sync-pull remote-host --project my-web-app
```

### Resolve sync conflicts

```bash
forge-next sync-conflicts
forge-next sync-resolve <conflict-id>
```

## Session Management

### List active sessions

```bash
forge-next sessions
```

### List all sessions (including ended)

```bash
forge-next sessions --all
```

### Clean up stale sessions

```bash
forge-next cleanup-sessions --prefix "review-"
```

## Guardrails

Before performing destructive operations, agents can check guardrails:

### Check file permissions

```bash
forge-next check --file src/auth/secrets.rs --action edit
```

### Check blast radius

```bash
forge-next blast-radius --file src/models/user.rs
```

This queries the knowledge graph to find all files and symbols that depend on the target file, helping agents understand the impact of changes.

## Perceptions

View recent perceptions (file changes, git events, diagnostics) recorded by the daemon:

```bash
forge-next perceptions --project my-web-app --limit 20
```

Perceptions are the raw input layer (Chitta) that the daemon processes into memories.

## Next Steps

- [Getting Started](getting-started.md) -- install Forge and store your first memory
- [Cloud Deployment](cloud-deployment.md) -- deploy Forge to Kubernetes for team-wide shared memory
