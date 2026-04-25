# Handoff — Phase P3-1 closed at v0.6.0-rc.1 (2026-04-25)

**Public HEAD (pre-close commit):** `55c693d`.
**Forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (unchanged).
**Version:** **v0.6.0-rc.1** (bumped from v0.5.0 at this close).
**Plan:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`
**Halt:** awaiting user sign-off before opening Phase P3-2.

## State in one paragraph

This session executed all 8 waves of Phase P3-1 (2P-1b harness
hardening) under the autonomous-mode authorization granted on
2026-04-25, plus the formal phase close. **W1** built
`scripts/check-harness-sync.sh` enhancements (auto-amnesty timer,
`FORGE_HARNESS_SYNC_ENFORCE` env var, `--root` flag for fixture
testing) + drift fixtures + a 7-assertion bash test runner. **W2**
shipped the evidence-gated YAML audit contract (`docs/superpowers/
reviews/<slug>.yaml` schema v1, Python validator, 12-assertion
fixture suite, CI gate that refuses open BLOCKER/HIGH findings).
**W3** rewrote `.claude-plugin/LICENSES.yaml` as a coverage manifest
declaring SPDX licenses for every shipped JSON file (now 3:
plugin.json, marketplace.json, hooks.json) + a 25-assertion
validator. **W4** added the 2A-4d interlock: SHA-256 of
`crates/core/src/protocol/request.rs` mirrored as `protocol_hash` in
plugin.json, with a portable Python+regex sync helper and 18-assertion
round-trip tests. **W5** ran the 2P-1 rollback tabletop dry-run,
caught a substring-collision bug in the playbook's `pkill -f
'forge-daemon'` step (replaced with pidfile + SIGINT — the daemon
only handles SIGINT, not SIGTERM), and surfaced 5 gaps. **W6**
landed GitHub repo governance: CODEOWNERS extended to W1-W5
surfaces, three issue templates + a PR template, and a
`.github/pending-rollback` CI guard wired as the FIRST step of the
check job (closes W5 §G1). **W7** added a user-side sideload-state
detector with 15-assertion fixture coverage. **W8** ran a 14-cell
multi-OS dogfood matrix — 6 cells PASS on Linux, 8 cells handed off
(3 release-blocked, 2 by-design negative, 3 macOS-best-effort). The
phase close bumped all 7 version anchors (4 Cargo.toml + Formula +
plugin.json + marketplace.json), backfilled YAMLs for W2-W8 in the
review-artifacts tree, and verified all 11 CI gates green.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -25                              # most recent at top
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 154 + 107, no drift
bash scripts/check-review-artifacts.sh             # 8 reviews valid
bash scripts/check-license-manifest.sh             # 3 files, coverage clean
bash scripts/check-protocol-hash.sh                # in sync (9a38d781…)
bash tests/scripts/test-harness-sync.sh            # 7/0
bash tests/scripts/test-review-artifacts.sh        # 12/0
bash tests/scripts/test-license-manifest.sh        # 25/0
bash tests/scripts/test-protocol-hash.sh           # 18/0
bash tests/scripts/test-sideload-state.sh          # 15/0
bash tests/static/run-shellcheck.sh                # all PASS
bash scripts/ci/check_spans.sh                     # OK
cargo fmt --all --check                            # clean
```

## Phase P3-1 commits (most recent first)

| #   | SHA          | Wave  | Title |
|-----|--------------|-------|-------|
| 18  | _next_       | close | docs(P3-1 close): v0.6.0-rc.1 + W2-W8 review backfill + HANDOFF |
| 17  | `55c693d`    | W8-fix | fix(P3-1 W8 review): table/transcript drift + macOS setup guard + quoting |
| 16  | `47d0da4`    | W8 | docs(P3-1 W8): multi-OS dogfood matrix — 2026-04-25 |
| 15  | `5a26af7`    | W7-fix | fix(P3-1 W7 review): source.repo fixture + flag-eat guard + scope comment |
| 14  | `b10d1ed`    | W7 | feat(P3-1 W7): sideload-state detector + Linux/macOS migration notes |
| 13  | `49ec70a`    | W6-fix | fix(P3-1 W6 review): bug template fields + PR template scope + guard ordering |
| 12  | `92e65a3`    | W6 | feat(P3-1 W6): GitHub repo governance — templates + CODEOWNERS + rollback CI guard |
| 11  | `2a5ed8c`    | W5-fix | fix(P3-1 W5 review): SIGINT not SIGTERM + drill checklist semantics |
| 10  | `1fab0d6`    | W5 | docs(P3-1 W5): rollback playbook tabletop drill — 2026-04-25 |
|  9  | `f683e55`    | W4-fix | fix(P3-1 W4 review): portable sync helper + real round-trip test coverage |
|  8  | `7d3dd2f`    | W4 | feat(P3-1 W4): 2A-4d interlock — plugin.json protocol_hash gate |
|  7  | `35808c4`    | W3-fix | fix(P3-1 W3 review): tighter SPDX validation + reference existence + diagnostics |
|  6  | `82db6f0`    | W3 | feat(P3-1 W3): SPDX sidecar manifest + coverage validator |
|  5  | `f9971e2`    | W2-fix | fix(P3-1 W2 review): yaml-load robustness + path traversal + status-coupled fields |
|  4  | `ed6950e`    | W2 | feat(P3-1 W2): evidence-gated YAML audit contract |
|  3  | `beeb2be`    | W1-fix | fix(P3-1 W1 review): trap leak + --root guard + assertion + CI order |
|  2  | `c261e99`    | W1 | feat(P3-1 W1): harness-sync auto-amnesty + drift fixtures + tests |
|  1  | `442a9b4`    | plan | docs(P3): complete production-readiness plan — autonomous drain |
| (carryover) | `1862a43` | — | docs(2A-4d): close W5-W8 — HANDOFF rewrite + final state |

17 P3-1 commits + plan + carryover.

## What shipped — by wave

### W1 — harness-sync hardening (`c261e99` + `beeb2be`)

* Auto-amnesty self-flip: WARN before 2026-05-09, FAIL on/after.
* `FORGE_HARNESS_SYNC_ENFORCE={0,1}` env var (canonical) +
  `FORCE_FAIL=1` legacy alias.
* `--root <dir>` for fixture testing.
* `FORGE_HARNESS_SYNC_MIN_REQUEST` / `FORGE_HARNESS_SYNC_MIN_CLI`
  thresholds for fixture trees with synthetic 6-variant enums.
* `tests/fixtures/harness-sync/{clean,drift}` + 7 assertions in
  `tests/scripts/test-harness-sync.sh`.
* CI integration (fixture-test step ordered BEFORE the real-repo
  drift check after the W1 review caught the masking).
* W1 review fixes: trap leak (early-exit unbound-var on
  parser-regression branch), `--root <flag>` swallows next flag,
  loose `assert_contains "no drift"` precision, CI step ordering.

### W2 — evidence-gated YAML audit contract (`ed6950e` + `f9971e2`)

* Schema v1 documented in `docs/superpowers/reviews/README.md`.
* `scripts/check_review_artifacts.py` — Python+PyYAML validator.
  Asserts schema_version, target_paths exist + repo-contained,
  artifacts non-empty, every BLOCKING-severity finding (BLOCKER/
  CRITICAL/HIGH) is resolved or deferred (no open).
* W2 review fixes: ValueError catch on bad date scalars
  (PyYAML constructs `datetime.date`), path-traversal containment
  guard, status-coupled field requirements (`resolved` →
  `fixed_by`, `deferred` → `rationale`), unknown-key warnings.
* 12 assertions across clean + 9 drift YAMLs.
* First real artifact backfilled for W1 in
  `2026-04-25-w1-harness-sync.yaml` + `.transcript.md`.

### W3 — SPDX sidecar manifest (`82db6f0` + `35808c4`)

* `.claude-plugin/LICENSES.yaml` rewritten as v1 schema with
  `coverage_paths` + repo-relative paths. Now declares 3 JSONs
  (plugin.json + marketplace.json + hooks/hooks.json — the latter
  was previously uncovered).
* `scripts/check_license_manifest.py` validator: SPDX expression
  tokenizer, coverage-walk, containment guard, references
  existence check.
* W3 review fixes: tightened SPDX regex (rejects whitespace-only,
  free-form prose, dangling operators), schema_version diagnostic
  prints type, references[] existence enforced, WARN on missing
  coverage_paths, top-level Exception catch.
* 25 assertions across clean + 8 drift fixtures.

### W4 — 2A-4d interlock (`7d3dd2f` + `f683e55`)

* SHA-256 of `crates/core/src/protocol/request.rs` mirrored as
  `protocol_hash` in plugin.json (initial: `9a38d781…`).
* `scripts/check_protocol_hash.py` validator + bash wrapper.
* `scripts/sync-protocol-hash.sh` refresh helper — Python+re.subn
  rather than sha256sum+sed for cross-platform (W4 review HIGH-1)
  + multi-line layout robustness (HIGH-3) + `--root` for testability
  (HIGH-2).
* 18 assertions including a real sync round-trip + multi-line
  layout regression test.

### W5 — rollback tabletop drill (`1fab0d6` + `2a5ed8c`)

* `docs/operations/rollback-drills/2026-04-25-tabletop.md` — full
  drill log with per-step status + 5 discovered gaps.
* `docs/operations/2P-1-rollback.md` updated:
  - Step 4 daemon shutdown rewritten to use `$FORGE_DIR/forge.pid`
    + `kill -INT` (W5 review HIGH-1: daemon only handles SIGINT,
    not SIGTERM; default `kill PID` would skip the socket-drain).
    Strategic fix (add SIGTERM handler in main.rs) tracked in
    plan-doc backlog.
  - Step 0 caveat that the `.github/pending-rollback` flag needs
    enforcement (G1 deferred to W6 — landed via CI guard).
  - Step 1 `--cleanup-tag` syntax cleanup.
  - Tabletop checklist split into "paper drill" (this run) vs
    "full drill (annual)" cadences.

### W6 — GitHub repo governance (`92e65a3` + `49ec70a`)

* `.github/CODEOWNERS` extended to cover W1-W5 surfaces +
  meta-protect the .github/ tree itself.
* 3 issue templates (bug, feature, rollback) + PR template with
  scope checklist, test-plan mirroring CI's exact commands, and
  conditional adversarial-review section ("(P3 waves only)").
* `.github/workflows/ci.yml` Pending-rollback flag guard (closes
  W5 §G1) — W6 review moved it to the FIRST step of the check job
  so a rollback freeze can't be masked by harness-sync failures.
* W6 review fixes: bug template field source (`doctor` not
  `health`), PR template review scope, guard ordering, guard
  message hint, rollback.md `git push --force-with-lease` typo,
  semver placeholder `v<bad-version>`, clippy command alignment,
  feature_request mentions sync-protocol-hash.sh.

### W7 — sideload-state detector (`b10d1ed` + `5a26af7`)

* `scripts/check-sideload-state.sh` — bash + python3 user-side
  detector. Reads `~/.claude/settings.json`, reports forge-app /
  forge-private references in enabledPlugins or
  extraKnownMarketplaces source.path/repo.
* Inline `_has_private_fragment()` helper unifies plugin + market
  scans (refactor pulled in via W7 review M1+M2 fixes).
* `docs/operations/sideload-migration.md` + Auto-detection
  section, Platform notes (Linux/macOS quit semantics with W5 §G3
  pkill caveat).
* 15 assertions across clean + 5 drift fixtures (incl. the
  source.repo branch added in W7 review M1 fix).

### W8 — multi-OS dogfood matrix (`47d0da4` + `55c693d`)

* `docs/benchmarks/results/2P-1b-dogfood-matrix.md` — 14-cell
  matrix.
  - 6 PASS on Linux: source-build cycle (health/doctor/recall/
    stats/realities) + sideload (running daemon validates the path).
  - 8 cells handed off: 3 release-blocked (cargo install, brew,
    tarball), 2 by-design negative (mid-session kill + parallel
    sessions — both with full reproduction steps), 3 macOS
    (best-effort per locked decision #2).
* W8 review fixes: 171→174 transcript drift, macOS
  `setup-dev-env.sh` is Linux-only guard, shell-quoting in
  user-reproduction snippets, hook WARN dual-case explanation,
  acceptance precision (6/14 PASS, 8 handoff with reproduction).

## Tests + verification (final state at v0.6.0-rc.1)

* `cargo fmt --all --check` — clean
* `bash scripts/ci/check_spans.sh` — OK (23 names matched)
* `bash tests/static/run-shellcheck.sh` — all 19 scripts PASS
* **5 fixture-test runners (77 total assertions, all PASS):**
  - `test-harness-sync.sh` — 7/0
  - `test-review-artifacts.sh` — 12/0
  - `test-license-manifest.sh` — 25/0
  - `test-protocol-hash.sh` — 18/0
  - `test-sideload-state.sh` — 15/0
* **4 real-repo gates (all PASS):**
  - harness-sync — 154 methods + 107 subcommands, no drift
  - review-artifacts — 8 review YAMLs valid, no open blockers
  - license-manifest — 3 files declared, coverage clean
  - protocol-hash — request.rs ↔ plugin.json synced (`9a38d781…`)
* **8 review YAMLs in `docs/superpowers/reviews/`** covering W1-W8;
  every BLOCKER/HIGH/MEDIUM resolved or deferred with rationale.

## Deferred backlog — single source of truth

`docs/superpowers/plans/2026-04-25-complete-production-readiness.md`
§"P3-1 deferred backlog" — currently 21 items across W1-W8 reviews,
all classified resolved/deferred per their wave's review YAML. None
block P3-2 entry.

Highlight items worth surfacing here:

* **Daemon SIGTERM handler** — strategic fix for the W5 HIGH-1
  finding. Requires `tokio::signal::unix::signal(SignalKind::terminate())`
  in `crates/daemon/src/main.rs`. Until landed, `systemctl stop` and
  any default `kill PID` skip graceful shutdown. Track for P3-2 or
  earlier.
* **W5 §G4: pre-migration DB snapshot** — DB compatibility matrix
  in the rollback playbook flags this as a real production-safety
  hole when rolling back across schema boundaries.
* **W5 §G5: quarterly drill cadence reminder** — no automated
  reminder mechanism; documented in the playbook checklist.

## P3-1 → P3-2 transition

**Halt-and-ask point per locked decision #5:** before opening P3-2,
user reviews:

1. The 17 P3-1 commits (this HANDOFF table).
2. The 8 review YAMLs and the consolidated transcript at
   `docs/superpowers/reviews/2026-04-25-p3-1-w2-w8.transcript.md`.
3. The deferred backlog tail in the plan doc.
4. The acceptance bullets in
   `docs/benchmarks/results/2P-1b-dogfood-matrix.md` for any
   handoff cells worth running on macOS / against a release.

**Phase P3-2 scope (queued, not started):** 6 waves of 2A-4d
follow-up — Tier 3 M3 protocol change (`Request::CompileContextTrace`
gains `session_id`), Tier 3 M2 (batch `resolve_scoped_config`),
Tier 1 #5 (T10 OTLP-path latency variant), Tier 1 #2 (record() span
scope refactor — 22 sites), Tier 3 #5 (`shape_bench_run_summary`
CTE rewrite), Tier 3 #6 cosmetic batch. Close at v0.6.0-rc.2.

The harness-sync CI gate (W1) and protocol-hash interlock (W4) are
specifically designed to guard P3-2's first wave (the protocol
change touching `request.rs`). When P3-2 W1 lands, both gates
should fire as expected — the contributor must run `bash scripts/
sync-protocol-hash.sh` after editing the Request enum.

## Known quirks (P3-1)

* `test_daemon_state_new_is_fast` — pre-existing timing flake
  (since 2P-1a). Unchanged.
* `gh release delete --cleanup-tag=false` (legacy form) is accepted
  by gh CLI but its `--help` only shows bare `--cleanup-tag`. The
  W5-updated playbook now uses the bare form for clarity.
* The harness-sync amnesty auto-flips to fail-closed on 2026-05-09
  via the script's `date -u` check — no CI workflow edit needed at
  that date. Test contributor environments where `date` returns
  non-UTC may behave unexpectedly (locked to `-u` so this should
  hold).

## One-line summary

P3-1 closed: 17 commits, 8 waves of 2P-1b harness hardening (each
shipped + adversarially reviewed + fix-committed), v0.6.0-rc.1
across 8 version anchors, 11 CI gates green (4 real-repo + 5
fixture-runners + shellcheck + spans), 8 review YAMLs validated
zero open blocking findings. Halt for user sign-off before P3-2.
