# Changelog

## 0.1.0 (2026-04-01)

### Added
- Two modes: greenfield (`/forge:new`) and existing codebase (`/forge:feature`)
- 3 agents: forge-planner, forge-generator, forge-evaluator
- 7 skills: forge, forge-new, forge-feature, forge-review, forge-ship, forge-handoff, forge-setup
- Bundled codebase-memory-mcp for code graph intelligence (66 languages)
- CSV-driven domain knowledge injection for PRD creation (7 project types, 6 domains)
- Cross-model Codex adversarial review integration
- Graded evaluation rubrics (code quality, security, architecture, infrastructure)
- Google Stitch MCP integration for visual design (optional)
- Session lifecycle hooks (test gate, idle checkpoint, auto-format, file protection, memory sync)
- Templates: CONSTITUTION.md, STATE.md, HANDOFF.md, PRD.md
- Delegation to superpowers, episodic-memory, serena, codex-plugin-cc, frontend-design
