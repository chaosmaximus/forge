# Handoff — Post-Compact Continuation (2026-04-24)

**Public HEAD (chaosmaximus/forge):** see `git log -1` at session start.
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (post-2P-1a prune).
**Current version:** **v0.5.0** across all four crates + plugin manifest + marketplace + Homebrew formula (bumped from 0.4.0 at phase close).

## State in one paragraph

Phase **2P-1a (plugin surface migration)** is SHIPPED. The public repo now carries the full agent-facing Forge SDK — plugin manifest, marketplace entry, 3 agent teams, 11 hook scripts, 15 skills + shared reference, Homebrew formula, install.sh, full test suite (BATS unit + bash integration + 9 static validators + Claude Code E2E), daemon-side Hook HealthCheck, CI gating. Public plugin is installed in the user's `~/.claude/plugins/marketplaces/forge-marketplace/` (symlink into the repo); `"forge@forge-marketplace": true` in `~/.claude/settings.json`. **Forge HUD renders live** through `~/.claude/statusline-command.sh` → `target/release/forge-hud`. forge-app has been pruned to its private allowlist (Tauri app, licensing, internal product docs).

Phase **2A-4c2 (Behavioral Skill Inference)** is T1-T8 complete; T9-T11 remain.

Phase **2P-1b (harden)** is a 13-item backlog tracked below in §Lifted constraints.

## First actions after `/compact`

```bash
# Sanity-check environment
cd /mnt/colab-disk/DurgaSaiK/forge/forge
cargo fmt --all --check                                 # should be clean
cargo clippy --workspace -- -W clippy::all -D warnings  # 0 warnings
cargo test --workspace                                  # 1710 passed, 0 failed, 3 ignored

# Verify plugin + HUD still wired
ls -l ~/.claude/plugins/marketplaces/forge-marketplace  # symlink → public repo
echo '{"session_id":"t","model":{"display_name":"Claude Opus 4.7"}}' | bash ~/.claude/statusline-command.sh
```

If all four pass, you're ready to work. If not: see §Environment prerequisites below.

## Next work (pick one)

### Stream A — Finish 2A-4c2 (3 tasks)

Plan: `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md`
Spec: `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md` (LOCKED; do not edit).

- **T9**: Schema rollback recipe test. Follows 2A-4c1 T11 precedent. Lands in `crates/daemon/src/db/schema.rs` or a sibling test file. The 2A-4c2 T1 ALTERs on `skill` table need a rollback recipe that's actually tested.
- **T10**: Adversarial review of T1-T9 pure-daemon diff (Claude code-reviewer + Codex rescue, inverted prompts).
- **T11**: Live-daemon dogfood + results doc at `docs/benchmarks/results/forge-behavioral-skill-inference-2026-04-XX.md`.

Once T11 lands, 2A-4c2 is done and **Phase 2A-4d Forge-Identity Bench** unblocks.

### Stream B — Start 2P-1b (harden, pick any item)

Spec: `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md` §Phase 2P-1b.

Highest-leverage items:
1. **Harness-sync CI check** (`scripts/check-harness-sync.sh`) — the invariant that turns the CLAUDE.md harness philosophy into a testable gate. Scans JSON method literals + Rust fixtures + CLI subcommand refs + rustdoc `Request::<Variant>` against the authoritative list in `crates/core/src/protocol/request.rs` + `forge-cli`'s clap derive. 2-week warn-only window, then fail-closed.
2. **Cut v0.5.0 public release** — `git tag v0.5.0 && git push --tags` triggers `.github/workflows/release.yml` which ships `forge-v0.5.0-x86_64-unknown-linux-gnu.tar.gz` + macOS variants. Once live, `scripts/install.sh` resolves 200 OK and Formula/forge.rb's URL template resolves. Closes latent 2P-1a fragility.
3. **Re-enable `#[ignore]` tests** in `crates/daemon/tests/test_hook_e2e.rs` (`test_session_start_hook_registers_session`, `test_full_pipeline_remember_check_via_cli`) once CI provisions release `forge-next` + running daemon.
4. Full 2P-1b backlog (13 items) in §Lifted constraints below.

### Stream C — Triage orphan plan

`docs/superpowers/plans/2026-04-16-housekeeping-and-doctor-observability.md` — 4 deferred items from Forge-Persist reviews (Version endpoint, HttpClient timeout, session pagination, Doctor-on-steroids). Never executed. Decide: schedule or defer.

## Environment prerequisites

- Ubuntu 22.04 (glibc 2.35) or newer. On this cloud box `scripts/setup-dev-env.sh` downloads Microsoft's manylinux_2_17 ORT 1.23.0 into `.tools/` — needed because `fastembed 5` / `ort 2.0.0-rc.11` default to pyke.io's glibc ≥ 2.38 binary.
- `sudo apt-get install -y pkg-config libssl-dev` (one-time).
- `.cargo/config.toml` + `scripts/with-ort.sh` runner wire ORT env into every cargo invocation transparently — no manual `LD_LIBRARY_PATH` exports needed.
- For running the daemon directly (outside cargo), export `LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib"` before launch.

## Files of interest

| File | Why |
|------|-----|
| `CLAUDE.md` | Project identity, harness philosophy, git workflow rules. |
| `HANDOFF.md` | This file. |
| `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md` | 2A-4c2 locked design. |
| `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md` | 2A-4c2 task plan (T1-T8 done, T9-T11 pending). |
| `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md` | 2P-1 design v3 (2P-1a shipped, 2P-1b pending). |
| `docs/superpowers/plans/2P-1a-inventory.md` | Source of truth for what migrated. |
| `docs/benchmarks/results/forge-public-resplit-2026-04-23.md` | 2P-1a dogfood results (6/7 PASS, 1 WARN, 0 FAIL). |
| `crates/daemon/src/workers/consolidator.rs` | `infer_skills_from_behavior` (T4) + Phase 23 registration (T5). |
| `crates/daemon/src/server/handler.rs` | Request dispatch; doctor Hook check (T6a). |
| `crates/daemon/tests/skill_inference_flow.rs` | 2A-4c2 T8 integration test. |
| `crates/daemon/tests/test_hook_e2e.rs` | Restored hook E2E (2 tests `#[ignore]`'d pending infra). |
| `scripts/migrate-{copy,scrub,scrub-allowlist}.sh` + `migrate-{lexicon,exclude}.txt` | T2a migration tooling. |
| `tests/static/*.sh` | Plugin/hook/skill/agent validators (wired to CI in T4a). |
| `tests/fixtures/scrub/` + `tests/scripts/test-migrate-scrub.sh` | Scrub fixture tests. |

## Lifted constraints (changelog)

- **2026-04-23 — `forge:*` plugin skills ban lifted.** Original rule: "DO NOT invoke any `forge:*` plugin skill or `superpowers:using-git-worktrees` — both corrupted git state via rogue branches / orphan commits." Root cause: the worktrees interaction, not forge plugin. Worktrees now off by default (CLAUDE.md §Git workflow). Plugin rebuilt in 2P-1a.

- **2026-04-23 — Phase 2P-1a COMPLETE at public `b48991e`.** Six migration commits (`3863b24` → `b48991e`) landed the move from `forge-app@480527b`. Dogfood: 6/7 PASS, 1 WARN (by design, 2A-4c2 T7 dual-gate), 0 FAIL. See `docs/benchmarks/results/forge-public-resplit-2026-04-23.md`.

- **2026-04-24 — Phase 2P-1a closed.** `chaosmaximus/forge-app#1` merged (`SHA_A_private = 665c372`). Plugin installed locally via `~/.claude/plugins/marketplaces/forge-marketplace` symlink + `"forge@forge-marketplace": true` in `~/.claude/settings.json` + `extraKnownMarketplaces.forge-marketplace.source.path`. Forge HUD rendering live. All four crates + plugin manifest + marketplace + Formula bumped to **v0.5.0**.

## Phase 2P-1b backlog (harden — 13 items)

1. **Harness-sync CI check** (`scripts/check-harness-sync.sh`). JSON + Rust fixtures + CLI subcommand refs (incl. flags-before-command and `forge-next identity list`-style nested) + rustdoc `Request::<Variant>` refs cross-checked against `crates/core/src/protocol/request.rs` + clap derive. Warn-only 2 weeks then fail-closed. Rename/delete via `#[deprecated]` grace window.
2. **Evidence-gated audit contract** — `docs/superpowers/reviews/*.yaml` artifacts from `skill-creator` + inverted-prompt Claude/Codex passes; `scripts/check-review-artifacts.sh` enforces HIGH+CRITICAL == 0 on every PR touching `skills/`, `agents/`, `hooks/`.
3. **SPDX JSON sidecar** (`.claude-plugin/LICENSES.yaml`) since inline `// SPDX` breaks JSON parse.
4. **Expanded dogfood matrix** — macOS (ARM + x86) × marketplace install + symlink install × session-start / session-end / post-edit × daemon-kill mid-session × parallel-session test.
5. **Rollback playbook** (`docs/operations/2P-1-rollback.md`) — repo revert + GitHub release-asset revocation + Homebrew bottle revocation + sideload-user advisory; tabletop exercise.
6. **Marketplace publication** — resolve Q6 (new submission vs version bump); explicit task owns the Anthropic marketplace listing.
7. **2A-4d interlock** — any PR touching `crates/core/src/protocol/request.rs` bumps a sync version in `.claude-plugin/plugin.json` + updates referenced hooks/skills in the same PR; enforced by harness-sync CI in fail-closed mode.
8. **Sideload-user migration note** — short guide for anyone who sideloaded the private plugin pre-ban-lift.
9. **Repo governance** — CODEOWNERS, dependabot, branch protection, issue templates.
10. **Hook-level bugs** (latent, non-ship-blocking):
    - `scripts/hooks/session-end.sh` — `rm -f SESSION_FILE` before confirming `end-session` succeeded.
    - `scripts/hooks/user-prompt.sh` — advances `SINCE` watermark on empty first-call.
    - `scripts/hooks/subagent-start.sh` — nested XML-escape bug on recall args.
    - `scripts/hooks/post-edit.sh` / `post-bash.sh` — don't call `record_tool_use`; direct API works.
    - `skills/forge/SKILL.md` — docs `cargo install ... forge-cli` but binary is `forge-next`.
    - `Formula/forge.rb` — no `depends_on "rust"`, no `head` stanza; `sha256 "PLACEHOLDER"` needs release-workflow fill.
    - `scripts/install.sh` — no SHA256 checksum verification.
11. **Re-enable `#[ignore]` tests** in `test_hook_e2e.rs` once CI provisions release binary + daemon.
12. **Stale validators** not wired to CI: `validate-csv.sh`, `validate-rubrics.sh`, `validate-templates.sh` (content gaps). Migrate the missing content or retire the validators.
13. **Cut v0.5.0 release** — tag + push triggers `release.yml`; without it, `Formula/forge.rb` and `scripts/install.sh` URL templates 404.

## Process rules

- Work on `master` directly (no feature branches by default).
- **Do not use git worktrees without explicit per-task permission** (CLAUDE.md §Git workflow).
- `cargo fmt --all` clean + `cargo clippy --workspace -- -W clippy::all -D warnings` 0 warnings at every commit boundary.
- `cargo test --workspace` green after each GREEN phase.
- Two adversarial reviews (Claude code-reviewer + Codex rescue) on every design BEFORE implementation AND on every diff BEFORE merge.
- Commit prefixes: `feat(<phase> <task>):`, `fix(...)`, `chore(...)`, `docs(...)`, `test(...)`. Co-author trailer `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`. New commits, never amend.

## What NOT to redo

- 2P-1a — SHIPPED. Don't re-migrate. Hardening is 2P-1b, separate scope.
- 2A-4c2 spec + plan — LOCKED. Don't edit either.
- 2A-4c1, 2A-4b, 2A-4a — SHIPPED. Don't touch.
- Don't re-derive the scrub lexicon, migration tooling, or inventory from scratch — `scripts/migrate-*.sh`, `tests/fixtures/scrub/`, `docs/superpowers/plans/2P-1a-inventory.md` are authoritative.
- Don't create new git branches (master-direct workflow).
