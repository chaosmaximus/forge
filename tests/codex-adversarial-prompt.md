# Forge Plugin -- Comprehensive Adversarial Review

> This prompt is designed to be passed to `/codex:adversarial-review`.
> It instructs Codex to perform a multi-dimensional adversarial analysis of the
> Forge plugin at the current repository.

---

## Instructions for Codex

You are performing a comprehensive adversarial review of the **Forge** Claude Code plugin (v0.1.0). Forge is a production-grade agent team orchestrator with two modes (greenfield/existing codebase), three agents (planner, generator, evaluator), seven skills, six hook scripts, bundled code intelligence, and cross-model Codex adversarial review.

Read every file in the plugin. Do NOT skip any. Then systematically work through all seven review dimensions below. For every finding, cite the **exact file path and line number** where the issue exists. Rate each finding as CRITICAL / HIGH / MEDIUM / LOW.

---

## Dimension A: Architecture Review

Examine the overall plugin structure and inter-component relationships.

### A1. Structural Soundness

1. Read `.claude-plugin/plugin.json`. Verify all declared paths exist:
   - Do all `agents/*.md` files exist?
   - Does the `skills/` directory contain exactly the skills referenced elsewhere?
   - Does `hooks/hooks.json` exist and is it valid JSON?
   - Does the `mcpServers.forge-graph.command` binary path pattern resolve?

2. Check for orphan files -- files that exist but are never referenced by any manifest, skill, or agent.

3. Check for phantom references -- files or tools referenced in skills/agents that do not exist in the plugin.

### A2. Circular Dependencies Between Skills

1. Map the invocation chain:
   - `forge` (SKILL.md) routes to `forge-new` or `forge-feature`
   - `forge-new` invokes `forge-review` (Phase 8), then `forge-ship` (Phase 9)
   - `forge-feature` invokes `forge-review` (Phase 4), then `forge-ship` (Phase 5)
   - `forge-review` may invoke `forge-ship` or loop back to generators
   - `forge-ship` is terminal
   - `forge-handoff` is callable from any phase
   - `forge-setup` is standalone

2. Are there any circular paths? Can `forge-review` -> fix cycle -> `forge-review` run infinitely? What bounds it?

3. Does `forge-review` explicitly invoke `forge-ship`, or does it rely on the lead to do so? (KNOWN_ISSUES.md mentions this.)

### A3. Agent Role Overlap

1. Compare `forge-planner.md`, `forge-generator.md`, `forge-evaluator.md`:
   - Are tool permissions cleanly separated? (Planner: read-only; Generator: read+write; Evaluator: read-only)
   - Is there any tool that appears in more than one agent where it should not?
   - Are `disallowedTools` comprehensive? Could the planner write files indirectly (e.g., via Bash `echo >`)?

2. Can the generator agent accidentally perform evaluation tasks? Can the evaluator accidentally generate code?

3. The planner says "Never specifies implementation details" and the generator says "implements ONE task." Is there clear delineation in the prompts, or could the planner drift into specifying file paths?

### A4. MCP Tool Name Consistency

1. Collect every MCP tool name referenced across ALL files (agents, skills, scripts). Verify:
   - `mcp__forge_forge-graph__get_architecture` -- is this the correct naming convention?
   - `mcp__forge_forge-graph__search_graph`
   - `mcp__forge_forge-graph__trace_call_path`
   - `mcp__forge_forge-graph__detect_changes`
   - `mcp__forge_forge-graph__index_status`
   - `mcp__forge_forge-graph__index_repository`
   - `mcp__forge_forge-graph__get_code_snippet`
   - `mcp__plugin_serena_serena__find_symbol`
   - `mcp__plugin_serena_serena__find_referencing_symbols`
   - `mcp__plugin_serena_serena__get_symbols_overview`

2. Are any tool names used in skills but NOT listed in the agent `tools:` frontmatter?
   - Specifically: `forge-feature/SKILL.md` references `mcp__forge_forge-graph__index_status` and `mcp__forge_forge-graph__index_repository` -- are these in any agent's tool list?
   - `README.md` mentions `get_code_snippet` -- is this tool referenced anywhere in agents or skills?

3. Is the naming convention for plugin MCP tools (`mcp__forge_forge-graph__*`) correct for how Claude Code resolves plugin-bundled MCP servers?

---

## Dimension B: Security Review

### B1. Hook Script stdin Parsing

All hook scripts read JSON from stdin via `cat` piped to `jq`. Examine each script for:

1. **protect-sensitive-files.sh** (line 13):
   - `INPUT=$(cat)` then `echo "$INPUT" | jq -r ...`
   - Can malicious JSON input cause command injection? What if `tool_input.file_path` contains `$(command)` or backticks?
   - The `jq -r` output is used in `basename` and `case` -- is this safe?
   - What if the JSON is malformed? Does `jq` return empty string or error?
   - The script blocks `.env`, `.env.*`, `credentials*`, `secrets*`, `*.key`, `*.pem`, `*.tfstate`, lock files. Can this be bypassed with:
     - Path traversal: `../../.env`?
     - Symlinks pointing to protected files?
     - Case variations: `.ENV`, `.Env`?
     - Unicode homoglyphs in filenames?

2. **post-edit-format.sh** (line 7):
   - Same `INPUT=$(cat)` pattern. Same injection risk analysis.
   - The `FILE_PATH` is passed to `ruff format`, `npx eslint`, `npx prettier`, `rustfmt`, `gofmt`, `terraform fmt`.
   - Can a malicious file path cause command injection via these tools?
   - The `|| true` pattern suppresses all errors -- could this mask an attack?

3. **task-completed-gate.sh** (lines 7-10):
   - `INPUT=$(cat)` then `jq -r '.task_subject // empty'`
   - `TASK_SUBJECT` is exported as an environment variable. Is this safe?
   - Can a malicious `task_subject` value escape into other commands?
   - Line 64: `CHANGED_FILES=$(git diff --name-only ...)` -- can filenames with special characters cause issues in the `grep` pattern matching on line 70?

4. **session-start.sh** (line 9):
   - `cat > /dev/null` to drain stdin -- is this sufficient? What if stdin is very large?

5. **session-end-memory.sh** (line 7):
   - `cat > /dev/null 2>/dev/null || true` -- same analysis.
   - Line 11-14: Glob pattern `$HOME/.claude/plugins/cache/*/episodic-memory/*/cli/episodic-memory.js` -- could a malicious plugin cache entry cause arbitrary code execution via `node "$EM_CLI"`?

6. **teammate-idle-checkpoint.sh** (line 5):
   - `cat > /dev/null` to drain stdin. Uses `sed -i` on STATE.md.
   - Can the `date -Iseconds` output contain characters that break the sed expression?

### B2. Sensitive File Protection Completeness

1. The protect-sensitive-files.sh blocks: `.env`, `.env.*`, `credentials*`, `secrets*`, `*.key`, `*.pem`, `*.tfstate`, `*.tfstate.backup`, `poetry.lock`, `package-lock.json`, `yarn.lock`

2. What about:
   - `.env.production`, `.env.staging` (covered by `.env.*`?)
   - `id_rsa`, `id_ed25519` (SSH keys without `.key` extension)
   - `*.p12`, `*.pfx` (PKCS12 certificates)
   - `*.jks` (Java keystores)
   - `*.keystore`
   - `kubeconfig`, `kubectl.config`
   - `.npmrc` (may contain auth tokens)
   - `.pypirc` (may contain auth tokens)
   - `*.gpg` (GPG keys)
   - `.git-credentials`
   - `config/database.yml` (Rails database secrets)
   - `wp-config.php` (WordPress secrets)
   - `service-account.json`, `*-sa-key.json` (GCP service account keys)
   - `token.json` (OAuth tokens)
   - `*.sqlite`, `*.db` (databases that may contain sensitive data)
   - Terraform variable files: `*.auto.tfvars`, `terraform.tfvars`

3. The protection is basename-only. Does it catch `/deeply/nested/path/.env`? (It should, since `basename` extracts just the filename.)

4. The protection only runs on `PreToolUse` for `Edit|Write`. What about:
   - `Bash` tool running `cat .env` (read, not write)?
   - `Bash` tool running `cp .env backup.env` then editing `backup.env`?
   - `Read` tool reading `.env` contents?

### B3. Git Staging Safety in Handoff

1. `forge-handoff/SKILL.md` line 23-29 shows:
   ```
   git add -u
   git add '*.py' '*.ts' '*.js' '*.go' '*.rs' '*.tf' 2>/dev/null
   git add STATE.md HANDOFF.md 2>/dev/null
   ```
   - `git add -u` stages ALL tracked file changes. Could this include modified `.env` files if they were previously tracked?
   - The glob patterns `'*.py' '*.ts'` etc. -- do these recurse into subdirectories? (Yes, git interprets these as pathspecs.)
   - Could `git add '*.tf'` accidentally stage `secrets.tf` or `terraform.tfvars`?
   - The handoff says "Do NOT use git add -A" but `git add -u` still stages tracked files indiscriminately.

2. The handoff warns about public repos but only as a note. Should this be a HARD-GATE?

### B4. Install Script Security

1. `install-server.sh` downloads a binary from GitHub over HTTPS and makes it executable:
   - No checksum verification
   - No GPG signature verification
   - No pinned version (uses `latest`)
   - `curl -fsSL` follows redirects -- could a compromised DNS redirect to a malicious binary?
   - The binary gets execute permission and is then auto-launched by `session-start.sh`

2. `session-start.sh` auto-installs and auto-launches this binary. Is there a TOCTOU race between install and execution?

---

## Dimension C: Reliability Review

### C1. MCP Tool Failure Modes

For each MCP tool used across agents and skills, identify what happens when it fails:

1. `mcp__forge_forge-graph__get_architecture` -- used in planner and forge-feature. What if it returns an error? Does any skill have a fallback?
2. `mcp__forge_forge-graph__search_graph` -- used in planner, generator, evaluator, forge-feature. Fallback path?
3. `mcp__forge_forge-graph__trace_call_path` -- used in planner, evaluator, forge-feature. Fallback?
4. `mcp__forge_forge-graph__detect_changes` -- used in planner, evaluator, forge-feature. Fallback?
5. `mcp__forge_forge-graph__index_status` -- used in forge (router) and forge-feature. What if the MCP server is completely down?
6. `mcp__forge_forge-graph__index_repository` -- used in forge-feature. What if indexing fails mid-way?
7. `get_code_snippet` -- mentioned in README but never referenced in any agent or skill. Is it available?

8. Serena tools: `forge-feature/SKILL.md` has an explicit fallback section ("If Serena is not available"). Do all skills that use Serena have this fallback? Check the generator agent.
9. Stitch MCP: loaded unconditionally via `.mcp.json`. What if the user does not have npm? What if `npx` fails?

### C2. Binary Missing

1. `session-start.sh` auto-installs if missing. What if:
   - No internet connection?
   - GitHub is down?
   - The platform is unsupported (e.g., Windows WSL with non-standard uname)?
   - Disk space is full?
   - The `|| true` on line 16 silently swallows the failure -- does the session start without the graph server?

2. `forge-setup/SKILL.md` checks the binary path. What guidance does it give if the binary is missing?

### C3. Codex Plugin Missing

1. `forge` (router) checks for Codex and warns. Is this sufficient?
2. `forge-review` blocks on prod paths if Codex is missing. What about:
   - The evaluator `forge-evaluator.md` references Codex gate but cannot invoke it directly. How does the handoff work?
   - Is there a race condition where the evaluator says "run Codex" but the lead cannot because the plugin is missing?
3. What if Codex is installed but returns an error (API down, quota exceeded)?

### C4. Error Path Completeness

Walk through each script and identify unhandled error paths:

1. **task-completed-gate.sh**:
   - Line 64: `git merge-base HEAD "$BASE_BRANCH"` -- what if `$BASE_BRANCH` does not exist? The `|| echo HEAD~1` fallback -- what if HEAD~1 does not exist (initial commit)?
   - Line 70: `grep -q "^${pattern%/\*\*}/"` -- what if `$pattern` contains regex special characters?

2. **post-edit-format.sh**:
   - What if `npx` hangs indefinitely? The timeout is 30 seconds (from hooks.json) but `npx` may need to download packages.

3. **session-start.sh**:
   - What if `pwd` returns a path with spaces? Is `"$(pwd)"` properly quoted everywhere?

4. **protect-sensitive-files.sh**:
   - Line 7: If `jq` is not installed, exit 2 blocks ALL edits. Is this the right behavior? Should it warn and allow?

---

## Dimension D: Consistency Review

### D1. Cross-Reference Resolution

1. Every skill reference in every file -- do they all resolve?
   - `forge` -> `forge-new`, `forge-feature` (exist)
   - `forge-new` -> `forge-review`, `forge-ship` (exist)
   - `forge-feature` -> `forge-review`, `forge-ship` (exist)
   - `forge-review` -> Codex adversarial-review (external plugin)
   - `forge-ship` -> `superpowers:verification-before-completion` (external plugin)
   - `forge-handoff` (standalone, no outgoing skill refs)
   - `forge-setup` (standalone, no outgoing skill refs)

2. Template references:
   - `forge-new` references `${CLAUDE_PLUGIN_ROOT}/templates/PRD.md` -- exists
   - `forge-new` references `${CLAUDE_PLUGIN_ROOT}/templates/STATE.md` -- exists
   - `forge-feature` references `${CLAUDE_PLUGIN_ROOT}/templates/STATE.md` -- exists
   - `forge-generator` references `CONSTITUTION.md` in project root -- template exists but it is looked for in the working directory, not the plugin root

3. CSV references:
   - `forge-new` references `${CLAUDE_PLUGIN_ROOT}/data/project-types.csv` -- exists
   - `forge-new` references `${CLAUDE_PLUGIN_ROOT}/data/domain-complexity.csv` -- exists
   - `forge-planner` references both CSVs -- exists

4. Evaluation criteria references:
   - `forge-evaluator` references `${CLAUDE_PLUGIN_ROOT}/evaluation-criteria/` -- all four exist

### D2. MCP Tool Name Consistency

1. Compile the complete list of MCP tool names across all files. Check for:
   - Typos in tool names
   - Inconsistent naming (e.g., `index_status` vs `indexStatus`)
   - Tools named in README but not used anywhere (e.g., `get_code_snippet`)
   - Tools used in skills but not declared in agent frontmatter `tools:` field

### D3. Evaluator Criteria vs Rubric File Alignment

1. The evaluator agent references four rubric files. For each:
   - Does the evaluator's scoring template match the rubric's criteria?
   - Does the evaluator's pass threshold match the rubric's `Pass Threshold` section?
   - Does the evaluator correctly implement auto-fail rules from each rubric?
   - Code quality: "Any criterion = 1" -> auto FAIL
   - Security: "Input Validation or Auth <= 2" -> auto FAIL
   - Architecture: "Consistency < 3" -> auto FAIL
   - Infrastructure: "Security Posture < 3 OR Blast Radius < 3" -> auto FAIL
   - The evaluator says "No individual criterion below 3" -- but code-quality rubric says "Any criterion = 1" is auto-fail, NOT "below 3." Is there a mismatch?

2. Are rubric weights reflected in the evaluator's scoring instructions? The evaluator says `weighted_avg = sum(score * weight) / sum(weights)` but does not list the weights. It says "read the rubric files" -- is this sufficient for consistent scoring?

### D4. Phase Name Consistency

1. Collect all phase names used across skills and templates:
   - STATE.md template: `classify|discover|prd|design|plan|build|review|ship`
   - forge-new: classify (Phase 1), discover (Phase 2), prd (Phase 3), design (Phase 5), plan (Phase 6), build (Phase 7), review (Phase 8), ship (Phase 9)
   - forge-feature: explore (Phase 1), clarify (Phase 1b), plan (Phase 2), build (Phase 3), review (Phase 4), ship (Phase 5)
   - forge-handoff: `explore|classify|discover|prd|design|plan|build|review|ship`
   - The forge-feature uses `explore` and `clarify` phases that are NOT in the STATE.md template
   - Does forge-feature set the STATE.md phase to `explore`? The template only lists `classify|discover|prd|design|plan|build|review|ship`

2. Are phase transitions consistent?
   - forge-new Phase 4 is "User Approves PRD" but the phase name would be... what? There is no `approve_prd` phase name
   - forge-feature Phase 1b is "Clarify Requirements" -- where does this fit in the phase enum?

### D5. Prod Path Pattern Consistency

1. Default prod paths:
   - `task-completed-gate.sh`: `infrastructure/**,terraform/**,k8s/**,helm/**,production/**`
   - `forge-evaluator.md`: `infrastructure/**`, `terraform/**`, `k8s/**`, `helm/**`, `production/**`
   - `forge-review/SKILL.md`: references `CLAUDE_PLUGIN_OPTION_PROD_PATHS` but does not list defaults
   - Are these consistent everywhere?

2. KNOWN_ISSUES.md notes: "Default prod_paths won't match variant names (hive_production, prod, live)." Is this addressed?

3. How does the user-configured `prod_paths` from plugin.json `userConfig` propagate to:
   - The task-completed-gate.sh script (via `CLAUDE_PLUGIN_OPTION_PROD_PATHS` env var)?
   - The evaluator agent (KNOWN_ISSUES says "Custom prod_paths must be passed by lead to evaluator (no env var access)")?
   - The forge-review skill?

---

## Dimension E: Workflow Completeness

### E1. Greenfield Fintech API -- Full Walkthrough

Walk through every phase for building a fintech payment processing API:

1. **Route** (`/forge`): Empty directory -> greenfield mode detected. Prerequisites checked. Routes to forge-new.

2. **Classify** (forge-new Phase 1):
   - Matches `api_backend` in project-types.csv (signals: api, backend)
   - Matches `fintech` in domain-complexity.csv
   - Surfaces: KYC/AML, PCI-DSS, fraud prevention, financial regulations, data residency, audit trails
   - Creates STATE.md with mode=greenfield, phase=classify
   - **Question:** Does the skill actually SET `phase=classify` in STATE.md, or does it just say "set mode to greenfield and phase to classify"? Is there an explicit `Write STATE.md` instruction?

3. **Discover** (forge-new Phase 2):
   - key_questions from api_backend: endpoints, auth, rate limits, data model
   - Plus fintech-specific questions from domain concerns
   - **Question:** Are fintech-specific questions auto-injected, or does the planner need to formulate them from `key_concerns`?

4. **PRD** (forge-new Phase 3):
   - required_sections: executive_summary, success_criteria, functional_reqs, nfr_performance, nfr_security, api_design
   - skip_sections: user_journeys_visual, ui_design, accessibility
   - special_sections from fintech domain: regulatory_compliance, financial_security, audit_requirements
   - **Question:** Are `special_sections` appended to `required_sections`, or treated separately? The skill says "Include domain-specific sections from domain-complexity.csv `special_sections`" but does not say WHERE in the PRD template.

5. **Approve PRD** (forge-new Phase 4):
   - HARD-GATE: explicit approval required
   - "Start over" resets to Phase 1
   - **Question:** After "start over," is the STATE.md actually reset? Does the skill write to STATE.md?

6. **Visual Design** (forge-new Phase 5):
   - `ui_design` is in `skip_sections` for api_backend -> Phase 5 SKIPPED entirely
   - **Verify:** The Phase 5 logic correctly checks `skip_sections` first

7. **Build Plan** (forge-new Phase 6):
   - Extracts deliverables from PRD, groups into waves
   - HARD-GATE: user approval before build

8. **Build** (forge-new Phase 7):
   - Team spawned: generators + evaluator
   - Delegate mode instructions given
   - **Question:** The session guard tracks time via `date +%s`. How does the lead remember START_TIME across turns? Is it stored in STATE.md?

9. **Review** (forge-new Phase 8):
   - Invokes forge-review
   - forge-review runs evaluator, then Codex gate
   - Fintech API touches security -> security rubric applied with >= 4.0 threshold
   - API design + PCI -> hard Codex gate if prod paths involved
   - **Question:** A greenfield project has no `prod_paths` yet. How are prod paths determined for a NEW project?

10. **Ship** (forge-new Phase 9):
    - Final verification, PR creation, memory save
    - **Question:** For a brand new project, what is the "base branch"? Is `main` created automatically?

### E2. Existing Codebase Feature -- hive_production

Walk through adding an authentication feature to a large existing codebase called hive_production:

1. **Route** (`/forge`): Directory has source code -> existing codebase mode. Routes to forge-feature.

2. **Explore** (forge-feature Phase 1):
   - Checks graph index, indexes if needed
   - get_architecture, search_graph for auth area, trace_call_path
   - **Question:** What if the codebase is very large and indexing takes > 5 minutes? Is there a timeout?

3. **Clarify** (forge-feature Phase 1b):
   - Asks clarifying questions about the auth feature
   - **Question:** Is there an explicit state transition from `explore` to `clarify` in STATE.md? Or is Phase 1b invisible to state tracking?

4. **Plan** (forge-feature Phase 2):
   - Spawns forge-planner with mode=existing
   - Planner uses graph tools to produce wave plan
   - HARD-GATE: user approval
   - **Question:** The skill says "Spawn the forge-planner agent" but the skill itself is running as the lead. Does the lead spawn the planner as a teammate or subagent?

5. **Build** (forge-feature Phase 3):
   - Team execution
   - Auth changes touch security -> evaluator applies security rubric
   - If auth code is in `production/` -> hard Codex gate
   - **Question:** hive_production has "production" in its NAME, not its path. Does the glob `production/**` match the directory name `hive_production/`? (It should NOT, since `production/**` looks for a directory literally named `production/`.)

6. **Review** (forge-feature Phase 4):
   - Evaluator + Codex gate

7. **Ship** (forge-feature Phase 5):
   - PR creation, cleanup, memory save

### E3. Edge Cases

1. **Resume after crash:**
   - Session starts, finds STATE.md with `phase=build`, `mode=existing`
   - `forge` router reads STATE.md, detects in-progress session
   - Routes to forge-feature
   - **Question:** Does forge-feature know to skip explore and plan phases when resuming mid-build? Or does it start from Phase 1 again?
   - CRITICAL: STATE.md says phase=build, but forge-feature has no "resume from build" logic. It always starts at Phase 1. How is this handled?

2. **Partial wave success:**
   - Wave 1: 3 generators. Gen-1 DONE, Gen-2 DONE_WITH_CONCERNS, Gen-3 BLOCKED.
   - **Question:** Does the build phase handle mixed results? The skill says "On FAIL, relay findings back to generator." But what about DONE_WITH_CONCERNS vs BLOCKED? Are these treated differently?

3. **User says "start over" during build:**
   - forge-new Phase 4 handles "start over" (resets to Phase 1)
   - But what if the user says "start over" during Phase 7 (build)?
   - Are there worktrees to clean up? Active teammates to shut down?
   - There is NO "start over" handling for build phase. Only the handoff skill does cleanup.

4. **Evaluator fails evaluation 3 times:**
   - forge-feature has a circuit breaker (3 failures -> escalate to user)
   - forge-new does NOT have this circuit breaker. Is this intentional or an oversight?

5. **User runs forge-new in a directory with existing code:**
   - The router detects existing code and routes to forge-feature
   - But what if the user explicitly invokes `/forge:new` directly in a directory with existing code?
   - Skills can be invoked directly, bypassing the router

6. **Multiple forge sessions on same repo:**
   - STATE.md is a single file. Two sessions writing to it = corruption.
   - Is there any locking mechanism?

---

## Dimension F: Prompt Engineering Review

### F1. Will Claude Follow These Instructions?

1. **HARD-GATE enforcement:**
   - Forge uses `<HARD-GATE>` tags in multiple skills. These are NOT a Claude Code feature -- they are prompt engineering patterns.
   - How reliably does Claude respect these? Are they placed prominently enough?
   - The HARD-GATEs in forge-new:
     - Phase 4: "Do NOT proceed to design or build until the user has approved the PRD"
     - Phase 6: "Do NOT start building until the user has approved the wave plan"
     - Phase 7: "Do NOT merge any generator output without evaluator review passing"
   - Are these enforceable? Can the model rationalize around them?

2. **Rationalization Prevention Tables:**
   - Every skill and agent has a "Rationalization Prevention" table.
   - These tables pre-empt common model failure modes.
   - Are they effective? Common weaknesses:
     - Does Claude actually check these tables mid-generation?
     - Are the entries specific enough? "NO. Run tests." is clear. But does the model reason: "this is a special case not covered by the table"?
     - Are there missing entries for common failure modes?

3. **Agent role boundaries in practice:**
   - The planner says "Never specifies implementation details" -- but what if the user asks the planner for implementation details? Will the planner refuse?
   - The evaluator says "Trust nothing. Verify." -- but what if verification is slow (running tests takes 10 minutes)? Will the evaluator skip verification?
   - The generator says "Do NOT add features not in the task" -- but generators routinely add "helpful" extras. Is the prompt strong enough?

### F2. Description-Based Auto-Invocation

1. Each skill has a `description` in its YAML frontmatter. Claude uses these to decide when to auto-invoke skills.
   - `forge`: "Use when starting any development task, building features, or creating new projects -- before writing any code"
   - `forge-new`: "Use when building a new project from scratch in an empty or near-empty directory"
   - `forge-feature`: "Use when adding features, fixing bugs, or modifying code in an existing codebase with source files"
   - `forge-review`: "Use after forge-build completes all waves, before merging or shipping any code"
   - `forge-ship`: "Use when forge-review has passed and work is ready to integrate or create a PR"
   - `forge-handoff`: "Use when pausing work or before ending a session with in-progress Forge work"
   - `forge-setup`: "Use on first run in a new project directory, or when Forge prerequisites need checking"

2. **Overlap analysis:**
   - `forge` and `forge-feature` overlap: "building features" appears in forge, "adding features" in forge-feature. If the user says "add a login feature," which triggers?
   - `forge` and `forge-new` overlap: "creating new projects" in forge, "building a new project from scratch" in forge-new. If the user says "build a new API," which triggers?
   - The INTENDED flow is: `forge` always triggers first (it's the router). But if `forge-new` or `forge-feature` trigger directly, the router is bypassed.
   - Is the `forge` description broad enough to ALWAYS win over `forge-new` and `forge-feature`?

3. **When should forge NOT trigger?**
   - User asks to "write a test" -- should forge trigger? The description says "before writing any code."
   - User asks to "fix a typo in README" -- should forge trigger?
   - User asks to "review this PR" -- should forge trigger?

### F3. Proactive-but-User-Guided Principle

1. Both forge-new and forge-feature have a prominent "proactive-but-user-guided" section. Is this principle:
   - Repeated enough to survive context window compression?
   - Strong enough to override Claude's default helpful behavior?
   - In conflict with any other instruction in the skills?
   - Could the session guard's "Auto-checkpointing to STATE.md" at 120 minutes be seen as violating the "never acts autonomously" principle?

### F4. Missing Prompt Engineering Patterns

1. Are there common failure modes not addressed?
   - Claude apologizing excessively instead of making progress
   - Claude asking too many clarifying questions (discovery phase should be bounded)
   - Claude interpreting ambiguous instructions in the most literal way
   - Claude losing track of which phase it is in (especially in long sessions)
   - Claude confusing skill instructions with agent instructions when both are in context

---

## Dimension G: Production Readiness

### G1. Safety on Real Codebases

1. Can Forge cause data loss?
   - Generator runs in worktree isolation. But what about the lead?
   - The lead has Write and Edit tools. Nothing prevents the lead from modifying files directly.
   - `git add -u` in handoff stages all tracked changes. Could this include unintended changes?

2. Can Forge leak sensitive data?
   - forge-ship writes session summaries to MEMORY.md -- could this contain sensitive business logic?
   - The SessionEnd hook triggers episodic-memory sync -- what data is synced?
   - Codex adversarial review sends code to OpenAI. Is the user warned about this?
   - The PRD template captures business requirements. If committed to a public repo, is this a data leak?

3. Can Forge corrupt a git repo?
   - Multiple generators in worktrees + merge after evaluation. What if:
     - Two worktrees modify the same file (even in different waves)?
     - A merge conflict occurs and the auto-merge fails?
     - The lead force-pushes (nothing prevents this)?

### G2. Cost and Resource Risks

1. Token cost:
   - 5 teammates * their own context windows = 5x cost
   - The README acknowledges this but does not estimate cost
   - What is the estimated token cost for a greenfield fintech API project? (ballpark)

2. Resource leaks:
   - Orphaned tmux sessions (README section 7)
   - Orphaned git worktrees (never cleaned up if crash during build)
   - The codebase-memory-mcp server process (started by session-start.sh, when is it killed?)

3. Time risks:
   - The session guard is aspirational (tracked via `date +%s` in Bash) -- it has no enforcement mechanism
   - What happens at hour 3 if the guard never triggered?

### G3. Known Issues Audit

Review KNOWN_ISSUES.md and assess:
1. Which "Remaining" issues are actually CRITICAL for production use?
2. Which are genuinely minor?
3. Are there issues missing from the known issues list that this review uncovered?

---

## Output Format

Structure your findings as:

```
## Summary
[2-3 sentence overview]

## Critical Findings (MUST FIX before production use)
[numbered list with file:line references]

## High Findings (Should fix soon)
[numbered list with file:line references]

## Medium Findings (Should fix eventually)
[numbered list with file:line references]

## Low Findings (Nice to have)
[numbered list with file:line references]

## Architecture Assessment
[paragraph]

## Security Assessment
[paragraph]

## Production Readiness Verdict
[SAFE / SAFE_WITH_CAVEATS / NOT_SAFE_FOR_PRODUCTION]
[explanation]
```
