# Handoff — Post-Compact Continuation (2026-04-24)

**Public HEAD (chaosmaximus/forge):** see `git log -1` at session start.
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (post-2P-1a prune).
**Current version:** **v0.5.0** across all four crates + plugin manifest + marketplace + Homebrew formula (bumped from 0.4.0 at phase close).

## State in one paragraph

Phase **2P-1a (plugin surface migration)** is SHIPPED. The public repo now carries the full agent-facing Forge SDK — plugin manifest, marketplace entry, 3 agent teams, 11 hook scripts, 15 skills + shared reference, Homebrew formula, install.sh, full test suite (BATS unit + bash integration + 9 static validators + Claude Code E2E), daemon-side Hook HealthCheck, CI gating. Public plugin is installed in the user's `~/.claude/plugins/marketplaces/forge-marketplace/` (symlink into the repo); `"forge@forge-marketplace": true` in `~/.claude/settings.json`. **Forge HUD renders live** through `~/.claude/statusline-command.sh` → `target/release/forge-hud`. forge-app has been pruned to its private allowlist (Tauri app, licensing, internal product docs).

Phase **2A-4c2 (Behavioral Skill Inference)** is SHIPPED. T9 rollback test, T10 adversarial review + 3-finding hardening, T11 live-daemon dogfood all landed 2026-04-24. Results at `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md`. Phase **2A-4d Forge-Identity Bench** is unblocked.

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

### Stream A — Start 2A-4d Forge-Identity Bench

Now unblocked by 2A-4c2. No spec yet — design phase first.

### Stream B — Start 2P-1b (harden, pick any item)

Spec: `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md` §Phase 2P-1b.

Highest-leverage remaining items (others shipped 2026-04-24):
1. **Cut v0.5.0 public release** — `git tag v0.5.0 && git push --tags` triggers `.github/workflows/release.yml`. Currently blocked on GHA billing. Once that clears: re-tag and push. Closes latent 2P-1a fragility where `Formula/forge.rb` and `scripts/install.sh` URL templates 404.
2. **2P-1b §2 evidence-gated audit contract** — YAML review artifacts + CI enforcement. Bigger design, pick up cold.
3. **2P-1b §16 shape-vs-behavior fingerprint split** — Codex-H3 carry-forward from 2A-4c2 T10. Non-trivial design.
4. **Full 2P-1b backlog** in §Lifted constraints below (current status: 11 of 18 shipped this session).

### Stream C — Triage orphan plan — DONE 2026-04-24

`docs/superpowers/plans/2026-04-16-housekeeping-and-doctor-observability.md` — SHIPPED in the interim. All 4 tasks (Version endpoint, HttpClient timeout, session pagination, Doctor-on-steroids) are live; plan marked SHIPPED with per-task evidence pointers at its top.

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
| `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md` | 2A-4c2 task plan (SHIPPED). |
| `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md` | 2A-4c2 dogfood results at HEAD `90d9b74`. |
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

- **2026-04-24 — Phase 2A-4c2 SHIPPED.** T9 (`f85d15c` rollback recipe test), T10 (`90d9b74` Codex-H1 + H2 + Claude-B1 hardening + 4 regression tests), T11 live-daemon dogfood (results: `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md`). Dogfood produced `<skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [b9f98611]</skill>` at HEAD `90d9b74`. Partial unique index widened from `(agent, fingerprint)` → `(agent, project, fingerprint)` and renamed `idx_skill_agent_project_fingerprint`.

- **2026-04-24 — Hook schema fix.** All 7 Forge hooks that emit `hookSpecificOutput` (pre-bash, pre-edit, post-bash, post-edit, session-start, user-prompt, subagent-start) were missing the required `hookEventName` field. Every invocation emitted a "Hook JSON output validation failed" non-blocking error and dropped `additionalContext` silently. Fixed in `d660562`. Live-verified — PreToolUse:Bash context now flows through as a valid hook event.

- **2026-04-24 — Phase 2P-1b partial: 11 of 18 items shipped.** §1 harness-sync CI (`ab88450`), §3 SPDX sidecar + §8 sideload migration (`0563873`), §5 rollback playbook (`a26ac7a`), §9 CODEOWNERS + dependabot (`d9fda72`), §10 all 5 hook-level bugs (`c61d926` + `030711b` — last also ships `forge-next record-tool-use` + post-bash/post-edit wiring so Phase 23 gets live data), §11 TestDaemon helper un-ignores hook e2e tests (`baea19b`), §12 retired 3 stale validators (`d9fda72`), §14 inferred_from windowed pruning (`f0fccf3`), §15 skills_inferred in ConsolidationComplete + tracing (`6ee6e9f`), §17 json_valid guard (obsoleted by §14). Remaining: §2 audit contract, §4 dogfood matrix (macOS), §6 marketplace, §7 2A-4d interlock (time-gated), §13 v0.5.0 release (billing-blocked), §16 fingerprint split, §18 Phase 23 numbering.

- **2026-04-24 — Stream C closed.** Orphan `housekeeping-and-doctor-observability` plan triaged: all 4 tasks were shipped in the interim. Plan marked SHIPPED with per-task evidence pointers.

## Phase 2P-1b backlog (harden — 18 items)

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
14. **Phase 23 `inferred_from` windowed pruning** (Codex-MED from T10) — currently monotonic across runs. Recompute `inferred_from` from the current window each run, or move observations into a separate table and derive count/windowed set at read time.
15. **Expose `skills_inferred` in `ConsolidationComplete` response + structured tracing** (Codex-LOW from T10) — today only `eprintln!`, so Phase 23 is invisible to request-level telemetry and to the forthcoming 2A-4d bench.
16. **Shape-vs-behavior fingerprint split** (Codex-H3 from T10) — current fingerprint hashes tool names + arg KEYS only, so `Read{"/tmp/a"}` and `Read{"/prod/secret"}` collide. Future: normalize value features (`file_path`, `cmd`, URL host) into the hash, or maintain shape vs behavior fingerprints.
17. **Defensive `WHERE json_valid(skill.inferred_from)` on UPDATE merge** (Claude-H2 from T10) — malformed JSON in `inferred_from` would error the json_each subquery. Column DEFAULT is `'[]'` + all writers emit valid JSON, so edge case is manual corruption only.
18. **Phase-number vs orchestrator-position alignment** (Claude-H3 from T10) — Phase 23 runs between Phase 17 and 18 in `run_consolidation`. Rename to Phase 17.5 / 17a OR extend `PHASE_ORDER` const to list every phase's fn_name in actual run order so the probe API is honest.

## Process rules

- Work on `master` directly (no feature branches by default).
- **Do not use git worktrees without explicit per-task permission** (CLAUDE.md §Git workflow).
- `cargo fmt --all` clean + `cargo clippy --workspace -- -W clippy::all -D warnings` 0 warnings at every commit boundary.
- `cargo test --workspace` green after each GREEN phase.
- Two adversarial reviews (Claude code-reviewer + Codex rescue) on every design BEFORE implementation AND on every diff BEFORE merge.
- Commit prefixes: `feat(<phase> <task>):`, `fix(...)`, `chore(...)`, `docs(...)`, `test(...)`. Co-author trailer `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`. New commits, never amend.

## What NOT to redo

- 2P-1a — SHIPPED. Don't re-migrate. Hardening is 2P-1b, separate scope.
- 2A-4c2 — SHIPPED. Spec + plan LOCKED. Carry-forward items live in 2P-1b §14-18.
- 2A-4c1, 2A-4b, 2A-4a — SHIPPED. Don't touch.
- Don't re-derive the scrub lexicon, migration tooling, or inventory from scratch — `scripts/migrate-*.sh`, `tests/fixtures/scrub/`, `docs/superpowers/plans/2P-1a-inventory.md` are authoritative.
- Don't create new git branches (master-direct workflow).
