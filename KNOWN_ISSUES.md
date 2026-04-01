# Known Issues — Forge v0.1.0

Issues identified during smoke testing. Tracked for v0.2.0.

## Critical (fixed in v0.1.1)

- [x] Test detection fails for monorepos — added pytest/conftest.py subdirectory search
- [x] PRD template missing CSV-referenced sections — needs section mapping (partial: documented below)
- [x] Missing .tfstate protection — added to protect-sensitive-files.sh
- [x] Missing Terraform formatter — added tf/tfvars case to post-edit-format.sh

## Important (v0.2.0)

### PRD Template & CSV Alignment
- PRD template uses generic section headers but CSVs reference specific IDs (api_design, multi_tenancy, etc.)
- Need: either a section ID-to-header mapping table, or expand PRD template with all referenced sections
- Workaround: Claude infers section content from the ID name (works reasonably well)

### Evaluator Scoring Alignment
- Evaluator inline criteria (Correctness, Test Coverage, etc.) don't exactly match rubric file criteria
- Need: evaluator should reference rubric files exclusively, remove inline criteria
- Include weighted average formula: `weighted_avg = sum(score * weight) / sum(weights)`

### Build Phase Duplication
- forge-new and forge-feature contain near-identical build phase instructions
- Need: extract shared build workflow to a separate reference file

### CONSTITUTION.md Enforcement
- Generator and evaluator agents don't check for or read CONSTITUTION.md
- Need: add "If CONSTITUTION.md exists, read it first" to both agents

### Planner Role Clarity (Greenfield)
- Planner agent has greenfield logic but forge-new never spawns it
- Resolution: forge-new does classification/discovery itself (correct), planner is only spawned for existing codebase mode. Remove or clearly mark greenfield section in planner as "reference only."

### Visual Design for Non-UI Projects
- forge-new Phase 5 offers Stitch even for api_backend (no frontend)
- Need: check if ui_design is in skip_sections, auto-skip Phase 5

### default_generator_model Config
- Hardcoded model: opus in agent frontmatter overrides userConfig
- Need: either use model: inherit and let skill specify, or document that model is set via spawn prompt

### Phase Names Inconsistency
- STATE.md uses greenfield phases only (classify/discover/prd/design/plan/build/review/ship)
- Existing mode uses different phases (explore/plan/build/review/ship)
- Need: universal phase enum covering both modes

### No Serena Fallback
- If Serena is not installed, forge-feature Phase 1 has no tertiary fallback
- Need: "If both graph and Serena unavailable, use Grep/Read with warning"

### BLOCKED/FAIL Circuit Breaker
- No retry limit for generators that fail evaluation or report BLOCKED
- Need: max 3 retries, then escalate to user

### Session Timer
- 90/120 minute guards have no timing mechanism
- Workaround: Claude estimates from turn count and tool call duration

## Minor (v0.2.0)

- Delegate mode references Shift+Tab (user keybinding, not agent action)
- git add -A in handoff could stage sensitive files
- Codex invocation syntax should be validated against actual plugin API
- Companion plugin install commands need validation
- forge-setup disable-model-invocation conflicts with interactive steps
- Evaluator Codex gate overlaps with forge-review Codex gate
- No weighted average formula specified in rubrics
- Resume mode lacks specificity on how to re-establish teams
- Base branch placeholder not auto-resolved for greenfield
