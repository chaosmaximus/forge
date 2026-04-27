# Pre-Release Audit — Tracking Ledger (147 findings)

**Sources:** 6 parallel audits at `docs/superpowers/audits/2026-04-27-zr-close-pre-release-{A,B,C,D,E,F}-*.yaml`.
**Synthesis:** `docs/superpowers/audits/2026-04-27-pre-release-audit-synthesis.md`.

**Status legend:**
- ✅ **fixed** — landed in commit
- 🟡 **deferred** — moved to v0.6.1 / backlog with rationale
- ❌ **false positive** — agent claim was wrong; no fix needed
- ⏳ **pending** — not yet addressed; queued for next session

**Tally:**

| | CRITICAL | HIGH | MED | LOW | NIT | Total |
|---|---|---|---|---|---|---|
| ✅ fixed       | 5 | 14 | 1 | 0 | 0 | **20** |
| ❌ false-pos   | 1 | 0 | 0 | 0 | 0 | **1** |
| 🟡 deferred    | 0 | 1 | 0 | 0 | 0 | **1** |
| ⏳ pending     | 0 | 16 | 54 | 40 | 15 | **125** |
| **Total**      | **6** | **31** | **55** | **40** | **15** | **147** |

---

## Audit A — Docs vs reality (20 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **DOCS-A-001** | HIGH | ⏳ pending | Docs reference fake `forge` binary (secret scanning) | `docs/getting-started.md:223` |
| **DOCS-A-002** | HIGH | ⏳ pending | Memory types `fact`/`entity`/`skill` don't match `MemoryType` enum | `docs/getting-started.md:86` |
| **DOCS-A-003** | HIGH | ⏳ pending | Wrong Unix socket filename (`daemon.sock` vs `forge.sock`) | `docs/api-reference.md:19` |
| **DOCS-A-004** | HIGH | ⏳ pending | security.md says gRPC is "(Planned)" — actually shipped | `docs/security.md:212` |
| **DOCS-A-005** | MED | ⏳ pending | Quick-Start example shows `0.4.0` (current `0.6.0-rc.3`) | `docs/getting-started.md:67` |
| **DOCS-A-006** | MED | ⏳ pending | Endpoint count `98` stale (158 variants) | `README.md:15` |
| **DOCS-A-007** | MED | ⏳ pending | Worker count `8` stale (10 + skill_inference) | `README.md:16` |
| **DOCS-A-008** | MED | ⏳ pending | Test counts in README + CONTRIBUTING wildly stale | `README.md:14` |
| **DOCS-A-009** | MED | ⏳ pending | cli-reference.md still surfaces `reality.*` config keys post-ZR | `docs/cli-reference.md:507` |
| **DOCS-A-010** | MED | ⏳ pending | `task_completion_check` request undocumented in api-reference + flag drift | `docs/api-reference.md` |
| **DOCS-A-011** | MED | ⏳ pending | Quick-Start cargo install string wrong / installs nothing | `README.md:41` |
| **DOCS-A-012** | MED | ⏳ pending | Marketplace.json claims "Auto-installs on first use" while marketplace deferred | `.claude-plugin/marketplace.json:6` |
| **DOCS-A-013** | MED | ⏳ pending | agent-development.md memory-types same drift as DOCS-A-002 | `docs/agent-development.md:123` |
| **DOCS-A-014** | MED | ⏳ pending | `Phase 2A-4d.2` internal phase tags leak into user-facing docs | `docs/cli-reference.md:769` |
| **DOCS-A-015** | LOW | ⏳ pending | operations.md worker list omits `reaper` + `kpi_reaper` | `docs/operations.md:31` |
| **DOCS-A-016** | LOW | ⏳ pending | "Migration note for 0.5.x" reads as live advice but is historical | `docs/operations.md:385` |
| **DOCS-A-017** | LOW | ⏳ pending | README mixed `localhost:8420` vs socket claims without clarifying default | `README.md:63` |
| **DOCS-A-018** | LOW | ⏳ pending | `recall --layer identity` — `identity` is not a Manas layer | `docs/cli-reference.md:40` |
| **DOCS-A-019** | LOW | ⏳ pending | Cargo install installs `forge-bench` silently as third binary | `README.md:41` |
| **DOCS-A-020** | NIT | ⏳ pending | agent-development.md `grpc://` URL scheme — verify CLI accepts it | `docs/agent-development.md:81` |

---

## Audit B — CLI feature completeness (24 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **B-HIGH-1** | HIGH | ✅ Phase 2 (`12e0466`) | `register-session --role` parses but discards | `crates/cli/src/commands/system.rs:814` |
| **B-HIGH-2** | HIGH | ✅ Phase 2 | `team create --parent` parses but discards | `crates/cli/src/commands/teams.rs:310` |
| **B-HIGH-3** | HIGH | ✅ Phase 2 | `cleanup-sessions --older-than` silent unwrap_or(0) | `crates/cli/src/main.rs:1703` |
| **B-HIGH-4** | HIGH | ✅ Phase 2 | `recall --since` silent unwrap_or(0) | `crates/cli/src/main.rs:1530` |
| **B-HIGH-5** | HIGH | ✅ Phase 2 | `version` advertised in --help; not a subcommand | `crates/cli/src/main.rs:41` |
| **B-MED-1** | MED | ⏳ pending | 5 Skills + 2 Meeting handlers print raw `{:?}` Debug | `crates/cli/src/main.rs:2299` |
| **B-MED-2** | MED | ⏳ pending | Multibyte UTF-8 byte-slice panic hazards (4 sites) | `commands/system.rs:1125; sync.rs:378; teams.rs:103,715` |
| **B-MED-3** | MED | ✅ Phase 2 (partial) | COMMAND_CATEGORIES drops `init`/`consolidate` | `crates/cli/src/main.rs:38` |
| **B-MED-4** | MED | ⏳ pending | `export --format ndjson` silent no-op | `crates/cli/src/commands/system.rs:285` |
| **B-MED-5** | MED | ⏳ pending | No CLI surface for `Request::UpdateAgentTemplate` | `crates/cli/src/commands/teams.rs` |
| **B-MED-6** | MED | ⏳ pending | No CLI surface for `Request::ListTeamTemplates` | `crates/cli/src/commands/teams.rs` |
| **B-MED-7** | MED | ⏳ pending | `record-tool-use` silently re-encodes malformed JSON | `crates/cli/src/commands/system.rs:851` |
| **B-MED-8** | MED | ⏳ pending | `act-notification --id X` (no --approve/--reject) silently rejects | `crates/cli/src/main.rs:2337` |
| **B-LOW-1** | LOW | ⏳ pending | `Observe` doc-comment lists shapes incompletely | `crates/cli/src/main.rs:1075` |
| **B-LOW-2** | LOW | ⏳ pending | `forge-next init` derives project name from basename, not marker | `crates/cli/src/commands/system.rs:3017` |
| **B-LOW-3** | LOW | ⏳ pending | `--confidence`, `--strength` accept any f64 (no range validation) | `crates/cli/src/main.rs:148` |
| **B-LOW-4** | LOW | ⏳ pending | `send --kind`, `respond --status` accept any string | `crates/cli/src/main.rs:439` |
| **B-LOW-5** | LOW | ⏳ pending | `team run --topology X` accepts any string | `crates/cli/src/main.rs:1351` |
| **B-LOW-6** | LOW | ⏳ pending | `context-refresh --since X` passes raw to daemon | `crates/cli/src/main.rs:891` |
| **B-LOW-7** | LOW | ⏳ pending | `agent-template create --identity-facets '{...}'` JSON not pre-validated | `crates/cli/src/commands/teams.rs:7` |
| **B-LOW-8** | LOW | ⏳ pending | `subscribe` has no graceful-shutdown story | `crates/cli/src/main.rs:863` |
| **B-LOW-9** | LOW | ⏳ pending | 32 `Request::*` variants have no CLI surface | `crates/cli/src/main.rs` |
| **B-NIT-1** | NIT | ⏳ pending | `import` slurps file into memory | `crates/cli/src/commands/system.rs:325` |
| **B-NIT-2** | NIT | ⏳ pending | `restart` magic 6s sleep | `crates/cli/src/main.rs:1955` |

---

## Audit C — Dead code + redundancy (23 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **C-HIGH-1** | HIGH | ⏳ pending | `crates/daemon/src/migrate.rs` (150 LOC) is dead production code | `crates/daemon/src/migrate.rs` |
| **C-HIGH-2** | HIGH | ⏳ pending | `teams::create_meeting_with_voting` (40 LOC) zero callers | `crates/daemon/src/teams.rs:1217-1256` |
| **C-MED-1** | MED | ⏳ pending | `extraction::router::check_quality_guard` unreferenced | `crates/daemon/src/extraction/router.rs:276-317` |
| **C-MED-2** | MED | ⏳ pending | Wrapper-triplet rot in `recall.rs` (6 pub fns test-only) | `crates/daemon/src/recall.rs:142,204,2228,907,565` |
| **C-MED-3** | MED | ⏳ pending | Wrapper-triplet rot in `consolidator.rs` (10 pub fns test-only) | `crates/daemon/src/workers/consolidator.rs` |
| **C-MED-4** | MED | ⏳ pending | `ProjectEngine` trait has only 1 impl + 1 mock — premature abstraction | `crates/core/src/types/project_engine.rs:60-76` |
| **C-MED-5** | MED | ⏳ pending | `db::diagnostics::expire_diagnostics` (8 LOC) test-only | `crates/daemon/src/db/diagnostics.rs:83-90` |
| **C-MED-6** | MED | ⏳ pending | `notifications::count_pending` + `proactive::should_surface` test-only | `notifications.rs:266; proactive.rs:375` |
| **C-MED-7** | MED | ⏳ pending | `lsp::regex_python::extract_imports_python` + go duplicate `lsp::symbols` | `lsp/regex_python.rs:147; lsp/regex_go.rs:112` |
| **C-LOW-1** | LOW | ⏳ pending | `hud::render::colors::security_color` + `ratio_color` zero callers | `crates/hud/src/render/colors.rs:13-30` |
| **C-LOW-2** | LOW | ⏳ pending | `lsp::client::file_uri` premature alias for `path_to_file_uri` | `crates/daemon/src/lsp/client.rs:589-591` |
| **C-LOW-3** | LOW | ⏳ pending | `cli::transport::Transport::is_http` `#[allow(dead_code)]` | `crates/cli/src/transport.rs:60-63` |
| **C-LOW-4** | LOW | ⏳ pending | Stale TODOs in handler.rs claim org_id threading incomplete (it's done) | `crates/daemon/src/server/handler.rs:655,912,1195,3497,3758` |
| **C-LOW-5** | LOW | ⏳ pending | `config::RealityConfig` Rust struct + `[reality]` TOML key (post-ZR vocabulary) | `crates/daemon/src/config.rs:181,830-850` |
| **C-LOW-6** | LOW | ⏳ pending | `bench::longmemeval` stale TODO comments for Consolidate + Hybrid modes | `crates/daemon/src/bench/longmemeval.rs:13-14` |
| **C-LOW-7** | LOW | ⏳ pending | `consolidator.rs:2528` carries `TODO(2A-4+): migrate to ops::supersede_memory_impl()` | `crates/daemon/src/workers/consolidator.rs:2528` |
| **C-LOW-8** | LOW | ⏳ pending | LSP client 4× `#[allow(dead_code)]` on JSON-RPC envelope fields | `crates/daemon/src/lsp/client.rs:77,103,112,114` |
| **C-NIT-1** | NIT | ⏳ pending | `find_project_dir_candidate_for_test` cfg(test) makes allow redundant | `workers/indexer.rs:389-414` |
| **C-NIT-2** | NIT | ⏳ pending | `TEST_RSA_PUBLIC_KEY` const annotated `#[allow(dead_code)]` | `server/auth.rs:436-445` |
| **C-NIT-3** | NIT | ⏳ pending | `SubscribeParams.token` `#[allow(dead_code)]` | `server/http.rs:330-332` |
| **C-NIT-4** | NIT | ⏳ pending | `PtySession.master` `#[allow(dead_code)]` | `server/pty.rs:19-22` |
| **C-NIT-5** | NIT | ⏳ pending | `find_project_dir_candidate_for_test` 3-arg signature mirrors prod logic | `workers/indexer.rs:391-414` |
| **C-NIT-6** | NIT | ⏳ pending | `extract_call_edges_regex` could be `pub(crate)` | `workers/indexer.rs:1194` |

---

## Audit D — Harness sync (25 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **D-01** | HIGH | ⏳ pending | forge-security advertises `forge scan` (doesn't exist) | `skills/forge-security/SKILL.md` |
| **D-02** | HIGH | ⏳ pending | forge-research advertises `forge research` | `skills/forge-research/SKILL.md` |
| **D-03** | HIGH | ⏳ pending | forge-ship references `forge verify .` and `forge test run` | `skills/forge-ship/SKILL.md` |
| **D-04** | HIGH | ⏳ pending | forge-think + forge-evaluator reference `forge query` Cypher endpoint | `skills/forge-think/SKILL.md` + `agents/forge-evaluator.md` |
| **D-05** | HIGH | ⏳ pending | forge-review references `forge review .` | `skills/forge-review/SKILL.md` |
| **D-06** | HIGH | ⏳ pending | Agents reference `data/` and `evaluation-criteria/` dirs that don't exist | agent files |
| **D-07** | HIGH | ⏳ pending | harness-sync gate misses bare `forge X` invocations | `scripts/check-harness-sync.sh` |
| **D-08** | HIGH | ⏳ pending | forge-setup advertises Stitch MCP via `.mcp.json` (doesn't exist) | `skills/forge-setup/SKILL.md` |
| **D-09** | HIGH | ⏳ pending | forge-setup uses wrong slash syntax `/forge:new` (should be `/forge:forge-new`) | `skills/forge-setup/SKILL.md` |
| **D-10** | HIGH | ⏳ pending | 3 skills invoke fictional `TaskCreate` tool (CC tool is `TodoWrite`) | various |
| **D-11** | MED | ⏳ pending | `skills/forge-build-workflow.md` no front-matter — skipped by loader | `skills/forge-build-workflow.md` |
| **D-12** | MED | ⏳ pending | harness-sync gate doesn't scan `agents/*.md` content | `scripts/check-harness-sync.sh` |
| **D-13** | MED | ⏳ pending | forge-generator references bare `forge recall` | `agents/forge-generator.md` |
| **D-14** | MED | ⏳ pending | forge-think uses bare `forge build` and `forge plan` placeholders | `skills/forge-think/SKILL.md` |
| **D-15** | MED | ⏳ pending | marketplace.json owner.email empty; plugin.json author no email | `.claude-plugin/marketplace.json` + `plugin.json` |
| **D-16** | MED | ⏳ pending | LICENSES.yaml doesn't declare licenses for skills/ or agents/ markdown | `.claude-plugin/LICENSES.yaml` |
| **D-17** | MED | ⏳ pending | forge-verify hardcodes `cargo clippy -p forge-daemon -p forge-core -p forge-cli` | `skills/forge-verify/SKILL.md` |
| **D-18** | MED | ⏳ pending | FORGE_HOOK_VERBOSE undocumented outside session-start.sh comment | n/a |
| **D-19** | LOW | ⏳ pending | hooks.json uses `${CLAUDE_PLUGIN_ROOT}/scripts/hooks/...` but plugin.json doesn't expose hooks dir | `hooks/hooks.json` |
| **D-20** | LOW | ⏳ pending | All 9 hook scripts hardcode `forge-next` with no fallback to `forge-cli` | hook scripts |
| **D-21** | LOW | ⏳ pending | forge top-level skill description 100+ words (CC skill triggers should be short) | `skills/forge/SKILL.md` |
| **D-22** | LOW | ⏳ pending | forge-research promises "git checkpoint" but no impl | `skills/forge-research/SKILL.md` |
| **D-23** | LOW | ⏳ pending | agents/forge-evaluator.md hardcodes Codex `gpt-5.2` model | `agents/forge-evaluator.md` |
| **D-24** | NIT | ⏳ pending | Plugin.json description sentence-fragment style | `.claude-plugin/plugin.json` |
| **D-25** | NIT | ⏳ pending | check-harness-sync.sh threshold defaults stale (MIN_REQUEST=50, real 158) | `scripts/check-harness-sync.sh` |

---

## Audit E — DB schema + migration + recovery (25 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **E-1** | CRITICAL | ✅ Phase 1 (`386d32f`) | PRAGMA foreign_keys=OFF in canonical helper | `db/pragma.rs:85-113` |
| **E-2** | CRITICAL | ✅ Phase 1 | sync_export soft-scope reintroduces W29 leak | `crates/daemon/src/sync.rs:242-248` |
| **E-3** | CRITICAL | ❌ false positive | "ZR migration has no regression test" — agent missed `zr_c3_*` tests | n/a |
| **E-4** | CRITICAL | ✅ Phase 1 | store_project INSERT OR REPLACE data-loss | `db/ops.rs:2939-2952` |
| **E-5** | HIGH | ✅ Phase 3 (`b8f7fb9`) | memory_vec.store_embedding doesn't validate dim | `db/vec.rs:27-35` |
| **E-6** | HIGH | ✅ Phase 3 | audit_log triggers via `let _ = conn.execute_batch(...)` | `db/schema.rs:1490-1502` |
| **E-7** | HIGH | ✅ Phase 3 | register_session INSERT OR REPLACE wipes lifecycle columns | `crates/daemon/src/sessions.rs:70-103` |
| **E-8** | HIGH | 🟡 deferred v0.6.1 | code_search `LIKE '%pattern%'` no covering index | `crates/daemon/src/server/handler.rs:5252-5347` |
| **E-9** | HIGH | ✅ Phase 3 | kpi_reaper missing composite (event_type, timestamp) index | `workers/kpi_reaper.rs:98-126` |
| **E-10** | MED | ⏳ pending | Read-only requests bypass writer-actor's audit_log insert | `server/{http.rs:180-203, writer.rs:307-338}` |
| **E-11** | MED | ⏳ pending | Wave Z (Z10) backup is health-check, not pruner | `server/handler.rs:1795-1838` |
| **E-12** | MED | ⏳ pending | session WHERE agent+project+status no covering index | `crates/daemon/src/sessions.rs:274-281` |
| **E-13** | MED | ⏳ pending | Edge dedup migration via `let _ = conn.execute_batch(...)` | `db/schema.rs:1652-1682` |
| **E-14** | MED | ⏳ pending | sync_import unchecked_transaction + conn.execute (not tx.execute) | `sync.rs:418-552` |
| **E-15** | MED | ⏳ pending | memory.organization_id backfill via `let _ = conn.execute(...)` | `db/schema.rs:986-992` |
| **E-16** | MED | ⏳ pending | code_file pollution DELETE migrations via `let _ = conn.execute(...)` | `db/schema.rs:1134-1148` |
| **E-17** | MED | ⏳ pending | list_symbols silently coerces NULL line_start to 0 | `db/ops.rs:1479-1495` |
| **E-18** | MED | ⏳ pending | `notification.reality_id` post-ZR vocabulary leak in wire shape | `db/schema.rs:1402` |
| **E-19** | MED | ⏳ pending | SkillsRegistry FTS bootstrap uses `unwrap_or(false)` on probe | `db/schema.rs:1566-1593` |
| **E-20** | LOW | ⏳ pending | Manas 8-layer claim — layer boundaries enforced by convention only | `db/schema.rs:505-625` |
| **E-21** | LOW | ⏳ pending | kpi_events_retention reaper — no end-to-end retention test | `workers/kpi_reaper.rs:267-464` |
| **E-22** | LOW | ⏳ pending | Read-only HTTP path inlines PRAGMA literal | `server/handler.rs:214-222` |
| **E-23** | LOW | ⏳ pending | Project struct's `metadata` is String not serde_json::Value | `crates/core/src/types/entity.rs:40-55` |
| **E-24** | LOW | ⏳ pending | raw_chunks_vec dim asymmetry — verify writer asymmetry documented | `db/schema.rs:252-275` |
| **E-25** | NIT | ⏳ pending | Inline migration comments duplicate W1.3 LOW-* tags but no canonical migration log | `db/schema.rs` throughout |

---

## Audit F — Observability + first-run UX (30 findings)

| ID | Sev | Status | Summary | File |
|----|-----|--------|---------|------|
| **F-CRITICAL-1** | CRITICAL | ✅ Phase 1 | First-run BROKEN — `~/.forge/` not pre-created | `crates/cli/src/client.rs:143` |
| **F-CRITICAL-2** | CRITICAL | ✅ Phase 1 | `observe --shape row-count` permanently broken — Inspect not in is_read_only | `crates/daemon/src/server/writer.rs:70` |
| **F-HIGH-1** | HIGH | ✅ Phase 4 (`5432641`) | Grafana label-key drift (phase_name vs phase, etc.) | `deploy/grafana/forge-operator-dashboard.json:29` |
| **F-HIGH-2** | HIGH | ✅ Phase 4 | 9 alert runbooks reference non-existent CLI commands | `docs/operations/runbooks/*.md` |
| **F-HIGH-3** | HIGH | ✅ Phase 4 | /metrics + /inspect HTTP unreachable on default install | `crates/daemon/src/config.rs:246` |
| **F-HIGH-4** | HIGH | ✅ Phase 4 | "8-layer Manas" headline vs 11 layers via row-count | `crates/daemon/src/server/metrics.rs:559` |
| **F-HIGH-5** | HIGH | ✅ Phase 4 | docker compose monitor profile hard-fails (missing prometheus.yml) | `deploy/docker-compose.yml:46` |
| **F-MED-1** | MED | ⏳ pending | RPATH bakes onnxruntime-1.23.0 paths — won't survive install relocation | `.cargo/config.toml` / `crates/daemon/build.rs` |
| **F-MED-2** | MED | ⏳ pending | macOS install error suggests wrong binary name `forge-cli` | `scripts/install.sh:34` |
| **F-MED-3** | MED | ⏳ pending | Operator dashboard hardcodes `"datasource": "Prometheus"`/SQLite (no template) | `deploy/grafana/forge-operator-dashboard.json:25` |
| **F-MED-4** | MED | ⏳ pending | docs/observability/otlp-validation.md references `forge-next service restart` | `docs/observability/otlp-validation.md:34` |
| **F-MED-5** | MED | ⏳ pending | Inspect handler's lazy-refresh from 2A-4d.2.1 #1 — dead code on prod socket path (CRIT-2 subsumed) | `server/handler.rs:7669` |
| **F-MED-6** | MED | ⏳ pending | `forge-next plugin install/uninstall` doesn't exist (audit asked) | `crates/cli/src/main.rs` |
| **F-MED-7** | MED | ⏳ pending | compile-context self-reports '9 layers'; README says '8' | `commands/system.rs` (compile-context render) |
| **F-MED-8** | MED | ⏳ pending | doctor warns about no embeddings on fresh DB but no fix hint | `crates/daemon/src/server/handler.rs:1769` |
| **F-MED-9** | MED | ⏳ pending | OTLP silently disables when FORGE_OTLP_ENDPOINT empty | `crates/daemon/src/main.rs:158` |
| **F-MED-10** | MED | ⏳ pending | `observe --shape row-count` shows misleading `(no rows)` on stale | `commands/observe.rs:372` |
| **F-MED-11** | MED | ⏳ pending | `forge_worker_healthy` only ever 1 — `ForgeWorkerDown` alert untriggerable | `crates/daemon/src/server/metrics.rs:544` |
| **F-MED-12** | MED | ⏳ pending | Default config doesn't auto-create `~/.forge/config.toml` template | `config/default.toml` |
| **F-LOW-1** | LOW | ⏳ pending | ForgeMetrics doc-comment claims 7 families; impl registers 13 | `crates/daemon/src/server/metrics.rs:3` |
| **F-LOW-2** | LOW | ⏳ pending | Daemon log mixes JSON + bracket-prefix lines | `crates/daemon/src/main.rs` |
| **F-LOW-3** | LOW | ⏳ pending | `service install` flow but no top-level `forge-next quickstart` | `crates/cli/src/main.rs` |
| **F-LOW-4** | LOW | ⏳ pending | Backup-hygiene threshold (1 GB / 5 files) tuned to one user; may be aggressive | `crates/daemon/src/server/handler.rs:1817` |
| **F-LOW-5** | LOW | ⏳ pending | `/healthz` and `/readyz` exist but undocumented | `crates/daemon/src/server/http.rs:556` |
| **F-LOW-6** | LOW | ⏳ pending | HUD render shows `Forge v0.6.0-rc.3` baked-in (not dynamic) | `crates/hud/src/render/mod.rs` |
| **F-LOW-7** | LOW | ⏳ pending | `/inspect bench-run-summary` 180-day window undocumented | `crates/daemon/src/server/inspect.rs:31` |
| **F-LOW-8** | LOW | ⏳ pending | Operator dashboard claims 5 metric families; targets 4 | `deploy/grafana/forge-operator-dashboard.json:15` |
| **F-NIT-1** | NIT | ⏳ pending | `observe` clap help wraps inconsistently | `crates/cli/src/commands/observe.rs` |
| **F-NIT-2** | NIT | ⏳ pending | docker-compose healthcheck uses `curl -sf` but Dockerfile may not install curl | `deploy/docker-compose.yml:28` |
| **F-NIT-3** | NIT | ⏳ pending | manas-health output literal column widths uneven | `commands/system.rs` (manas_health render) |

---

## Resolution roadmap

**Phases 1-4 done** (this session):
- 5 CRITICAL ✅ (E-1, E-2, E-4, F-CRITICAL-1, F-CRITICAL-2)
- 14 HIGH ✅ (5 CLI + 4 DB + 5 obs/UX)
- 1 false positive (E-3)

**Phases 5-7 queued** (next session):
- **Phase 5** — Dead code (2 HIGH): C-HIGH-1, C-HIGH-2
- **Phase 6** — Harness drift (10 HIGH): D-01..D-10
- **Phase 7** — Docs (4 HIGH): DOCS-A-001..A-004

**Phase 8 — Adversarial review on Phases 1-4 + 5-7 combined diff** (Plan A §6 mandatory).

**Phase 9 — Fix-wave** for review findings.

**Phase 10 — MED batch** (54 remaining): grouped by domain — DB MEDs (10), CLI MEDs (8), obs MEDs (12), docs MEDs (10), C MEDs (7), D MEDs (8), F-MED-5 already subsumed by F-CRITICAL-2.

**Phase 11 — LOW + NIT triage** (40 LOW + 15 NIT = 55): each item reviewed for "fix vs document" — some are intentional design choices (E-20 Manas convention, E-24 dim split is documented).

**Phase 12 — HANDOFF rewrite + halt for #101 release stack.**

## Cross-cutting deferrals (already documented)

These are items the audits flagged but were already documented as deferrals in `docs/operations/v0.6.0-pre-iteration-deferrals.md`:

- **E-8** — FTS5 over code_symbol (perf, not correctness)
- **C-LOW-5** — `config::RealityConfig` post-ZR vocabulary (entry #11 in deferrals)
- **C-LOW-7** — `consolidator.rs:2528` TODO(2A-4+) supersede_memory_impl
- **E-22** — Read-only HTTP path PRAGMA literal (entry #12)

Anything not on the deferrals list goes through Phases 5-12 above.
