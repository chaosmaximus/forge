# 2026-04-28 — Adversarial review: harness-sync gate (Phase 6, 37bd99a)

## Verdict
NEEDS-FOLLOWUP — gate is currently green and fixture coverage is solid, but two
latent risks were uncovered: (a) `SKIP_CLI_TOKENS` masks four high-risk
fictional surfaces that are *not* real subcommands (`memory`, `session`,
`skill`, `plugin`), and (b) the suffixed-form regex captures only the first
token after `forge-next`/`forge-cli`, so `forge-next sync push` would
mis-extract `sync` (not a real subcommand) and create a false-positive when
that pattern eventually lands in a skill/agent body. No live drift today.

## 1. Adversarial regex inputs

Pattern under test:
`(^|[^a-zA-Z0-9/:._-])forge[[:space:]]+[a-z][a-z-]+`

| # | Input | Result | Captured | Correct? |
|---|---|---|---|---|
| 1 | `` `forge scan` to scan secrets `` | MATCH | `` `forge scan `` | yes |
| 2 | `␣␣␣␣forge research how it works` (4-sp indent) | MATCH | ` forge research` | yes |
| 3 | `echo "forge ship now"` | MATCH | `"forge ship` | yes |
| 4 | `https://github.com/org/forge/blob/main/README.md` | NO-MATCH | — | yes (slash skip) |
| 5 | `Use /forge:foo skill` | NO-MATCH | — | yes (slash+colon skip) |
| 6 | `See :forge: tag` | NO-MATCH | — | yes (colon skip) |
| 7 | `forge recall starts the line` (BOL) | MATCH | `forge recall` | yes |
| 8 | `- forge build the project` (after dash) | MATCH | ` forge build` | yes (`-` is in skip class but the leading char is the space before `forge`, so OK) |
| 9 | `(forge query the daemon)` (after paren) | MATCH | `(forge query` | yes |
| 10 | `...forge plan ahead` (after period) | NO-MATCH | — | **NO — blind spot.** `.` is in the negative class to exempt `pkg.forge`, but it also exempts a sentence-final period followed by `forge`. Low likelihood in practice but a documented gap. |
| 11 | `\|forge sync now` (after pipe) | MATCH | `\|forge sync` | yes |
| 12 | `forge-next sync push --remote x` (multi-token positional) | MATCH (suffixed regex) | `forge-next sync` | **partial — see §4.** `awk '{print $NF}'` returns `sync`, which is NOT a real subcommand (`sync-pull`/`sync-push` exist). Would silently create a false-positive if such a line lands in skills/agents. No occurrences today. |
| 13 | `**forge plan**` | MATCH | `*forge plan` | yes |
| 14 | `pkg.forge plan` | NO-MATCH | — | yes |
| 15 | `org_forge research` | NO-MATCH | — | yes (underscore not in negative class — wait, `_` IS in `a-zA-Z0-9` class, so it's correctly excluded) |

Action: document #10 as known limitation; consider tightening #12 to capture
the second positional token or whitelist multi-positional commands.

## 2. SKIP_CLI_TOKENS audit

`crates/cli/src/main.rs` checked for each token (real subcommand-name
match: explicit `#[command(name = "...")]` OR a kebab-cased Commands
variant exactly equal to the token).

| Token | Real subcommand? | Verdict |
|---|---|---|
| binary | no | OK (prose) |
| cli | no | OK (prose) |
| agent | YES (line 779) | safe — skip is no-op since it would pass the cli-list check anyway, but entry is dead/noise |
| agents | YES (line 786) | same as `agent` — dead skip |
| daemon | YES (line 187, `Daemon`) | same — dead skip |
| daemons | no | OK (prose) |
| plugin | no | **RISK** — masks fictional `forge plugin install` etc.; CC plugin docs make this likely drift |
| plugins | no | **RISK** — same as `plugin` |
| skill | no | **RISK** — masks `forge skill list`/`forge skill run` style fiction |
| skills | YES (skills-list/skills-install/...) — but bare `skills` is NOT a subcommand (only `skills-list` etc.) | **RISK** — `forge skills` (without `-list`) would be silently skipped; common drift |
| team | YES (line 809) | dead skip |
| teams | no | OK (prose) |
| memory | no (only in help banner) | **RISK** — masks `forge memory recall` style fiction (memory is a known fictional surface in past skill drift) |
| memories | no | **RISK** — same |
| context | no | **RISK** — masks `forge context show`-style fiction |
| contexts | no | **RISK** — same |
| session | no | **RISK** — masks `forge session start`-style fiction |
| sessions | YES (line 270) | dead skip |

Action for fix-wave: drop `plugin/plugins/skill/skills/memory/memories/context/contexts/session` from `SKIP_CLI_TOKENS` (or split into `SKIP_PROSE_ONLY` checked only when the captured `$sym` is followed by sentence punctuation). Keep `agent/agents/daemon/team/sessions` (harmless) but add a comment noting they're real subcommands so removal is also fine. The four real high-risk fictions all involve nouns that ARE plugin-domain words and ARE plausible drift surfaces.

## 3. Live state (verbatim)

`bash scripts/check-harness-sync.sh 2>&1 | tail -3`:

```
harness-sync: OK — 158 JSON methods + 108 CLI subcommands authoritative, no drift
```

`bash tests/scripts/test-harness-sync.sh 2>&1 | tail -8`:

```
  PASS — exit 1
  PASS — contains 'drift entries detected'
  PASS — contains 'invented-subcmd'
  PASS — contains 'nonexistent-cmd'
Test 4: drift fixture, legacy FORCE_FAIL=1
  PASS — exit 1

harness-sync fixture tests: 9 passed, 0 failed
```

## 4. Prose tweaks justification

All three "before" forms match the regex (`forge deploy`, `forge handoff`,
`forge prereqs`) and none of `deploy`/`handoff`/`prereqs` are real
subcommands or in SKIP_CLI_TOKENS, so each would have flagged drift.
Tweaks were necessary.

| File | Before-form match (capture last token) | After-form | Mechanism | Less-disruptive alternative? |
|---|---|---|---|---|
| README.md | `forge deploy` | `helm install forge ./deploy/helm/` | leading `.` after the space breaks the `[a-z]` lookahead | adding `deploy` to SKIP_CLI_TOKENS would also work but pollutes the list; `./` is cleaner and arguably more correct shell prose |
| forge-handoff/SKILL.md | `forge handoff` | `forge-handoff` (hyphen instead of space) | suffixed form excluded from bare regex | could fence in a `<example>` non-shell block, but the rename makes the prose more accurate (it's a skill name not a CLI verb) |
| forge-setup/SKILL.md | `forge prereqs` | `Forge prereqs` (capital F) | uppercase `F` doesn't match the literal `forge` in the pattern | acceptable; matches the project's brand-noun style ("Forge state files" etc.) |

All three changes are low-cost (single token edits) and arguably improve the
prose semantics independent of the gate. No less-disruptive alternative is
clearly better. Verdict on §4: justified.

## Summary for fix-wave

1. (HIGH) Prune `SKIP_CLI_TOKENS`: drop `plugin/plugins/skill/skills/memory/memories/context/contexts/session` — these mask plausible fictional drift surfaces and only `skills` has any real subcommand connection (and only as a prefix, not a bare command).
2. (LOW) Either tighten the suffixed regex to capture the full positional run for multi-token commands like `forge-next sync push`, or document the limitation; no current harness file triggers it.
3. (DOC) Add a comment in the script noting the after-period blind spot (input #10) so future maintainers don't think `pkg.forge plan` exemption was free.
