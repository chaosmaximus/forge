# Forge dogfood findings — P3-3.8 — 2026-04-26

**Status:** P3-3.8 closed at HEAD `6118ec2` (post P3-3.7).
**Daemon under test:** v0.6.0-rc.3 (commit `50a2b95`, rebuilt for P3-3.6 W10/W11).
**Scope:** every user-visible feature surface of forge — health/doctor, identity,
agent teams, FISP messaging, recall, compile-context, perceptions, contradictions,
code search, blast-radius, indexer.

## Headline

The cognitive infrastructure (memory layers, identity, perception, contradiction
detection, healing) **works** end-to-end at v0.6.0-rc.3. The agent-team
intercommunication primitives **work in principle but have rough edges** that
would surprise a first-time user. The indexer + project-scoping surfaces have
**concrete bugs** that should be triaged before v0.6.0 GA.

23 distinct findings captured below. No BLOCKERs. **3 HIGH, 7 MED, 11 LOW, 2
WORKS-AS-EXPECTED.**

## Findings

### HIGH

**F4 — Daemon auto-spawn doesn't propagate `LD_LIBRARY_PATH` for ONNX**
- `forge-next <cmd>` auto-starts the daemon when it isn't running, but the
  spawned `forge-daemon` process inherits a shell env where
  `LD_LIBRARY_PATH` doesn't include the bundled ONNX runtime, so it dies
  immediately with `error while loading shared libraries: libonnxruntime.so.1`.
- Reproducer: `forge-next restart` followed by any command — daemon log
  shows the libonnxruntime error.
- **Suggested fix:** the daemon binary should use `rpath` to embed the
  ONNX lib directory, OR the auto-spawn call site should construct the
  env with `LD_LIBRARY_PATH` set (the same way `cargo`'s
  `.cargo/config.toml` does for builds).

**F11 — `forge-next send` ignores caller identity, all messages get `from=api`**
- Sending a FISP message from any CLI invocation defaults `from_session`
  to the literal string `"api"`, regardless of `FORGE_SESSION_ID` env or
  any other context.
- For real planner→generator→evaluator coordination this is fatal:
  every agent's outbound message looks like it came from a daemon-internal
  identity, so the receiver can't tell who sent it.
- **Suggested fix:** add an explicit `--from <session_id>` flag on `send`
  (and `respond`), OR make the CLI auto-detect the calling agent's
  session via a hook-set env var (`FORGE_SESSION_ID` or
  `CLAUDE_SESSION_ID`).

**F23 — Synchronous `force-index` blocks the daemon writer loop**
- `forge-next force-index` returned `daemon response timed out (30s)`.
  Subsequent `team stop` and `cleanup-sessions` calls also timed out at
  30s, which strongly suggests the indexer holds the writer lock for
  the whole run.
- Even if the index completes eventually, the daemon is effectively
  unavailable for all writes during the run — a real DoS risk for any
  multi-tenant or long-running deployment.
- **Suggested fix:** `force-index` should either return immediately and
  run in a background task (the way `consolidate` does), or take a
  read-only snapshot before re-indexing so the writer remains responsive.

### MEDIUM

**F1 — Daemon version in CLI doctor lags the binary on auto-restart.**
After rebuild, `forge-next health` reports the prior version (v0.5.0 at
commit `d9fda72`) until an explicit `forge-next restart` cycles the
daemon. The auto-start path uses whichever binary is on PATH, but does
NOT replace a running daemon's process.

**F2 — `[WARN] hook: plugin hooks.json not found`** in `doctor` even when
running in-tree from the repo. The path the doctor probes is
`~/.claude/plugins/forge/hooks.json` — for in-repo dogfood the
`hooks/hooks.json` file lives at the repo root and isn't symlinked.
Either auto-symlink on `forge-next init`, or downgrade the warning when
running outside a plugin install.

**F3 — Cold-start socket-bind timeout 3s.** The CLI gives the daemon
3s to bind its socket; the cold-start ONNX init takes longer than that
on this machine (~5–7s depending on cache state). First command after
restart sometimes fails with
`error: forge-daemon started but socket not available after 3s`.
Bumping the timeout to 10s (or using exponential backoff with up to
30s) would eliminate the spurious first-call failure.

**F9 — Team members show role=`?` instead of the spawning template name.**
`forge-next team members --name <T>` prints lines like
`01KQ47JZ680JSG2TXX9SXR58CC: ? [idle]`. The `?` appears to be a default
when the agent record lacks a role/template field — but `team run
--templates product-manager,backend-dev,qa` should have set this. Trace
back: probably the agent_template→agent.role wiring.

**F15/F17 — Recall is not strictly project-scoped even with `--project forge`.**
Top results for `forge-next recall "polish wave drift fixtures" --project
forge` included a Hive Finance (`dashboard`-tagged) feature engineering
audit. Either the project filter is being applied as a soft boost
rather than a hard filter, OR the memory's project field was left
empty / mis-tagged at extraction time and the recall path treats empty
project as a wildcard. forge-isolation D1 cross_project_precision
explicitly tests this property and scores 1.0 in the bench — but the
production recall path apparently has a different code path.

**F20 — Indexer doesn't have recently-modified files in its symbol graph.**
`forge-next find-symbol audit_dedup` returns "no symbols found" despite
the function existing in `crates/daemon/src/bench/forge_consolidation.rs`.
Same for `code-search "drift_fixtures"`. Indexer is either lagging or
the file is excluded from indexing.

**F22 — `blast-radius --file <path>` reports
`File '<path>' not found in the code graph. It may not have been indexed yet`**
even for a file present in HEAD for weeks.

### LOW

**F5 — `identity show` command doesn't exist** (the actual subcommand is
`identity list`). Discoverability nit; the `show` verb is more
conventional for "view current state" semantics.

**F6 — `team create` + `team run` aren't compositional.** `team create
--name X` makes a row but doesn't spawn agents. `team run --name X
--templates ...` requires X to NOT exist (UNIQUE constraint failure if
it does). So the natural "create then populate later" workflow is
broken. The CLI should either reject `team create` for agent-type
teams (make them only via `team run`), or make `team run` reuse an
existing team-by-name.

**F7 — `team stop` reports `0 agent(s) retired`** for a team that was
created but never `run`. Correct (no agents to retire), but the wording
implies the stop did something when it actually was a no-op. Add a
`(team had no spawned agents)` annotation.

**F8 — Templates spawn agents with `project: (none)`.** `forge-next
sessions` shows the 3 spawned agents with no project. If the orchestrator
intends to scope work to a project, the spawn must take a `--project`
flag. Today the operator has to call `register-session` separately.

**F10 — `message-read` requires `--id` flag, not positional.** Other
commands that take a single ID accept it positionally (`ack <id>`,
`forget <id>`). Inconsistent surface.

**F12/F14 — `message-read --id <full-ULID>` returns "message not found"
even though `messages --session ...` lists the message.** Either the
`message-read` lookup uses a different ID column or the truncation
displayed in `messages` strips characters the lookup needs. The
storage-vs-display ID mismatch is at minimum a UX bug.

**F13 — `FORGE_SESSION_ID` env var doesn't propagate to message
`from_session`.** Setting `FORGE_SESSION_ID=01KQ...` then calling
`forge-next send ...` still produces a row with `from_session='api'`.
Tracked separately from F11 since this is the natural escape hatch a
user would try first.

**F16 — Identity facets cross-pollinate across projects.** 41 facets
are visible in `identity list` for the `claude-code` agent — including
"Working on Hive Finance credit risk pipeline", "Building Hive Finance
platform with K8s and OTLP", "Manages a credit risk ML platform on
GKE" — none of which are relevant to the forge repo. Identity should
probably be per-(agent, project) not per-agent.

**F18 — 4 contradictions detected on the live database**, 2 unresolved.
Looking at the unresolved pair:
`A: "Session 17: Full prod deployment + backfill fix" (neutral) vs B:
"Session 16: Full prod deployment pipeline complete" (neutral)` —
these aren't actually contradictory; both describe sequential
sessions completing. The contradiction detector is firing false
positives on chronological session-summary memories.

**F19 — `blast-radius <path>` (positional) errors with
`unexpected argument` instead of accepting it as `--file`.** Mirror of F10.

**F21 — `force-index` doesn't surface the timeout to the user with a
useful message.** It returns `daemon response timed out (30s)` — but
it's not clear whether the indexer is still running, has crashed, or
has succeeded silently. A `--background` flag with progress polling
would resolve this.

### WORKS-AS-EXPECTED

**Identity (Ahankara) — 41 facets surfaced cleanly.** The
`<forge-context>` XML output of `compile-context --session <id>`
renders all facets, decisions, lessons, skills, active-protocols,
deferred items, tools, agent templates, and project conventions in a
form ready for LLM consumption. ~6KB context payload for a single
session, well-structured.

**Healing system surfaces work.** `manas-health` cleanly reports all
8 layers' counts, identity facet count (41), and disposition traits.
Auto-extraction is running (1593 embeddings, 271 memories, 50784
edges).

## Forge-eval summary numbers

| Layer | Count | Verdict |
|-------|------:|---------|
| Layer 1 Platform   | 5     | OK |
| Layer 2 Tool       | 52    | OK |
| Layer 3 Skill      | 16    | OK |
| Layer 4 Domain DNA | 36    | OK |
| Layer 5 Experience | 278   | OK |
| Layer 6 Perception | 20    | OK (auto-consume working) |
| Layer 7 Declared   | 7     | OK |
| Layer 8 Latent     | 1593  | OK |
| Identity facets    | 41    | OK (concern: cross-project pollution) |
| Active sessions    | 14    | (some stale; cleanup-sessions broken — see F23) |
| Workers            | 8     | OK |
| Contradictions     | 4 (2 resolved, 2 false-positive) | YELLOW (see F18) |

## Recommendation for v0.6.0 release

**Ship-blocker analysis:** none of the findings above are ship-blockers
in the strict sense — the daemon serves recall, the cognitive layers
populate, identity persists, FISP messaging round-trips. But the agent-
team primitives (F11, F13) are too rough for users to dogfood the
"planner→generator→evaluator" pattern from the README without first
patching the CLI to forward caller identity.

**Recommended pre-GA punch-list (3 fixes, ~2-3 hours):**

1. **Fix F4** (LD_LIBRARY_PATH on auto-spawn) — blocking for any user
   who installs via the binary release without a wrapper script.
2. **Fix F11/F13** (`--from <session_id>` flag on `send`/`respond`) —
   makes agent-team intercommunication actually work.
3. **Fix F23** (force-index async) — current behavior can wedge the
   daemon for 30s+ during operator maintenance.

The rest can land in v0.6.1+. The cross-project recall scoping (F15/F17)
and identity-pollution (F16) are interesting longer-term investigations
but don't gate the release.

## References

* Plan: [`../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.8)
* Daemon log captured at `/tmp/forge_daemon_dogfood.log`.
* Team spawned: `polish-wave-validators-v2` (3 agents via templates
  `product-manager,backend-dev,qa`, topology=chain).
* FISP messages exchanged: 2 (planner→generator request, generator→planner response).
