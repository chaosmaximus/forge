# Phase 2P-1a — Inventory + Scope Lock

**Status:** DRAFT — awaiting user sign-off before T3a unblocks.
**Source SHA (frozen):** `480527b57c01aeed4052db13ed07c9140302786b` — `docs(session-17): expert council + 24 dogfood bugs + V3 strategy` on `forge-app` master as of 2026-04-23.
**Local source path:** `/mnt/colab-disk/DurgaSaiK/forge/forge-app/` (clone of `chaosmaximus/forge-app`).
**Spec:** `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md` v3, Phase 2P-1a §4.

## 1. Top-level classification

| Path | Type | Decision | Notes |
|------|------|----------|-------|
| `.claude-plugin/plugin.json` | file | **MIGRATE** | Retarget license `Proprietary` → `Apache-2.0`; owner `Bhairavi Tech` → `Forge Contributors`; drop `homepage: forge.bhairavi.tech` (§2.3). |
| `.claude-plugin/marketplace.json` | file | **MIGRATE** | Same retargets as plugin.json. |
| `marketplace.json` (root) | file | **DELETE (don't migrate)** | Identical to `.claude-plugin/marketplace.json` per diff — duplicate. Claude Code schema wants the canonical copy under `.claude-plugin/`. |
| `agents/` | dir | **MIGRATE** | 3 agent MDs + `README.md`. |
| `hooks/hooks.json` | file | **MIGRATE** | |
| `scripts/hooks/` | dir | **MIGRATE** | 11 hook shell scripts (post-bash, post-compact, post-edit, pre-bash, pre-edit, session-end, session-start, stop, subagent-start, task-completed, user-prompt). |
| `scripts/post-edit-enhanced.sh` | file | **MIGRATE** | |
| `scripts/protect-sensitive-files.sh` | file | **MIGRATE** | |
| `scripts/task-completed-gate.sh` | file | **MIGRATE** | |
| `scripts/teammate-idle-checkpoint.sh` | file | **STAYS PRIVATE** | Internal teammate-workflow tooling. |
| `scripts/upload-release.sh` | file | **STAYS PRIVATE** | Release tooling that targets private infrastructure. |
| `skills/` | dir | **MIGRATE** | All 15 skill dirs + 1 shared reference (see §2). No commercial-differentiation flag (§Q2). |
| `Formula/forge.rb` | file | **MIGRATE** | Rewrite Homebrew download URL to public GitHub release artifact. |
| `templates/CLAUDE.md.forge-template` | file | **MIGRATE** | |
| `tests/unit/` | dir | **MIGRATE** | BATS tests for migrated hook scripts (7 `.bats` + `fixtures/`). |
| `tests/integration/` | dir | **MIGRATE** | e2e-journey, hook-behavior, performance, plugin-loading. |
| `tests/static/` | dir | **MIGRATE** | 9 validation scripts (plugin/hooks/skills/agents/rubrics/templates/csv + shellcheck). |
| `tests/claude-code/` | dir | **MIGRATE** | test-agent-spawning, test-skill-invocation. |
| `tests/run-all.sh` | file | **MIGRATE** | Orchestrator; clean — no proprietary refs. |
| `tests/codex-adversarial-prompt.md` | file | **MIGRATE, but UPDATE** | Stale — says plugin v0.1.0; current is 0.7.0. Freshen as part of T3a. |
| `app/` | dir | **STAYS PRIVATE** | Tauri SolidJS desktop. |
| `archive/` | dir | **STAYS PRIVATE** | `legacy-v030/`, `swift-app-v01/` historical. |
| `product/` | dir | **STAYS PRIVATE** | Internal business/engineering/marketing/ops/growth/cross-team. Contains broken submodule ref at `product/marketing/taste-skill` — orphaned from history, stays private, ignore for migration. |
| `docs/site/` | dir | **STAYS PRIVATE** | Marketing website (Astro/Starlight). |
| `docs/blog/` | dir | **STAYS PRIVATE** | Blog posts. |
| `docs/plans/` | dir | **STAYS PRIVATE** | Internal planning. |
| `docs/specs/` | dir | **STAYS PRIVATE** | Internal specs. |
| `docs/superpowers/` | dir | **STAYS PRIVATE** | forge-app's own internal workflow docs — distinct from public repo's `docs/superpowers/` which holds the daemon phase specs. |
| `docs/archive/` | dir | **STAYS PRIVATE** | Historical. |
| `CLAUDE.md` (root) | file | **STAYS PRIVATE** | forge-app dev guide (commercial surface). |
| `HANDOFF.md` (root) | file | **STAYS PRIVATE** | forge-app session handoff. |
| `STATE.md` (root) | file | **STAYS PRIVATE** | forge-app session state. |
| `INSIGHTS.txt` (root) | file | **STAYS PRIVATE** | Untracked in forge-app's git — working-tree file. Not migrated regardless. |
| `README.md` (root) | file | **STAYS PRIVATE** | Explicitly "Forge (Private)". forge-app keeps this version; public `forge` keeps its own. |
| `LICENSE` (root) | file | **STAYS PRIVATE** | Proprietary license for forge-app. |
| `package.json` + `package-lock.json` | files | **STAYS PRIVATE** | Single devDep: `@wdio/cli` (WebdriverIO) for Tauri E2E tests. |
| `.gitignore` | file | **STAYS PRIVATE (reference only)** | forge-app-specific. Do not overwrite public's. |

## 2. Skills inventory (all 15 MIGRATE)

| Skill dir | Has SKILL.md | Commercial-marker scan | Decision |
|-----------|--------------|------------------------|----------|
| `skills/forge/` | ✓ | clean | MIGRATE |
| `skills/forge-agents/` | ✓ | clean | MIGRATE |
| `skills/forge-debug/` | ✓ | clean | MIGRATE |
| `skills/forge-feature/` | ✓ | clean | MIGRATE |
| `skills/forge-handoff/` | ✓ | clean | MIGRATE |
| `skills/forge-migrate/` | ✓ | clean | MIGRATE |
| `skills/forge-new/` | ✓ | clean | MIGRATE |
| `skills/forge-research/` | ✓ | clean | MIGRATE |
| `skills/forge-review/` | ✓ | clean | MIGRATE |
| `skills/forge-security/` | ✓ | **flagged**: description contains the word "security" but no commercial gating (scan for secrets, fingerprint only, no paid tier) | MIGRATE |
| `skills/forge-setup/` | ✓ | clean | MIGRATE |
| `skills/forge-ship/` | ✓ | clean | MIGRATE |
| `skills/forge-tdd/` | ✓ | clean | MIGRATE |
| `skills/forge-think/` | ✓ | clean | MIGRATE |
| `skills/forge-verify/` | ✓ | clean | MIGRATE |
| `skills/forge-build-workflow.md` | — (shared reference, not a skill dir) | clean | MIGRATE |

All 15 skill dirs have `SKILL.md`. None are commercially differentiated. The `forge-security` grep hit is a false positive (description mentions "security" as the skill's topic, not as a gated feature).

## 3. Baseline brand/license leak scan on migrate candidates

Pre-migration grep for `(Bhairavi|bhairavi\.tech|forge\.bhairavi|support@bhairavi|Proprietary)` in migrate-candidate paths returns **3 files**:

1. `.claude-plugin/plugin.json` — fixed by T3a license retarget.
2. `.claude-plugin/marketplace.json` — fixed by T3a license retarget.
3. `Formula/forge.rb` — fixed by T3a download-URL rewrite.

All three are expected and handled by §2.3 of the spec. The scrub gate (§2.2) should pass zero matches **after** T3a's retarget step runs. If anything else trips the scrub gate at T3a time, migration aborts for human review.

## 4. Open questions — all resolved

| Q | Question | Answer |
|---|----------|--------|
| **Q1** | Is `forge-graph` MCP server present in forge-app? | **NO.** No `mcp-servers/` dir, no `forge-graph/` dir anywhere in forge-app at frozen SHA. T9 from v2 was already conditional; it is skipped entirely in 2P-1a. |
| **Q2** | Any `forge-*` skills commercially differentiated? | **NO.** All 15 skills are generic agent workflows with no paid-tier gating. `forge-security` flagged on description keyword only; content is pure secret-scanning, no commercial layer. |
| **Q3** | `scripts/install.sh` per-OS variation? | Not applicable in 2P-1a — spec §6 acceptance is Linux-only. macOS install variant + Homebrew testing moves to 2P-1b. T3a's restored `install.sh` ships Linux-only with a clear "macOS pending" note. |
| **Q4** | Submodules / Git LFS present in migrate-candidate layers? | **NO.** git lfs is not installed and nothing in forge-app uses LFS. One broken submodule reference at `product/marketing/taste-skill` — lives entirely under private `product/`, ignored for 2P-1a. |
| **Q5** | GitHub Actions secrets that need to be created in public org? | **NONE.** forge-app has no `.github/` directory; no workflows to migrate. Release automation lives in `scripts/upload-release.sh` which stays private. |
| **Q6** | Marketplace republication strategy? | **Deferred to 2P-1b** per spec v3 §2P-1b item 6. 2P-1a's plugin installs via direct-path / symlink for dogfood; formal marketplace listing is a 2P-1b task. |
| **Q7** | package.json / package-lock.json handling? | **STAYS PRIVATE.** Only devDep is `@wdio/cli` for Tauri E2E tests. Not needed by any migrated layer. Public `forge` has no `package.json` today; none introduced by 2P-1a. |

## 5. Post-migration forge-app allowlist (for T5a pruning + §6 acceptance)

After T5a prunes forge-app, its root MUST contain only:

**Directories:** `app/`, `archive/`, `product/`, `docs/` (site/blog/plans/specs/superpowers/archive subdirs only), `.git/`, `.github/` (if any is added for Tauri CI in future).

**Files:** `app/*` (Tauri build config etc.), `README.md`, `CLAUDE.md`, `HANDOFF.md`, `STATE.md`, `INSIGHTS.txt` (if committed by then), `LICENSE`, `package.json`, `package-lock.json`, `.gitignore`.

**Explicitly absent:** `.claude-plugin/`, `marketplace.json`, `agents/`, `hooks/`, `skills/`, `templates/`, `Formula/`, `scripts/hooks/`, `scripts/post-edit-enhanced.sh`, `scripts/protect-sensitive-files.sh`, `scripts/task-completed-gate.sh`, `tests/unit/`, `tests/integration/`, `tests/static/`, `tests/claude-code/`, `tests/run-all.sh`, `tests/codex-adversarial-prompt.md`.

## 6. Action items unlocked by this inventory

- **T2a (tooling)**: `scripts/migrate-copy.sh` reads this file's §1/§2 as the source of truth for what `rsync` transfers. The path list is machine-derivable from the MIGRATE rows.
- **T3a (migration commit)**: uses the frozen SHA from the header + license-retarget rules from §3 + restoration scope from spec §2.4.
- **T5a (forge-app prune)**: uses §5 as the target end-state.
- **T6a (HANDOFF update)**: cites this inventory by path + commit SHA.

## 7. User sign-off

This inventory is the authoritative scope for 2P-1a. No file outside the MIGRATE rows will be copied by T3a. Any subsequent surface additions go through 2P-1b, not this phase.

**User, please confirm** (or redirect any row you disagree with) before T2a tooling work begins:

- [ ] Top-level classification in §1 is correct.
- [ ] All 15 skills migrate per §2 — no hidden commercial dependencies.
- [ ] §3 baseline leak scan is exhaustive enough (3 expected, fixed by retarget).
- [ ] §4 answers to Q1-Q7 are correct.
- [ ] §5 forge-app post-prune allowlist is acceptable.

Once confirmed, the inventory SHA is locked and T2a begins.
