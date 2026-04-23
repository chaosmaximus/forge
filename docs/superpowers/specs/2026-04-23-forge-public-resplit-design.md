# Forge Public Re-Split + Plugin Re-Merge — Phase 2P-1 Design

**Version:** v3 (2026-04-23, after two rounds of adversarial review).
**Split:** **2P-1a (Move)** — ship the plain copy-paste migration, scoped tight. **2P-1b (Harden)** — continue perfecting the harness (CI checks, evidence-gated audit, expanded dogfood, rollback playbook, marketplace ownership, 2A-4d interlock) against real in-tree code after the move lands.
**Rationale for the split:** v2 graded B (Claude) / D (Codex). Most of the ceremony the reviewers wanted (harness-sync CI, YAML-gated audit, inverted-prompt subagent passes per layer, multi-OS × multi-install dogfood matrix, rollback playbook as prerequisite) is genuinely valuable — but it has nothing to do with the actual copy operation. User direction: simple copy-paste migration, truly complete; *continue perfecting afterwards*. So the move lands in 2P-1a; the hardening ideas get their own phase (2P-1b) where they operate on code that's already in the public repo.

**Parent motivation:** conversation 2026-04-23 (see `HANDOFF.md` §Lifted constraints). 2026-04-12 split (`ef14b60`/`2084c41`/`1a30550`) moved plugin + hooks + skills + agent teams to private `forge-app` alongside Tauri/commercial bits — wrong side of the line. 2P-1 puts the agent-facing layers back in public.
**Prerequisite:** 2A-4c2 T1-T8 shipped (`7d1ef2c`), `chore(dev-env)` wired (`729f2d4`), ban-lift + harness-philosophy docs landed (`5d25256`), v1 + v2 adversarial reviews absorbed (`1f87606`, `65f5ea1`).

---

# Phase 2P-1a: Move (ships first)

## Summary

Plain file copy (`cp -a`/`rsync`) from private `forge-app` to public `chaosmaximus/forge`: plugin manifest + marketplace + hooks + skills + agent teams + templates + Homebrew formula. No git history import — nothing from `forge-app/.git` crosses the boundary. Scrub, review once, commit, prune private repo to allowlist, dogfood end-to-end, land `HANDOFF.md` update, done.

**Scope cap:** migration only. No new daemon endpoints. No new CI checks beyond what's needed to validate the copy landed (schema check on plugin.json, shellcheck on migrated hooks). Explicitly deferred to 2P-1b: harness-sync CI check, evidence-gated YAML audit artifacts, multi-OS dogfood matrix, rollback playbook, marketplace-republication task ownership, 2A-4d interlock mechanism, SPDX header backfill for files that support comments.

## 1. Architecture — after 2P-1a ships

```
public chaosmaximus/forge (Apache-2.0)
├── crates/            — unchanged
├── .claude-plugin/    — plugin.json, marketplace.json (NEW public)
├── agents/            — forge-planner, -generator, -evaluator (NEW public)
├── hooks/             — hooks.json (NEW public)
├── scripts/           — hook shell scripts + install.sh (restored) (NEW public)
├── skills/            — forge-{new,feature,tdd,ship,review,security,debug,
│                        migrate,research,setup,think,verify,handoff,agents,
│                        build-workflow}  (NEW public; subset per T1 lock)
├── templates/         — (NEW public)
├── Formula/           — Homebrew formula (NEW public, download URL → GitHub release)
├── mcp-servers/       — forge-graph (iff present in forge-app at T1)
├── docs/              — daemon docs + restored Claude Code hook section
├── tests/             — test_hook_e2e.rs restored
└── .github/workflows/ — CI: existing cargo + plugin.json schema + shellcheck

private chaosmaximus/forge-app (proprietary, post-2P-1a allowlist)
├── app/               — Tauri desktop only
├── licensing/         — Stripe, tier enforcement
├── product/           — internal engineering docs
└── archive/
```

## 2. Migration mechanics

### 2.1 Plain copy, one commit

`rsync -a --exclude-from=scripts/migrate-exclude.txt forge-app@<FROZEN_SHA>:/<layers> → forge/<layers>`. All layers in one operation. One commit on public master:

```
feat(2P-1a): migrate plugin surface from forge-app (source: forge-app@<SHA>)
```

Commit body records: the frozen SHA, the subset of layers migrated (from T1 inventory), SPDX header status per file type, scrub pass confirmation.

### 2.2 Proprietary scrub gate (`scripts/migrate-scrub.sh`)

Runs over file content + binary metadata + filenames. Fail-closed on any match. Covers:

**Text scan** (`grep -rIF --files-with-matches -f lexicon.txt` on all text files):
- Brand: `Bhairavi Tech`, `forge.bhairavi.tech`, `support@bhairavi.tech`, `@bhairavi.tech`
- License: `"license":\s*"Proprietary"`, `All rights reserved`
- Commercial: `stripe.com`, `price_`, `sk_live_`, `pk_live_`, `.bhairavi.tech/pricing`
- Internal URLs: `*.internal`, `*.slack.com/T[A-Z0-9]+`, `linear.app/<workspace>`, `notion.so/<workspace>`, private-org GitHub URLs
- Cloud IDs: 12-digit AWS account IDs near `arn:aws`, S3 buckets matching org prefix, GPG key IDs
- Internal paths: `forge-app-private/`, absolute `/home/<user>/`, `/Users/<user>/`

**Binary-asset scan** — covers the Codex CRITICAL from v2. Run on every:
- Image: `*.png`, `*.jpg`, `*.jpeg`, `*.svg`, `*.webp`, `*.gif`, `*.ico` — `exiftool` strip + author scan + `strings` keyword match
- Font: `*.woff`, `*.woff2`, `*.ttf`, `*.otf`, `*.eot` — `strings` keyword match
- Archive: `*.zip`, `*.tar`, `*.tgz`, `*.tar.gz`, `*.tar.bz2`, `*.7z` — extract to temp dir, recurse scan
- Database: `*.sqlite`, `*.sqlite3`, `*.db` — refuse to migrate (surface in T1 for case-by-case decision)
- WASM: `*.wasm` — `strings` keyword match

**Filename scan** — refuse: `*SESSION-GAPS*`, `*STRATEGY*`, `*PRICING*`, `*-private.*`, `*.env`, `*.env.local`.

**Exit 0 iff zero matches across all three scans.** One match = pipeline aborts; no commit.

Lexicon at `scripts/migrate-lexicon.txt`, extensible during T1 as new patterns surface.

### 2.3 License retarget (conservative — fixes Codex HIGH "SPDX breaks JSON")

Plugin manifest + marketplace entry: `"license": "Proprietary"` → `"Apache-2.0"`, owner string → `"Forge Contributors"`.
SPDX headers: **only on files that natively support comments** (`.rs`, `.sh`, `.md`, `.toml`, `.py`, `.yaml`). **Not** on `.json` — adding `// SPDX-...` would break JSON parse before the schema-check CI step. SPDX backfill for JSON files is deferred to 2P-1b (uses a sibling `LICENSE-<file>.spdx` sidecar or manifest-level assertion, TBD).

Relicense is valid **only if** T1 inventory confirms no commercially-differentiated assets were migrated. This is enforced at T1 lock — any skill/agent/hook flagged stays in `forge-app` and is excluded from the copy.

### 2.4 Restoration of 2026-04-12 deletions

Restored in the same migration commit (no separate task needed):

1. `scripts/install.sh` — rewritten, points at public GitHub release artifacts.
2. `docs/getting-started.md` Claude Code hook integration section.
3. `doctor`'s Hook health check in `crates/cli/src/commands/system.rs`.
4. `crates/daemon/tests/test_hook_e2e.rs`.

Plus minimal CI gating alongside:

5. `plugin.json` JSON-schema validation, `marketplace.json` schema validation, `shellcheck` on migrated hook scripts. Called out honestly as new CI gating accompanying the re-introduced surface (not "restoration").

## 3. Review contract (simple)

**One** adversarial review pass before the final commit: dispatch Claude `general-purpose` + Codex `codex:rescue` in parallel on the staged diff. Defend vs. attack prompts (same independence principle as v2 §3.2) but the outputs are **prose punch lists**, not YAML. HIGH/CRITICAL must be addressed before commit — acknowledged by human reading, not a CI check.

(The evidence-gated YAML-artifact contract from v2 §3.1/3.2 moves to 2P-1b where the audit runs continuously against in-tree code.)

## 4. Task decomposition (6 tasks)

| Task | Scope | Deps |
|------|-------|------|
| **T1a** | **Inventory + scope lock + freeze source SHA**: catalog every file/dir in `forge-app`; mark `migrate` / `stays-private` / `delete`; resolve Q1 (forge-graph), Q2 (commercially-differentiated skills), Q7 (submodules/LFS); freeze `forge-app@<SHA>` as the authoritative source. Output: `docs/superpowers/plans/2P-1a-inventory.md`. **User must sign the inventory before T3a.** | — |
| **T2a** | **Migration tooling**: `scripts/migrate-scrub.sh` + `scripts/migrate-lexicon.txt` + `scripts/migrate-exclude.txt`; dry-run mode prints what would copy + any scrub hits without writing. Tested on a known-leaky fixture in `tests/fixtures/scrub/`. | T1a |
| **T3a** | **Copy + scrub + restore + commit**: run `scripts/migrate-copy.sh` which (a) rsync's inventory-approved layers from frozen SHA, (b) runs §2.2 scrub (abort on hit), (c) applies §2.3 license retarget, (d) restores the four §2.4 deletions, (e) stages the combined tree, (f) stops for human review. If review passes, human does `git commit`. **Single commit, one task.** | T1a, T2a |
| **T4a** | **CI gating**: `plugin.json` + `marketplace.json` schema checks, `shellcheck` on `hooks/`+`scripts/*.sh`, `markdownlint` on `skills/*.md`. Wired into `.github/workflows/ci.yml` as a required step. | T3a |
| **T5a** | **Prune forge-app to allowlist**: in private `forge-app`, delete the migrated paths; verify `forge-app` CI still green; commit as `chore(split): forge-app post-2P-1a pruning`. Owns acceptance criterion §6.6. | T3a |
| **T6a** | **Dogfood + review + HANDOFF**: (a) adversarial review pass per §3 on T3a's diff; (b) one end-to-end dogfood on Linux (session-start + real task + session-end + recall with matching `session_id`); (c) update `HANDOFF.md` §Lifted constraints with the 2P-1a SHA + forge-app pruning SHA + pointer to 2P-1b. | T3a, T4a, T5a |

## 5. Risks & mitigations (2P-1a only)

| Risk | Mitigation |
|------|-----------|
| Proprietary leak in file content | §2.2 fail-closed text scrub. |
| Proprietary leak in binary asset | §2.2 expanded binary + font + archive + sqlite + wasm scan. |
| Commercial skill accidentally migrated | §2.3 dep on T1 inventory lock. |
| Source drifts during migration | T1a freezes forge-app SHA; T3a copies from that SHA only. |
| SPDX headers break JSON schema check | §2.3 conservative — SPDX only on comment-supporting formats; JSON backfill deferred to 2P-1b. |
| forge-app CI broken post-prune | T5a acceptance requires forge-app CI green at the pruned SHA. |
| Migration commit has hidden issues the one review misses | Accept risk for 2P-1a; 2P-1b's continuous harness-sync CI catches drift; revert-commit is always available on a single-commit migration. |

## 6. Acceptance criteria (2P-1a)

2P-1a is shipped when ALL are true:

1. A specific commit SHA on public `master` (`SHA_A`, recorded in `HANDOFF.md`) contains the migrated layers per T1a inventory.
2. `cargo test --workspace` green at `SHA_A` (≥ 1704 passed, 0 failed).
3. Fresh clone of public `forge` at `SHA_A` on Ubuntu 22.04 + glibc 2.35: `scripts/setup-dev-env.sh && cargo test --workspace` green.
4. Plugin installs into Claude Code from the public repo; opening a Claude Code session triggers `session-start.sh`, daemon logs `register_session`, agent sees at least one `forge-*` skill in context, `forge-next recall "<phase tag>"` returns ≥ 1 memory where `session_id` matches the dogfood session.
5. Private `forge-app` at commit `SHA_A_private ≥ SHA_A` has ONLY: `app/`, `licensing/`, `product/`, `archive/`, `README.md`, `CLAUDE.md`, `LICENSE`, `.git*`, and any build-tool configs needed for Tauri (allowlist documented in T1a inventory). No `plugin.json`, `marketplace.json`, `agents/`, `hooks/`, `skills/`, `templates/`, `Formula/`, `.claude-plugin/`.
6. `forge-app` CI green at `SHA_A_private`.
7. `HANDOFF.md` §Lifted constraints has a new entry dated at ship time noting 2P-1a completion, citing `SHA_A` + `SHA_A_private` + a pointer to the 2P-1b follow-up work.

---

# Phase 2P-1b: Harden (follow-up, scoped separately)

After 2P-1a ships, these items land as their own tracked work (one commit per item, or small bundles). Each is motivated by an adversarial-review finding on v1/v2 and is valuable, but the **move doesn't block on them**.

## Scope (from v2 + both reviews, deferred intact)

1. **Harness-sync CI check** (`scripts/check-harness-sync.sh`): scans JSON method literals + Rust test fixtures (including variable-name forms) + CLI subcommand refs (including flags-before-command and nested forms like `forge-next identity list`) + rustdoc `Request::Variant` refs; cross-checks against `crates/core/src/protocol/request.rs` and `forge-cli`'s clap derive. Warn-only for 2 weeks after first landing (amnesty for existing drift), then fail-closed. Rename vs delete handled via `#[deprecated]` grace-window + opt-out annotation. *(v2 §4.)*
2. **Evidence-gated audit contract** (`docs/superpowers/reviews/*.yaml`): `skill-creator` rubric + inverted-prompt Claude + Codex adversarial pass outputs land as machine-parseable YAML artifacts; `scripts/check-review-artifacts.sh` enforces non-empty `artifacts` array + complete file coverage + `HIGH+CRITICAL == 0` in CI. Runs on every PR that touches `skills/`, `agents/`, `hooks/`. *(v2 §3.)*
3. **SPDX header backfill for JSON** via sidecar `LICENSE-<file>.spdx` or `.claude-plugin/LICENSES.yaml` manifest.
4. **Expanded dogfood matrix**: macOS (latest stable ARM + x86 where available) × marketplace install + symlink install × session-start/session-end/post-edit full cycle × negative test (mid-session daemon kill) × parallel-session test. Results doc at `docs/benchmarks/results/forge-public-resplit-2026-05-XX.md`. *(v2 §7.)*
5. **Rollback playbook** (`docs/operations/2P-1-rollback.md`): covers repo revert + GitHub release-asset revocation + Homebrew bottle revocation + sideloaded-user advisory. Walked through in a tabletop exercise. *(v2 §8/§10.9.)*
6. **Marketplace republication task ownership**: Q6 from v2 — does Anthropic's marketplace require a new submission for the renamed/relicensed plugin, or is a version bump sufficient? Resolved by asking the user or checking Anthropic docs. Explicit task owns publishing the public plugin on the marketplace.
7. **2A-4d interlock mechanism**: any 2A-4d PR touching `crates/core/src/protocol/request.rs` must bump a sync version in `.claude-plugin/plugin.json` and update referenced hooks/skills in the same PR — enforced by the harness-sync CI check once in `fail-closed` mode.
8. **Sideload-user migration note**: short migration guide for anyone who sideloaded the private plugin pre-ban-lift (2026-04-23).
9. **GitHub repo governance**: CODEOWNERS for public repo, dependabot config, branch protection rules, issue templates.
10. **Git tags / releases migration** (if any are worth carrying from forge-app).

## 2P-1b acceptance (sketch, tightened when that phase opens)

- Every item above has landed or has an explicit "won't do, because" entry in a followup tracking doc.
- Harness-sync CI runs in `fail-closed` mode on public master for ≥ 1 week with no false positives.
- Dogfood matrix passes all cells.
- Rollback tabletop exercise completed.
- Marketplace listing confirmed.

---

## Changelog from v2

- **Structural split**: 2P-1a (move) + 2P-1b (harden). 2P-1a is 6 tasks; 2P-1b bundles all the v2 ceremony into a separate phase that operates against in-tree code.
- **§2.2 scrub**: binary-asset scope widened per Codex v2 CRITICAL — added `svg/webp/gif/woff/woff2/ttf/otf/eot/zip/tar/tgz/7z/sqlite/db/wasm` handling (exiftool, strings, archive recursion, sqlite refuse-list).
- **§2.3 SPDX**: conservative — only applied to files that natively support comments. JSON gets sidecar in 2P-1b. Fixes Codex v2 HIGH "SPDX breaks JSON schema validation."
- **T1a freeze**: forge-app source SHA frozen at inventory time; T3a copies from that SHA only. Fixes Codex v2 HIGH "T1 doesn't lock source."
- **§3 review**: prose punch lists, not YAML artifacts, for the move's single review pass. YAML-gated contract lives in 2P-1b where it runs continuously.
- **§4 harness-sync CI**: entirely deferred to 2P-1b. Was v2 §4.
- **§5 risks table**: scoped to 2P-1a. 2P-1b risks live in its own section when that phase opens.
- **§6 acceptance**: 7 concrete criteria, no "some commit X" variables. macOS acceptance moves to 2P-1b's matrix (2P-1a ships on Linux-only, explicitly).
- Task count dropped 16 → 6.

## Open questions (T1a must resolve; NONE carry past T1a)

- **Q1**: Is `forge-graph` MCP still in forge-app? Go/no-go on T3a including it.
- **Q2**: Any `forge-*` skills commercially differentiated? Excluded from T3a if yes.
- **Q3**: Linux/macOS variation strategy for `scripts/install.sh`? (Defer macOS variant to 2P-1b if macOS install isn't needed in T6a.)
- **Q4**: Submodules / Git LFS present in any migrated layer? Decide migrate-with / skip / flatten in T1a.
- **Q5**: Any forge-app GitHub Actions secrets referenced by workflows being migrated? If a workflow moves, document required secrets for public org before enabling.

---

Ready for one more quick adversarial review on the move-only scope (2P-1a), then land T1a.
