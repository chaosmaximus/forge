# Known Issues — Forge v0.1.x

## Fixed

### v0.1.1 (23 fixes from Round 1 smoke tests)
All critical and important issues from initial greenfield + existing codebase smoke tests.
See git log for details.

### v0.1.2 (6 fixes from Round 2 smoke tests)
- [x] No discovery/requirements step in forge-feature — added Phase 1b (Clarify Requirements)
- [x] Evaluator omits auto-fail sub-rules — added reminder to apply all rubric pass criteria
- [x] Prod path detection only checks HEAD~1 — now checks full branch diff via merge-base
- [x] Handoff stages *.md blindly — explicit STATE.md/HANDOFF.md staging with public repo warning
- [x] Generator Serena tools no fallback inline — added Grep fallback notes
- [x] No "start over" handling at PRD approval — resets STATE.md, returns to Phase 1

## Remaining (v0.2.0)

### Minor / Design
- Session timer (90/120 min) has no timing mechanism
- Build phase partially duplicated between forge-new and forge-feature
- Codex invocation syntax should be validated against actual codex-plugin-cc API
- Companion plugin install commands need end-to-end validation
- No escape hatch for minimal discovery (user wants to skip questions)
- No wave granularity decision framework for greenfield
- Merge strategy (rebase vs merge commit) unspecified for worktree merges
- Stitch MCP loaded unconditionally via .mcp.json even when not needed
- user_journeys_visual has no distinct template section (skip-only signal)
- forge-review doesn't explicitly invoke forge-ship (relies on lead checklist)
- No fallback if graph server dies mid-session after initial success
- Evaluator output template has no auto-fail tracking field
- forge-ship writes MEMORY.md without checking .gitignore
- Custom prod_paths must be passed by lead to evaluator (no env var access)
- Default prod_paths won't match variant names (hive_production, prod, live)
