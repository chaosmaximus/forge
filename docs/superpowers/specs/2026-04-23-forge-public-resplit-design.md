# Forge Public Re-Split + Plugin Re-Merge — Phase 2P-1 Design

**Version:** v2 (2026-04-23, after adversarial reviews from Claude code-reviewer + Codex rescue; v1 graded C/D for history-preservation risk + under-specified audit/acceptance).
**Phase:** 2P-1 (Packaging stream — sibling to 2A-4, not inside it)
**Date:** 2026-04-23
**Parent motivation:** conversation with user 2026-04-23 (see `HANDOFF.md` §Lifted constraints); the 2026-04-12 split (commits `ef14b60` / `2084c41` / `1a30550`) moved the plugin, hooks, skills, and agent teams to the private `forge-app` repo alongside the Tauri GUI and commercial bits. That is misaligned with the intended "open core daemon + commercial GUI" model: the plugin/hooks/skills are the agent-side SDK that makes the daemon usable, not part of the GUI.
**Prerequisite:** 2A-4c2 T1-T8 shipped (through commit `7d1ef2c`), `chore(dev-env)` wired (`729f2d4`), ban-lift + harness-philosophy docs landed (`5d25256`).
**Target milestone:** public `chaosmaximus/forge` exposes the full agent-facing Forge surface — daemon + CLI + HUD + plugin + marketplace entry + hooks + skills + agent teams + Homebrew formula + install script — so a fresh `git clone` + two commands yields a working, agent-wired Forge.

## Summary

Undo the half of the 2026-04-12 split that should not have happened. Migrate the plugin manifest, hooks, skills (15+ `forge-*`), agent team definitions, templates, and Homebrew formula from private `forge-app` into public `chaosmaximus/forge`. Keep only the Tauri desktop app, licensing/Stripe, and internal product docs private.

**Migration method:** plain file copy (`cp -a` / `rsync`). No `git subtree`, no history import, no author rewrites. Each migrated layer lands as a single new commit on public master, authored normally (daemon-team authors + Claude co-author trailer). Rationale: forge-app's history contains commit messages, author emails, GPG signatures, and cross-layer commits touching private files. Importing any of that creates leak risk that no grep lexicon can fully cover. A clean copy is both safer and what the user asked for.

**Harness-sync contract (new CLAUDE.md philosophy made testable):** every daemon endpoint referenced by any hook/skill/agent/doc must either have a contract test in `crates/daemon/tests/` or a doc cross-reference in `docs/api-reference.md`. Enforced by a new CI check (§4).

**Audit contract:** each layer gets a `skill-creator` rubric pass + two adversarial subagent reviews (Claude code-reviewer + Codex rescue); passes are **evidence-gated, not vibes-gated** — reviewers produce a machine-parseable findings file (`docs/superpowers/reviews/2P-1-<layer>.yaml`) with a fixed severity vocabulary, and the layer does not merge unless HIGH/CRITICAL count is 0 in both files (§3).

**Scope cap:** this phase is packaging + one small-but-acknowledged new feature (the harness-sync CI check). No new daemon endpoints, no 2A-4 scope bleed. Explicitly restored from the 2026-04-12 deletion: `scripts/install.sh`, Claude Code hook section of `docs/getting-started.md`, `doctor`'s Hook health check, `crates/daemon/tests/test_hook_e2e.rs`. Explicitly new-and-justified: `scripts/check-harness-sync.sh` + CI wiring + plugin manifest / marketplace JSON-schema validation + shellcheck + markdownlint (framed as "new gating accompanying the re-introduction of plugin surface," not as restoration).

**Explicit non-goals:** public Tauri app; licensing/Stripe/tier enforcement; any new daemon endpoints; re-architecture of skills (audit may rewrite for quality, agent-facing semantics preserved); Windows plugin support (Linux + macOS only for 2P-1); automatic skill generation from daemon metadata; opening `product/engineering/` internal docs.

## 1. Architecture — after re-split

```
public chaosmaximus/forge (Apache-2.0)
├── crates/            — daemon, cli, core, hud (unchanged)
├── .claude-plugin/    — plugin.json, marketplace.json                     (NEW public)
├── agents/            — forge-planner, -generator, -evaluator             (NEW public)
├── hooks/             — hooks.json                                         (NEW public)
├── scripts/           — install.sh, setup-dev-env.sh, with-ort.sh,
│                        post-edit-format.sh, session-{start,end}.sh,
│                        protect-sensitive-files.sh, task-completed-gate.sh (install.sh
│                        + hook scripts NEW public)
├── skills/            — forge-{new,feature,tdd,ship,review,security,debug,
│                        migrate,research,setup,think,verify,handoff,agents,
│                        build-workflow}                                    (NEW public, 15+)
├── templates/         — greenfield/existing project scaffolds              (NEW public)
├── Formula/           — Homebrew formula                                   (NEW public)
├── mcp-servers/       — forge-graph MCP (iff still in forge-app at T1)    (conditional)
├── docs/              — daemon docs + restored Claude Code hook section +
│                        new docs/plugin/ (skill/hook authoring guide)
├── tests/             — test_hook_e2e.rs restored
└── .github/workflows/ — CI: cargo + plugin validation + marketplace lint +
                          shellcheck + markdownlint + check-harness-sync.sh

private chaosmaximus/forge-app (proprietary, post-2P-1 allowlist)
├── app/               — Tauri desktop (SolidJS, Cortex 3D, xterm)
├── licensing/         — Stripe, tier enforcement, license-check server
├── product/           — engineering handoffs, SESSION-GAPS, strategy docs
└── archive/           — historical / superseded material
```

## 2. Migration mechanics

### 2.1 Plain copy (no history import)

For each layer L:

1. `rsync -a --delete-excluded --exclude-from=scripts/migrate-exclude.txt forge-app/<L>/ forge/<L>/`
2. Run scrub gate (§2.2) over the newly-copied tree. Fail-closed on any match.
3. Stage + commit: `git add <L> && git commit -m "feat(2P-1 T<N>): migrate <L> from forge-app (plain copy)"`

No `--graft`, no `subtree split`, no author rewrites. History on public master is clean and linear. forge-app history stays where it is — anyone wanting to trace "when did this skill get added?" can follow a cross-reference in the migration commit message (`Original forge-app path: <L>/ as of forge-app@<sha>`) to the private repo if they have access.

### 2.2 Proprietary scrub gate (`scripts/migrate-scrub.sh`)

The scrub runs over **file content + binary metadata + filenames**. Because history is not imported, commit messages and author emails are out of scope.

- **Text scan** (`grep -rIF --files-with-matches -f lexicon.txt`):
  - Brand: `Bhairavi Tech`, `forge.bhairavi.tech`, `support@bhairavi.tech`, `@bhairavi.tech`
  - License: `"license": "Proprietary"`, `All rights reserved`
  - Author strings: `Bhairavi Tech`, `konurud@` in author fields (ok in git authorship of the copy's commit, not in file content)
  - Pricing / Stripe: `stripe.com`, `price_`, `sk_live_`, `pk_live_`, `.bhairavi.tech/pricing`
  - Internal URLs: `*.internal`, Notion/Linear/Slack invite URL patterns (`*.slack.com/T[A-Z0-9]+`, `linear.app/<workspace>`, `notion.so/<workspace>`), GitHub URLs pointing at private orgs
  - Cloud IDs: AWS account IDs (12-digit near `arn:aws`), S3 bucket names matching the org prefix, GPG key IDs
  - Internal path leaks: `forge-app-private/`, absolute paths containing `/home/<user>/` or `/Users/<user>/`
- **Binary asset scan**: `exiftool` EXIF/author strip on all `*.png`, `*.jpg`, `*.ico`, `*.pdf`; `strings <blob> | grep -f lexicon` on font blobs.
- **Filename scan**: no files matching `*SESSION-GAPS*`, `*STRATEGY*`, `*PRICING*`, `*-private.*`, `*.env`, `*.env.local`.
- **Exit code 0 iff all scans return 0 matches.** One match = pipeline stops, layer does not commit.

Lexicon lives at `scripts/migrate-lexicon.txt`; can be extended during T1 survey as contributors surface new patterns.

### 2.3 License retarget

Plugin manifest (`plugin.json`) and marketplace entry move from Proprietary → Apache-2.0. Underlying assets (hook scripts, skill MDs, agent MDs) are Apache-2.0 by inheritance; SPDX-License-Identifier headers added to every text file missing one.

**Decision dependency (was Codex CRITICAL #3):** the relicensing is valid **only if** all migrated assets are confirmed non-proprietary by the T1 inventory. Any skill/agent/hook flagged as commercially differentiated during T1 stays in `forge-app` and is excluded from its layer's migration. Relicensing of the migrated subset can then proceed safely.

### 2.4 Restoration of artifacts deleted in 2026-04-12 split

Per `1a30550` and `2084c41`, four non-proprietary artifacts come back:

1. `scripts/install.sh` — rewritten to point at public GitHub release artifacts, not `forge.bhairavi.tech`.
2. `docs/getting-started.md` Claude Code hook integration section.
3. `doctor`'s Hook health check in `crates/cli/src/commands/system.rs`.
4. `crates/daemon/tests/test_hook_e2e.rs`.

Plus one new-and-explicit gating step (called out as new, not disguised as restoration):

5. **CI plugin validation** — JSON-schema checks on `plugin.json` + `marketplace.json`, shellcheck on hook scripts, markdownlint on skill MDs. New gating, introduced alongside the re-introduction of the plugin surface.

## 3. Per-layer audit contract (enforceable)

### 3.1 skill-creator rubric pass

Invoke the `skill-creator` skill on every `skills/forge-*/SKILL.md` and every `agents/forge-*.md`. Required output: `docs/superpowers/reviews/2P-1-<layer>-skill-creator.yaml` with keys:

```yaml
layer: skills
artifacts:
  - path: skills/forge-new/SKILL.md
    trigger_clarity: pass | flag | fail
    prereqs_explicit: pass | flag | fail
    examples_coverage: pass | flag | fail
    commands_idempotent: pass | flag | fail
    style_alignment: pass | flag | fail
    findings: [ {severity: LOW|MEDIUM|HIGH|CRITICAL, note: "..."} ]
overall: pass | fail
```

"pass" = no HIGH or CRITICAL findings. Fixup commits close out HIGH/CRITICAL. File is machine-parseable and checked in.

### 3.2 Adversarial subagent review pass

Two independent subagents per layer, **inverted prompts**:

- **Claude `code-reviewer`-style (general-purpose agent)**: "assume the migration is good; find what breaks if a user relies on it." (Defend → attack)
- **`codex:codex-rescue`**: "assume the migration is bad; find the leak/break first." (Attack → defend)

Each produces `docs/superpowers/reviews/2P-1-<layer>-<reviewer>.yaml` using the same severity vocabulary. Layer merge is gated on `HIGH/CRITICAL count == 0` in both files. The gate is enforced by `scripts/check-review-artifacts.sh` wired into CI.

### 3.3 Contract tests

Every daemon endpoint referenced by a hook/skill/agent/doc gets either a contract test in `crates/daemon/tests/test_hook_e2e.rs` (or sibling) or a doc cross-reference in `docs/api-reference.md`. Enforced by §4.

## 4. Harness-sync contract (CI-enforced, scope clarified)

**Honest scope**: this IS new feature work, not "packaging only." It is in-scope for 2P-1 because without it the re-introduced plugin surface will drift from the daemon within weeks. Acknowledging the scope expansion openly beats smuggling it in.

`scripts/check-harness-sync.sh` scans for *every* way a daemon surface gets referenced:

- **JSON method literals** in hook scripts, agent MDs, skill MDs, templates, docs: `"method"\s*:\s*"(?P<m>[a-z_]+)"`
- **Rust test/fixture JSON**: `serde_json::json!\(\s*\{\s*"method"\s*:\s*"(?P<m>[a-z_]+)"` and variants with `"method": name_var`
- **CLI subcommand references**: `forge-next\s+(?P<sub>[a-z-]+)` in docs and skill MDs cross-referenced against `forge-cli`'s subcommand list
- **Rustdoc / prose references to `Request::<Variant>`**: `Request::(?P<v>[A-Z][A-Za-z_]+)` in any `.rs` or `.md` file

Cross-checked against the authoritative list of `Request::` variants in `crates/core/src/protocol/request.rs` and `forge-cli`'s clap derive. Failure modes:

- Method/CLI-subcommand referenced that does not exist → **FAIL**.
- `Request::` variant added in-tree with no reference in hooks/skills/docs after 14 days → **WARN** (opt-out via `// harness-sync: unreferenced-ok` comment on the variant).
- `Request::` variant removed/renamed with stale references → **FAIL** until references updated, or variant marked `#[deprecated]` with a grace-window in its rustdoc.

**Rollout**: CI check runs in `warn-only` mode starting at T3 merge, flips to `fail-closed` only at T10 merge (after T3-T9 layers are all present and synced). This prevents T3-T9 intermediate merges from failing CI on layers that are not yet migrated.

## 5. Task decomposition

| Task | Scope | Deps | Notes |
|------|-------|------|-------|
| **T1** | **Inventory + scope lock**: catalog every file in private `forge-app`; each marked migrate / stays-private / delete. Commercially-differentiated check (Codex §2.3): any skill/agent flagged stays private. Output: `docs/superpowers/plans/2P-1-inventory.md`. **Must be locked (user-signed) before T3.** | — | No code changes. |
| **T2** | **Migration tooling**: `scripts/migrate-scrub.sh` + `scripts/migrate-lexicon.txt` + `scripts/check-review-artifacts.sh`; no actual migration yet. Includes dry-run mode; tested on known-leaky fixture. | T1 | |
| **T3** | **Plugin manifest layer**: copy `.claude-plugin/{plugin.json, marketplace.json}`, retarget license + owner + homepage, marketplace schema validation, adversarial review + skill-creator (n/a for JSON — just human rubric). | T2 | Smallest layer. |
| **T4** | **Hooks layer**: copy `hooks/hooks.json` + hook shell scripts; shellcheck; contract-test per daemon endpoint touched; adversarial review. | T3 | |
| **T5** | **Skills layer**: copy `skills/forge-*` (subset confirmed non-commercial in T1); skill-creator pass per SKILL.md; adversarial review. Split into **≤ 800 LoC review chunks** T5a/T5b/... (hard threshold, not "if too big"). | T3 | |
| **T6** | **Agent teams layer**: copy `agents/forge-{planner,generator,evaluator}.md`; skill-creator pass; adversarial review. | T3 | |
| **T7** | **Templates + Homebrew**: copy `templates/` and `Formula/`; Formula download URL rewritten to public GitHub release. | T3 | |
| **T8** | **Restore deleted artifacts**: `scripts/install.sh`, `docs/getting-started.md` hook section, `doctor` Hook check, `crates/daemon/tests/test_hook_e2e.rs`. Depends on T4 (hook scripts present) AND T5 (skills present — `test_hook_e2e.rs` exercises skill→hook→daemon flow). | T4, T5 | |
| **T9** | **forge-graph MCP** *(conditional on T1)*: migrate if still in forge-app. | T1, T2 | |
| **T10** | **Harness-sync CI** (wire `scripts/check-harness-sync.sh` into CI, **flip warn-only → fail-closed in this task**). | T3-T9 | |
| **T11** | **CI plugin validation**: schema + shellcheck + markdownlint as required CI steps. | T3, T4, T5 | |
| **T12** | **Prune private repo**: in `forge-app`, delete the migrated paths; verify `forge-app` CI still green; commit as `chore(split): forge-app post-migration pruning`. **Owns acceptance criterion §10.6.** | T3-T9 | |
| **T13** | **Lift `forge-app-private` references** from `docs/superpowers/specs/` and `docs/superpowers/plans/` in public repo. Runs **before** T14. | T5 | |
| **T14** | **Holistic adversarial review** on the full T3-T13 diff: Claude code-reviewer + Codex rescue with inverted prompts. | T3-T13 | Gate before T15. |
| **T15** | **Dogfood + results doc** (expanded per §7). | T14 | |
| **T16** | **Rollback playbook**: `docs/operations/2P-1-rollback.md` — if a CRITICAL issue surfaces post-ship, how to back out (which commits to revert, how to restore forge-app, how to re-issue a marketplace patch). Ships **before** T15 so T15 dogfood validates rollback path lightly. | T14 | |

## 6. Adversarial-review checkpoints

- **Before T2**: review of migration tooling + scrub gate logic.
- **Per-layer** (T3-T9): two independent subagent reviews with inverted prompts; artifacts at `docs/superpowers/reviews/`.
- **T14 holistic**: full-diff review, cross-layer issues.

## 7. Dogfood plan (T15, expanded)

Proves the full agent-facing surface, not just a happy path:

1. Fresh-clone public `forge` on Ubuntu 22.04 (glibc 2.35) + one macOS host.
2. `scripts/setup-dev-env.sh && cargo build --release` green on both.
3. Install plugin via **each** canonical install mode — marketplace install AND symlink install. Verify both surface the same slash commands + skills.
4. Install homebrew formula on the macOS host; verify `forge-daemon --version` matches `cargo run --release -p forge-daemon -- --version`.
5. Start daemon in release mode.
6. Agent session flow:
   - Open Claude Code session → `session-start.sh` hook registers session (verify `register_session` call landed via daemon `/api` log).
   - Run real task ("summarize phase 2P-1 status") → `<skills>` context includes `forge-handoff` skill.
   - Post-edit hook fires after a code edit → daemon sees `record_tool_use` row.
   - `session-end.sh` hook → daemon sees session closed.
7. `forge-next recall "2P-1"` returns ≥ 1 memory with `session_id` matching step 6's session.
8. `doctor` output shows Hook health check passing.
9. **Negative test**: stop daemon mid-session → verify hooks degrade gracefully (non-zero exit from hook script is tolerated by Claude Code, no session wedge).
10. **Parallel-session test**: two Claude Code sessions share one daemon → `register_session` × 2, neither wedges the other.
11. Results doc at `docs/benchmarks/results/forge-public-resplit-2026-05-XX.md` (precise date at ship time).

## 8. Risks & mitigations

| Risk | Mitigation |
|------|-----------|
| Proprietary leak via file content | §2.2 fail-closed scrub on text + binary metadata. |
| Proprietary leak via binary assets | exiftool + strings scan in §2.2. |
| Commercial-skill accidentally migrated | §2.3 dependency on T1 inventory lock. |
| Harness-sync false positives / negatives | §4 explicit scope (JSON + Rust + CLI + rustdoc); opt-out annotation; rename grace-window. |
| CI stays red during T3-T9 migrations | §4 `warn-only` mode through T3-T9, `fail-closed` only at T10. |
| Marketplace listing collision (public plugin.name == private plugin.name) | T3 renames public plugin slug or bumps major version; marketplace resubmission task. |
| 2A-4d development during 2P-1 drifts new endpoints away from newly-migrated harness | Interlock protocol: any 2A-4d PR touching `crates/core/src/protocol/request.rs` must bump a sync version in `.claude-plugin/plugin.json` and update relevant hook/skill refs in the **same PR**. Enforced by §4 CI check (in `warn-only` during overlap, `fail-closed` after). |
| forge-app CI broken post-pruning | T12 acceptance includes "forge-app CI green at the pruned state". |
| User who sideloaded private plugin pre-ban-lift has stale copy | T15 results doc includes a short "migration note for sideload users" section. |
| Rollback if CRITICAL surfaces post-ship | T16 playbook exists and is exercised in T15. |

## 9. Out of scope

- Public Tauri app; licensing/Stripe/tier enforcement.
- New daemon endpoints (belong to 2A-4 stream).
- Re-architecture of skills (audit may rewrite for quality; agent-facing semantics preserved).
- Windows plugin support.
- Automatic skill generation from daemon metadata.
- Plugin internationalization / non-English skill variants.

## 10. Acceptance criteria (objectively verifiable)

Phase 2P-1 is shipped when ALL are true:

1. A specific commit SHA (recorded in `HANDOFF.md`) on public `master` contains all layers migrated per T1 inventory; at that SHA, T3-T16 have all landed.
2. `cargo test --workspace` green at the SHA (1700+ passed, 0 failed).
3. `scripts/check-harness-sync.sh` passes in `fail-closed` mode at the SHA.
4. `scripts/setup-dev-env.sh && cargo test --workspace` green from a fresh clone of public `forge` on **both** Ubuntu 22.04 (glibc 2.35) **and** macOS (latest stable, ARM or x86 per host availability).
5. Dogfood results doc at `docs/benchmarks/results/forge-public-resplit-2026-05-XX.md` records, for the two canonical install modes × two OSes: `register_session` observed, `<skills>` rendered with ≥ 1 `forge-*` skill, `record_tool_use` observed, `session-end` observed, `forge-next recall "<phase tag>"` returns ≥ 1 memory where `session_id` matches the session from the run.
6. Private `forge-app` at commit `≥ SHA` contains ONLY these top-level paths: `app/`, `licensing/`, `product/`, `archive/`, `README.md`, `CLAUDE.md`, `LICENSE`, `.git*`, `Cargo.toml`/`package.json` etc. required for Tauri build. No `plugin.json`, `marketplace.json`, `agents/`, `hooks/`, `skills/`, `templates/`, `Formula/`, `.claude-plugin/`.
7. `forge-app` CI green at its post-pruning SHA.
8. `HANDOFF.md` §Lifted constraints has a new entry dated at ship time noting 2P-1 completion, citing the public SHA + forge-app SHA.
9. `docs/operations/2P-1-rollback.md` exists and has been walked through in a tabletop exercise during T15.
10. A short migration note for sideload users is present in T15 results doc.

## 11. Open questions (T1 survey must resolve; NONE carry past T1 lock)

- **Q1**: `forge-graph` MCP server still in `forge-app`? — resolves T9 go/no-go.
- **Q2**: any `forge-*` skills commercially differentiated / contain proprietary benchmarks? — resolves T5 scope.
- **Q3**: `scripts/install.sh` per-OS variation strategy — resolves T8 scope.
- **Q4**: CLI subcommand references — do skills call `forge-next` subcommands? (If yes, widen §4 scope to include those; already budgeted in the regex list above.)
- **Q5**: GitHub Actions secrets in `forge-app` workflows that don't exist in public — if T7 or T11 imports those workflows, document which secrets need to be created in public org before the workflow is enabled.
- **Q6**: Marketplace listing — does Anthropic's Claude Code marketplace require a new submission for the renamed/relicensed public plugin, or is a version bump on the existing listing sufficient? (Out-of-band answer from user or Anthropic docs.)
- **Q7**: Submodules / Git LFS in forge-app — any? If yes, they cannot cross the copy without additional steps; decide migrate vs. skip in T1.

---

**Changelog from v1:**
- §2.1: `git subtree split` → plain copy. Drops every history-preservation risk (orphan commits, bisectability loss, author-email leak, commit-message leak, GPG signature loss).
- §2.2: scrub lexicon expanded (internal URLs, cloud IDs, GPG, Slack/Linear/Notion patterns); adds binary-asset exif + strings scan; adds filename scan.
- §2.3: license retarget now explicitly gated on T1 inventory.
- §2.4: typo fix (five items, not "four"); new §2.4.5 framed honestly as new gating.
- §3.1 + §3.2: audit rubrics are evidence-gated (machine-parseable YAML artifacts), not vibes-gated. CI `scripts/check-review-artifacts.sh` enforces HIGH/CRITICAL == 0.
- §3.2: inverted prompts between Claude + Codex subagents (attack vs defend) to preserve independence.
- §4: scope widened (JSON + Rust fixtures + CLI subcommands + rustdoc); rename-vs-delete handled via `#[deprecated]` grace window + opt-out annotation; `warn-only → fail-closed` transition explicit at T10.
- §5: hard 800 LoC threshold for T5 grouping; T8 deps now include T5; T12 (forge-app pruning) added as real task owning §10.6; T13 moves before T14; T16 rollback playbook added.
- §7: dogfood expanded to two OSes, two install modes, session-end, negative test, parallel-session test, specific recall assertion.
- §8: added risks — marketplace collision, 2A-4d interlock, forge-app CI, sideload-user migration, rollback.
- §10: removed "some commit X" variable; added macOS; made recall assertion specific; added explicit allowlist for forge-app; added forge-app CI criterion; added migration-note criterion; added rollback tabletop criterion.
- §11: all questions tagged T1-lockable; none carry past T1.

Ready for a second adversarial review pass on v2, or (if the changes land the grade) move to the execution plan at `docs/superpowers/plans/2026-04-23-forge-public-resplit.md`.
