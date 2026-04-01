# Changelog

## 0.1.4 (2026-04-01)

### Security (Codex adversarial review findings)
- install-server.sh: SHA256 checksum verification for downloaded binary
- protect-sensitive-files.sh: symlink resolution via readlink -f, expanded blocked file list
- post-edit-format.sh: workspace boundary validation for file paths
- session-end-memory.sh: provenance check for executed JS files
- forge-planner: Bash removed from tools (truly read-only)
- forge-evaluator: Bash constrained to test execution only

## 0.1.3 (2026-04-01)

### Build Workflow
- Extract shared build phase to forge-build-workflow.md (DRY)
- Session timer via `date +%s` elapsed time tracking
- Wave sizing guidelines (testable increments, max 4 tasks/wave)
- Merge strategy: `git merge --no-ff` with manual conflict resolution

### Skills
- forge-new: discovery escape hatch, wave granularity framework, start-over handling
- forge-feature: graph fallback mid-session, clarify requirements phase (1b)
- forge-review: resilient Codex syntax, explicit ship transition, prod_paths passing
- forge-setup: install validation note, prod_paths customization
- forge-ship: MEMORY.md gitignore check
- forge-handoff: unified phase names, safe git staging, expanded language coverage

### Evaluator
- Auto-fail tracking field in structured output
- Rubric auto-fail rule reminder

## 0.1.2 (2026-04-01)

### Round 2 Smoke Test Fixes
- forge-feature: Phase 1b (Clarify Requirements) between explore and plan
- Evaluator: auto-fail rule reminder in scoring section
- task-completed-gate.sh: full branch diff (merge-base) not just HEAD~1
- forge-handoff: explicit STATE.md/HANDOFF.md staging with public repo warning
- forge-generator: Serena fallback notes inline
- forge-new: start-over handling at PRD approval

## 0.1.1 (2026-04-01)

### Round 1 Smoke Test Fixes (23 items)
- PRD template expanded with all 25+ CSV-referenced section IDs and mapping
- Evaluator: rubric-file-driven scoring with weighted averages
- Generator: model: inherit, CONSTITUTION.md check at step 0
- Planner: greenfield role clarity, universal rules fixes
- Skills: visual design skip for non-UI, full PRD presentation, plan HARD-GATE,
  Serena fallback, circuit breaker, STATE.md creation, Codex gate blocking,
  resume expansion, unified phase names, safe git staging
- Hook scripts: monorepo test detection, .tfstate protection, Terraform formatter
- Plugin.json: agents as array, userConfig with type/title fields

## 0.1.0 (2026-04-01)

### Initial Release
- Two modes: greenfield (`/forge:new`) and existing codebase (`/forge:feature`)
- 3 agents: forge-planner, forge-generator, forge-evaluator
- 7 skills: forge, forge-new, forge-feature, forge-review, forge-ship, forge-handoff, forge-setup
- Bundled codebase-memory-mcp for code graph intelligence
- CSV-driven domain knowledge (7 project types, 6 domains)
- Cross-model Codex adversarial review integration
- Graded evaluation rubrics (code quality, security, architecture, infrastructure)
- Google Stitch MCP integration (optional)
- Session lifecycle hooks (test gate, auto-format, file protection, memory sync)
- Comprehensive test suite (static + BATS + integration)
