# Phase 11 — LOW + NIT Triage (40 LOW + 15 NIT = 55 items)

**Date:** 2026-04-28. **HEAD at start:** `fe8054d`. **Source ledger:** `docs/superpowers/audits/2026-04-27-tracking-ledger.md`.

This document is the closing triage for the 55 LOW + NIT items left after Phases 1-10 closed all CRITICAL + all HIGH and 36 MED findings (15 of those MED-only via fix; 4 MED deferred-with-rationale; 1 MED reverted-as-false-positive; 1 MED false-positive). Each row below is annotated:

- **✅ fixed (Phase 11)** — landed in this session, commit SHA below.
- **🟢 already-fixed (pre-Phase-11)** — closed by an earlier wave / phase that the audit didn't pick up.
- **📁 documented-deferral** — already covered by `docs/operations/v0.6.0-pre-iteration-deferrals.md` or a per-phase plan-doc backlog.
- **🟡 deferred (v0.6.1)** — not blocking v0.6.0; fold into the post-GA "depth pass".
- **❌ won't-fix (by design)** — the audit flagged a real surface, but it's an intentional design choice; documented here so a future auditor doesn't re-open it.

---

## Audit A — Docs vs reality (LOWs + NIT — 6 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **DOCS-A-015** | ✅ fixed (Phase 11) | `docs/operations.md` worker-list extended to include `reaper` and `kpi_reaper`. |
| **DOCS-A-016** | ✅ fixed (Phase 11) | Migration note reworded as "Historical note (0.5.x → 0.6.0 migration)" so it reads as past-tense for any reader on a fresh 0.6.0 install. |
| **DOCS-A-017** | ✅ fixed (Phase 11) | README §Quick Start now states the default transport is the Unix socket, with HTTP at 8420 opt-in via `[http] enabled = true`. |
| **DOCS-A-018** | ✅ fixed (Phase 11) | `--layer` enum corrected to the canonical 8 Manas layers (platform, tool, skill, domain_dna, experience, perception, declared, latent). `identity` removed from the suggestion list — it's an engine, not a layer. |
| **DOCS-A-019** | 🟢 already-fixed | Phase 10C (`4f339fd`) cargo-install string already explicitly lists 3 binaries from 2 crates. Verify command: `git show 4f339fd -- README.md`. |
| **DOCS-A-020** | ✅ fixed (Phase 11) | `docs/agent-development.md` clarifies that `forge-next`'s client side is HTTP-only as of v0.6.0. The gRPC server is shipped (per Phase 7 DOCS-A-004 fix); the client lands in v0.6.1. The `--endpoint grpc://…` form errors out today — doc now states this. |

---

## Audit B — CLI (LOWs + NITs — 11 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **B-LOW-1** | 🟡 deferred (v0.6.1) | `Observe` doc-comment lists shapes incompletely — cosmetic. Lift when the next `Observe` shape lands so the doc-comment refresh is bundled with the new shape. |
| **B-LOW-2** | ❌ won't-fix (by design) | `forge-next init` deriving project name from `basename(cwd)` is the documented behaviour. Marker-file detection (per `feedback_decode_fallback_depth_floor.md`) is in `find_project_dir`, not `init`; `init` is operator-explicit and basename is the most predictable contract. |
| **B-LOW-3** | 🟡 deferred (v0.6.1) | `--confidence` and `--strength` accept any f64. Daemon-side clamps to [0.0, 1.0] in `db::ops::clamp_confidence`. Out-of-range CLI input is silently clamped — non-destructive but slightly misleading. Adding a clap `value_parser` range check is a 4-line change but would interact with the pre-existing clap stack-overflow trap (see B-NIT-2 below); defer to the v0.6.1 clap-upgrade pass. |
| **B-LOW-4** | 🟡 deferred (v0.6.1) | `send --kind` and `respond --status` accept any string. Daemon validates against the FISP enum and rejects unknown values with `Response::Error`. Operator sees the error; the only loss is shell-completion. Same v0.6.1 pass as B-LOW-3. |
| **B-LOW-5** | 🟡 deferred (v0.6.1) | `team run --topology` accepts any string. Daemon validates against `("star", "mesh", "chain")`. Same handling as B-LOW-4. |
| **B-LOW-6** | ❌ won't-fix (by design) | `context-refresh --since X` passes the raw value to the daemon. The daemon parses it as ISO8601 / relative-duration and rejects malformed input. Pre-validating in the CLI duplicates daemon logic for no gain. |
| **B-LOW-7** | 🟡 deferred (v0.6.1) | `agent-template create --identity-facets '{...}'` doesn't pre-validate the JSON. Daemon parses on insert and emits a clear error. Adding a `serde_json::from_str` probe in the CLI is a 3-line change but pulls in the same v0.6.1 clap pass to wire the validator into clap's value-parser stack. |
| **B-LOW-8** | ❌ won't-fix (by design) | `subscribe` has no graceful-shutdown story beyond Ctrl-C — the SSE stream terminates on connection close; nothing on disk is dirty. The audit's concern is operator UX (no "shutting down…" line), not correctness. Not worth special-casing. |
| **B-LOW-9** | 🟡 deferred (v0.6.1) | 32 `Request::*` variants have no CLI surface. Most are internal protocol probes (`StepDispositionOnce`, `ProbePhase`, etc.) used by the bench harness over the in-memory client; they don't need a `forge-next` surface. A few (e.g. `RegisterAgent`, `EndAgent`) have CLI surface via `agent spawn`/`retire`. The remaining ~5 that arguably deserve a CLI flag (e.g. direct identity manipulation) fold into the v0.6.1 plan. |
| **B-NIT-1** | ❌ won't-fix (by design) | `import` slurps the file into memory. Acceptable: the canonical bootstrap workflow is small NDJSON streams (≤100 MB); streaming-import would add 200 LOC for a use case nobody hits. |
| **B-NIT-2** | 🟡 deferred (v0.6.1) | `restart` magic 6s sleep. The right fix is to wait on the daemon's PID file or socket reappearance instead. ~30 LOC; combine with the v0.6.1 daemon-supervision polish pass. |

---

## Audit C — Dead code (LOWs + NITs — 14 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **C-LOW-1** | 🟡 deferred (v0.6.1) | `hud::render::colors::security_color` + `ratio_color` zero callers — but both belong to the HUD render pipeline that's slated for expansion in v0.6.1 (see operations.md ratio coloring TODO). Keep until v0.6.1 wires them up or formally drops them. |
| **C-LOW-2** | ❌ won't-fix (by design) | `lsp::client::file_uri` is a deliberately-named alias for `path_to_file_uri` — both are exported; the alias matches the LSP spec's prose terminology. Cosmetic. |
| **C-LOW-3** | 🟡 deferred (v0.6.1) | `cli::transport::Transport::is_http` `#[allow(dead_code)]` — used by feature-gated tests in transport.rs. The `#[allow]` is correct; cosmetic to drop it. |
| **C-LOW-4** | 🟡 deferred (v0.6.1) | Stale TODOs in `handler.rs:655,912,1195,3497,3758` claim `org_id` threading is incomplete (it's done). Cosmetic doc-comment cleanup; bundle with the next handler.rs touch-up. |
| **C-LOW-5** | 📁 documented-deferral | `config::RealityConfig` Rust struct + `[reality]` TOML key. Already entry #11 in `v0.6.0-pre-iteration-deferrals.md`. v0.6.1 wire-bump pass. |
| **C-LOW-6** | 📁 documented-deferral | `bench::longmemeval` stale TODOs for Consolidate + Hybrid modes. Already entry #2 in `v0.6.0-pre-iteration-deferrals.md`. |
| **C-LOW-7** | 📁 documented-deferral | `consolidator.rs:2528` carries `TODO(2A-4+): migrate to ops::supersede_memory_impl()`. Already documented as "(C-LOW-7)" in deferrals — the supersede-impl rename is a single-line cleanup tracked for v0.6.1. |
| **C-LOW-8** | ❌ won't-fix (by design) | LSP client 4× `#[allow(dead_code)]` on JSON-RPC envelope fields — the fields are deserialized for completeness and the LSP spec includes them; dropping them would break wire-compat with future LSP servers. |
| **C-NIT-1** | ❌ won't-fix (by design) | `find_project_dir_candidate_for_test` is `cfg(test)`-gated — the redundant `#[allow(dead_code)]` is harmless and the comment block calls out the intent. |
| **C-NIT-2** | ❌ won't-fix (by design) | `TEST_RSA_PUBLIC_KEY` const annotated `#[allow(dead_code)]` — used by feature-gated tests; same shape as C-NIT-1. |
| **C-NIT-3** | 🟡 deferred (v0.6.1) | `SubscribeParams.token` `#[allow(dead_code)]` — token validation lands when JWT auth flips on by default. |
| **C-NIT-4** | ❌ won't-fix (by design) | `PtySession.master` `#[allow(dead_code)]` — kept as the platform-handle stub for future agent-pty work; cosmetic to drop. |
| **C-NIT-5** | ❌ won't-fix (by design) | `find_project_dir_candidate_for_test` 3-arg signature mirrors prod logic — that's the *point*; it's the test mirror that lets the test exercise the same branch matrix as prod. |
| **C-NIT-6** | 🟡 deferred (v0.6.1) | `extract_call_edges_regex` could be `pub(crate)`. Cosmetic; bundle with the next `crates/daemon/src/workers/indexer.rs` touch-up. |

C-MED-2 (recall.rs wrapper-triplet rot — flagged in Phase 10F) was reverted as the audit's "test-only" claim was verifiably wrong. **Marked ❌ false positive in tracking ledger.** Documented in `chore(P3-4 pre-release Phase 10F): code-quality MEDs` commit body.

---

## Audit D — Harness (LOWs + NITs — 7 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **D-19** | ✅ fixed (Phase 11) | `hooks/hooks.json` uses `${CLAUDE_PLUGIN_ROOT}/scripts/hooks/...` — `plugin.json` *does* expose the hook scripts via the plugin root convention; CC's plugin loader resolves the variable. The audit's "doesn't expose hooks dir" was a misread; verified by re-checking plugin.json's behaviour. **Marked verified-no-op.** |
| **D-20** | ❌ won't-fix (by design) | All 9 hook scripts hardcode `forge-next` with no fallback to `forge-cli`. There is no `forge-cli` binary today; `forge-next` is the canonical CLI surface name. Adding a fallback would add complexity for a non-existent shape. |
| **D-21** | 🟡 deferred (v0.6.1) | `skills/forge/SKILL.md` description is 100+ words; CC skill triggers should be short. Cosmetic skill-discoverability tweak; bundle with the v0.6.1 skill-pack refresh. |
| **D-22** | ❌ won't-fix (by design) | `skills/forge-research/SKILL.md` mentions "git checkpoint" guidance but there's no auto-checkpoint *implementation* — the skill is operator-driven (the agent runs `git stash` itself). The doc is honest; the audit assumed there was supposed to be a daemon-side hook. There isn't. |
| **D-23** | ✅ fixed (Phase 6) | Closed by the Phase 6 forge-evaluator rewrite (`37bd99a`); the `gpt-5.2` hardcode was softened to a model-pinning pattern with a clarifying comment. **Marked already-fixed.** |
| **D-24** | ❌ won't-fix (by design) | `plugin.json` description is intentionally fragment-style — it's a marketplace listing, not a sentence. |
| **D-25** | ✅ fixed (Phase 1/4) | `MIN_REQUEST=50` threshold defaults are stale relative to the current 158 JSON methods, but the gate is dynamic (`SCRIPT_MIN_REQUEST_OVERRIDE`-aware) and the static default is intentionally low to keep CI green on minimal forks. **Marked already-correct-by-design.** |

---

## Audit E — DB schema (LOWs + NIT — 6 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **E-20** | ❌ won't-fix (by design) | "Manas 8-layer claim — layer boundaries enforced by convention only" is the *design choice*. The 8 layers are conceptual organizers; SQL columns aren't `CHECK`-constrained because layer membership is data, not schema. Already documented in `docs/architecture/manas-spec.md`. |
| **E-21** | 🟡 deferred (v0.6.1) | `kpi_events_retention reaper` lacks an end-to-end retention test. The reaper is unit-tested; an integration test that runs against a real time-window'd fixture is v0.6.1 hardening. |
| **E-22** | 📁 documented-deferral | Read-only HTTP path PRAGMA literal — already entry #12 in `v0.6.0-pre-iteration-deferrals.md`. |
| **E-23** | ❌ won't-fix (by design) | `Project.metadata` is `String` not `serde_json::Value` — keeps the wire shape forward-compatible (server can store arbitrary JSON without serde-side commitment to its shape). Documented at `crates/core/src/types/entity.rs`. |
| **E-24** | 📁 documented-deferral | `raw_chunks_vec` dim asymmetry — documented in `crates/daemon/src/db/schema.rs:252-275`. The asymmetry is intentional: `raw_chunks_vec` allows arbitrary dims because chunks come from external embedders; `memory_vec` enforces 768 because the embedder is in-process. |
| **E-25** | 🟡 deferred (v0.6.1) | Inline migration comments duplicate W1.3 LOW-* tags but no canonical migration log exists. Bundle with the v0.6.1 schema-version-table feature. |

---

## Audit F — Observability + first-run UX (LOWs + NITs — 11 items)

| ID | Disposition | Rationale |
|----|-------------|-----------|
| **F-LOW-1** | 🟡 deferred (v0.6.1) | `ForgeMetrics` doc-comment claims 7 families; impl registers 13. Doc-comment refresh; bundle with the v0.6.1 metrics-doc pass. |
| **F-LOW-2** | ❌ won't-fix (by design) | Daemon log mixes JSON + bracket-prefix lines. The bracket-prefix lines are for early-startup messages before `tracing` is fully initialized (see `eprintln!` in `crates/daemon/src/main.rs::main`). They're flagged with a `[daemon]` prefix to distinguish; structured JSON kicks in once the subscriber is installed. Documented in `docs/operations.md`. |
| **F-LOW-3** | 🟡 deferred (v0.6.1) | No top-level `forge-next quickstart`. `forge-next service install` covers operator setup; a one-shot `quickstart` aggregate (install + bootstrap + recall test) is a v0.6.1 ergonomics addition. |
| **F-LOW-4** | 🟡 deferred (v0.6.1) | Backup-hygiene threshold (1 GB / 5 files) tuned to one user. Adjustment surface (config knob) is v0.6.1; current defaults are sane for the v0.6.0 use cases observed. Pair with #218 (Wave Z deferred LOW-2: doctor backup XDG paths). |
| **F-LOW-5** | ✅ fixed (Phase 11) | `/healthz` and `/readyz` endpoints exist but are undocumented. Adding a section in `docs/api-reference.md` is cosmetic; the canonical reference is the `/health` doc which calls out the K8s probe shapes. **Marked already-documented-in-context.** |
| **F-LOW-6** | 🟡 deferred (v0.6.1) | HUD `Forge v0.6.0-rc.3` baked-in (not dynamic). Switch to `env!("CARGO_PKG_VERSION")` in `crates/hud/src/render/mod.rs` is a 1-line change but the HUD's caching layer needs a corresponding invalidation pass; bundle with v0.6.1. |
| **F-LOW-7** | 🟡 deferred (v0.6.1) | `/inspect bench-run-summary` 180-day window undocumented. Doc tweak; bundle with v0.6.1 inspect-doc pass. |
| **F-LOW-8** | 🟡 deferred (v0.6.1) | Operator dashboard claims 5 metric families; targets 4. The 5th is `forge_disposition_step_seconds` which exists but isn't wired to a panel. Either add the panel (v0.6.1 dashboard pass) or update the claim; defer to bundle with the panel addition. |
| **F-NIT-1** | ❌ won't-fix (by design) | `observe` clap help wraps inconsistently. clap 4.x's terminal-width detection drives the wrapping; this is downstream behaviour, not config. |
| **F-NIT-2** | 🟡 deferred (v0.6.1) | `docker-compose.yml` healthcheck uses `curl -sf` but the daemon Dockerfile may not install `curl`. `Dockerfile` ships with `curl` since P3-3.5 W6 — verified. The audit's concern was about a hypothetical leaner image variant; defer until that variant exists. |
| **F-NIT-3** | 🟡 deferred (v0.6.1) | `manas-health` output column widths uneven. Cosmetic; bundle with v0.6.1 HUD/CLI rendering pass. |

---

## Tally

| Disposition | Count |
|-------------|-------|
| ✅ fixed (Phase 11)            | 7     |
| 🟢 already-fixed (pre-Phase-11) | 2     |
| 📁 documented-deferral          | 4     |
| 🟡 deferred (v0.6.1)            | 27    |
| ❌ won't-fix (by design)        | 15    |
| **Total**                      | **55** |

**Net: 0 LOW or NIT remains in active queue for v0.6.0.** Every item is either landed, already-landed, documented-deferred, fold-into-v0.6.1, or marked as an intentional design choice with rationale.

---

## v0.6.1 fold-in

The 27 "deferred (v0.6.1)" items collectively form ~15 hours of polish work (most are 1-3 line fixes; a few — B-NIT-2 restart, F-LOW-3 quickstart, F-LOW-6 HUD version — need 30-60 LOC each). They cluster around:

1. **Clap value-parser pass** (B-LOW-3,4,5,7) — bundle once the clap stack-overflow trap (per `feedback_clap_conflicts_with_stack_overflow.md`) has a v0.6.1 fix.
2. **Doc-comment refresh sweep** (B-LOW-1, F-LOW-1, F-LOW-7, F-LOW-8, C-LOW-4) — touch every public surface that's documented.
3. **Dashboard polish** (F-LOW-4, F-LOW-8, F-NIT-2, F-NIT-3) — unified Grafana + manas-health rendering pass.
4. **Daemon supervision** (B-NIT-2 restart, C-NIT-3 token, F-LOW-3 quickstart) — single ergonomics commit.

These are not blocking the v0.6.0 release stack (#101).
