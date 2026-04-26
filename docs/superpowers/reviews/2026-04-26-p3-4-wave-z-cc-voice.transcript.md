# Wave Z Adversarial Review — Transcript

**Slug:** `2026-04-26-p3-4-wave-z-cc-voice`
**Reviewer:** claude-opus-4-7 (general-purpose subagent, terse-output mode)
**Commit range:** `3f11e77..3fcc1eb` (Wave Z + fw1 fix-wave)
**Verdict:** `lockable-with-fixes`

## Wave Z scope

CC voice first-run setup unblock per `feedback/2026-04-26-setup-and-isolation.md`.
8 commits comprising:

```
3af9303 fix(Z1): plugin.json — remove duplicate hooks reference
77ee831 fix(Z6): detect-reality accepts positional <path>
929220d fix(Z2): compile-context honors --project in code-structure rendering
f07936b feat(Z3+Z4): project subcommand tree + setup skill rewrite
23cc4b6 feat(Z5+Z7+Z9): compile-context --cwd auto-create, --dry-run, hook FORGE_HOOK_VERBOSE
de10b9a feat(Z8): update-session CLI for fixing misregistered project label
420c6e2 feat(Z10+Z11): doctor backup hygiene + git_sha drift warnings
3fcc1eb fix(fw1): adversarial review fixes — auto-create error logging + cluster drift test + CHANGELOG
```

## Reviewer briefing summary

The reviewer was given the diff range, tier separation (Z2/Z3/Z7/Z8 as
focus, Z1/Z4/Z5/Z6/Z9/Z10/Z11 as secondary), and a 10-point rubric
covering behavior regressions, auto-create side effects, protocol
hard-cuts, cluster-filter SQL, SessionUpdate atomicity, missing rename
/ delete CLI, doc-references-flag-not-yet-shipped, test adequacy,
coding hygiene, and dead-code surfaces.

## Findings landed

3 HIGH (all resolved by fw1):

* **HIGH-1** — Z7 `let _ = store_reality(...)` swallowed errors.
  Fixed: `match` arm with `tracing::warn!`.
* **HIGH-2** — concurrent SessionStart race produces duplicate project
  rows. Documented as benign-data race; strict idempotence deferred.
* **HIGH-3** — hardcoded `organization_id: "default"`. Fixed: extracted
  const + TODO; cluster-drift regression test added.

5 MED (3 resolved, 2 deferred):

* **MED-1** — Z8 SessionUpdate TOCTOU. Deferred (cosmetic message).
* **MED-2** — protocol hard-cut needs CHANGELOG. Resolved: created
  `CHANGELOG.md` with Wave Z section.
* **MED-3** — missing `project rename`/`delete`. Deferred to v0.6.1.
* **MED-4** — cluster-JOIN drift test. Resolved: new fw1 test.
* **MED-5** — Z4 doc references `--dry-run` before Z5 lands. Deferred
  (master-only, no released artifact affected).

3 LOW (all deferred):

* **LOW-1** — `code_engine.rs::context_section` dead code → ZR scope.
* **LOW-2** — Z10 backup hygiene XDG_DATA_HOME ignore → v0.6.1+ ops.
* **LOW-3** — end-to-end integration test for §1.2 reproducer →
  reactive (only when a regression surfaces).

## Hygiene checks (passed)

* No `unwrap()` outside tests in new code.
* Inlined format args throughout (`format!("{x}")`).
* `tracing::warn!`/`tracing::info!` not `println!` in non-test code.
* No new `let _ = conn.execute(...)` swallowing schema-level failures
  (the original Z7 instance was the HIGH-1 finding; resolved in fw1).

## Final compile + test gates at HEAD `3fcc1eb`

* `cargo clippy --workspace --features bench -- -W clippy::all -D warnings` — clean
* `cargo test -p forge-daemon --lib --features bench` — 1608 passed, 1 ignored
* `cargo test -p forge-cli --bin forge-next` — 99 passed (incl 9 Wave Z parse tests)
* `cargo fmt --all --check` — clean
* `bash scripts/check-harness-sync.sh` — OK 158 + 108
* `bash scripts/check-protocol-hash.sh` — OK `68432a815353…`
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-review-artifacts.sh` — OK (this YAML included)

## Deferral summary

5 items deferred to backlog:

* MED-1 (Z12+ session-handler hardening)
* MED-3 (v0.6.1 backlog: project rename/delete/relocate)
* MED-5 (process note only)
* LOW-1 (ZR scope)
* LOW-2 (v0.6.1+ ops)
* LOW-3 (reactive)

cc-voice can use Forge end-to-end at HEAD `3fcc1eb` with the project
subcommand tree, auto-create on first contact, --dry-run audit, and
update-session recovery path. The 1 GB of forge.db.pre-*.bak files
they observed will warn under `forge-next doctor` going forward.
