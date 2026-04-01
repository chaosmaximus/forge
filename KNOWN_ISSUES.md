# Known Issues — Forge v0.1.x

## Fixed in v0.1.1

- [x] PRD template missing CSV-referenced sections — expanded with all 25+ section IDs + mapping
- [x] Test detection fails for monorepos — added conftest.py/pytest subdirectory search
- [x] Missing .tfstate protection — added to blocked list
- [x] Missing Terraform formatter — added tf/tfvars support
- [x] Evaluator scoring mismatch — now rubric-file-driven with weighted averages
- [x] Evaluator can't access prod_paths — explicit defaults embedded
- [x] Generator doesn't check CONSTITUTION.md — step 0 added
- [x] Planner role confusion in greenfield — clarified as delegation-only
- [x] default_generator_model has no effect — model: inherit in agent frontmatter
- [x] Visual design triggers for non-UI projects — checks skip_sections first
- [x] Section-by-section PRD approval too slow — presents full PRD at once
- [x] Plan approval lacks HARD-GATE — added
- [x] No Serena fallback — Grep/Read tertiary fallback added
- [x] No BLOCKED/FAIL circuit breaker — 3-retry limit then escalate
- [x] Phase names inconsistent — unified across both modes
- [x] Router doesn't check Serena — added to prerequisite checks
- [x] Codex not-installed contradiction — blocks for prod, warns for non-prod
- [x] forge-setup disable-model-invocation — removed for interactivity
- [x] STATE.md never created in workflows — both modes create it at start
- [x] git add -A in handoff — safe staging with specific extensions
- [x] Resume mode lacks specificity — expanded with mode detection and fresh teammates
- [x] Bug fix "skip to build" contradicts HARD-GATE — scoped to existing mode only
- [x] "Never specify file paths" too broad — carve-out for existing codebase

## Remaining Minor Items (v0.2.0)

- Session timer (90/120 min) has no timing mechanism — Claude estimates from context
- Delegate mode references Shift+Tab keybinding — should say "enter delegate mode" generically
- Build phase partially duplicated between forge-new and forge-feature — extract to shared reference
- Codex invocation syntax should be validated against actual plugin API
- Companion plugin install commands need end-to-end validation
- No restart path after Phase 4 in greenfield mode
- No escape hatch for minimal discovery (user wants to skip questions)
- No wave granularity decision framework for greenfield
- No guidance for partial wave success (2 of 3 tasks pass, 1 fails)
- Merge strategy (rebase vs merge commit) unspecified for worktree merges
- Stitch MCP loaded globally even when not needed in existing mode
