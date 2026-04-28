# Changelog

All notable changes to Forge are recorded here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] — 2026-04-28

The first tagged release. v0.6.0 is the convergence point for ~1,100
commits of breadth work spanning multi-project scoping, multi-agent
coordination primitives, OpenTelemetry observability, first-run UX
unblocking, a 147-finding pre-release audit drain, and a fully-wired
harness-sync CI gate that keeps daemon ↔ plugin ↔ hooks ↔ skills ↔
agent definitions from drifting silently.

### What's new for users

* **Project-scoped everything.** `forge-next` now treats `project` as a
  first-class concept end-to-end: code graph, recall, memory, sessions,
  notifications. Code from `~/projectA` no longer leaks into recall for
  `~/projectB` even if both sit under `/mnt`. Multi-project users get
  per-project structural isolation by default; the `--project <name>`
  CLI flag and the `<code-structure project="…" resolution="…">` XML
  tag both carry the resolved scope through to the agent.
* **Multi-agent coordination.** Real Forge teams: `team create`,
  `team run --topology star|mesh|chain`, FISP-driven message passing,
  `forge-next send`/`recv`, agent personas, and a benchmarked
  multi-agent coordination eval (`2A-6`). Composite ≥ 0.95 in CI.
* **Daemon you can trust to shut down.** SIGTERM now triggers a
  graceful drain: in-flight forced indexing finishes, the writer queue
  flushes, the socket closes cleanly. `systemctl stop forge` no longer
  truncates a write mid-transaction.
* **Observability operators expect.** OTLP traces over HTTP, three
  Prometheus families (`forge_phase_duration_seconds`,
  `forge_phase_persistence_errors_total`, `forge_layer_freshness_seconds`),
  five Grafana operator dashboards out of the box, and a
  `kpi_events`-backed `forge-next observe` CLI for ad-hoc queries
  with six pre-shaped query templates (row-count, latency, error-rate,
  throughput, phase-run-summary, bench-run-summary).
* **First-run actually works.** `project init`, `project show`,
  `compile-context --dry-run`, opt-in hook verbosity, structured
  doctor output, backup-hygiene warnings, deterministic `--version`
  output for both binaries.
* **Releasable.** Multi-arch release workflow, RPATH-baked Linux
  binaries that don't need `LD_LIBRARY_PATH`, harness-sync /
  protocol-hash / license-manifest / review-artifacts CI gates, an
  evidence-gated YAML audit contract, and a comprehensive
  pre-release audit closing 147 findings with documented disposition
  for every one.

### Breaking — protocol

The user-facing vocabulary is now **project everywhere**. The internal
SQL table rename (`reality` → `project`) lands in v0.6.0 along with
the protocol-level rename. External daemon clients (third-party scripts
hardcoding `{"method": "..."}` payloads) must migrate. The
`protocol_hash` field in `.claude-plugin/plugin.json` is
`0ad998ba944d…`; the harness-sync CI gate enforces that downstream
callers stay in sync.

* `Request::DetectReality { path }` → `Request::ProjectDetect { path }`
* `Request::ListRealities { organization_id }` → `Request::ProjectList { organization_id }`
* `ResponseData::RealityDetected { reality_id, reality_type, ... }` →
  `ResponseData::ProjectDetected { id, engine, ... }`
* `ResponseData::RealitiesList { realities }` →
  `ResponseData::ProjectList { projects }`
* SQL: `reality` table renamed to `project`; the migration handles the
  full four-quadrant `(old_exists × new_exists)` state matrix and is
  pinned by regression tests.

### Breaking — CLI

* `forge-next detect-reality` removed; use `forge-next project detect [<path>]`.
* `forge-next realities` removed; use `forge-next project list`.

### Added

#### CLI surfaces

* `forge-next project init <name> [--path PATH] [--domain DOMAIN]` —
  explicit project creation. Lets users bind a project before the
  first SessionStart fires.
* `forge-next project show <name>` — detail view with indexed file +
  symbol counts.
* `forge-next compile-context --cwd <path>` — auto-create a project
  record from CWD when `--project <name>` is supplied for an unknown
  project. Surfaces `<code-structure project="<name>"
  resolution="auto-created" ...>` instead of `resolution="no-match"`
  on a fresh project's first turn.
* `forge-next compile-context --dry-run` — preview the assembled
  context without recording an injection event or touching memory
  access counts.
* `forge-next update-session --id <SESSION> --project <NAME> [--cwd <PATH>]` —
  fix a session whose project label was set incorrectly by the
  SessionStart hook.
* `forge-next agent-template update` — modify an agent template
  without delete-and-recreate.
* `forge-next team-template list` — list available team templates.
* `forge-next observe --shape <shape>` — six pre-shaped queries
  (row-count, latency, error-rate, throughput, phase-run-summary,
  bench-run-summary) with consistent envelope (window, filter,
  group-by, stale flag, truncated flag).
* `forge-next sessions --current` / `--cwd <PATH>` — filter session
  list by current process or directory.
* `forge-next remember --valence FLOAT --intensity FLOAT` — explicit
  affective tagging on stored memories.
* `forge-next export --format ndjson` — streaming-friendly export
  with a `_kind` discriminator on every record.
* `forge-next --version` and `forge-daemon --version` emit
  `<binary> 0.6.0 (<git_sha>)`.

#### Daemon surfaces

* `Request::TaskCompletionCheck` — protocol surface for the
  task-completion notification flow.
* `Request::CompileContextTrace { ..., session_id }` — `session_id`
  threads through so trace honors per-scope context_injection
  overrides.
* SIGTERM handler — daemon now gracefully drains in-flight work on
  both SIGINT and SIGTERM (was SIGINT-only).
* Background-task supervisor — fire-and-forget heavy writes
  (force-index, future analytical batch) tracked in a `Mutex<JoinSet>`
  with `signal_shutdown` gate + `drain(timeout)` + per-resource
  `AtomicBool` reject-overlap.
* First-run config seed — `~/.forge/config.toml` is auto-created
  from the bundled `config/default.toml` on first daemon start.
* OTLP-empty-endpoint warning — `FORGE_OTLP_ENABLED=true` with
  empty `FORGE_OTLP_ENDPOINT` now emits a stderr WARN line instead
  of silently disabling the OTLP layer.
* `forge_worker_healthy{worker="<name>"}` freeze-on-zero — once a
  worker reports unhealthy, the gauge stays 0 even if the gauge
  refresh loop sees it as nominally registered. Embedder shutdown
  wired; other workers planned for v0.6.1.

#### Bench harness

* **2A-5 domain-transfer isolation bench** — N synthetic projects,
  per-project token seeding, recall from each project asserts no
  cross-project leakage. Composite ≥ 0.95.
* **2A-6 multi-agent coordination bench** — FISP-driven planner →
  generator → evaluator pipeline; agent state isolation; per-agent
  step measurement. Composite ≥ 0.95.
* **2A-7 daemon restart persistence drill** —
  `scripts/chaos/restart-drill.sh` kills the daemon mid-pass,
  restarts, asserts no data loss.
* **forge-bench `forge-identity` subcommand** — measures the
  6-dimension identity composite over fixtures.
* `bench_run_completed` v1 KPI event — every bench run emits a
  structured event with per-dimension scores into `kpi_events`.

#### Observability

* OpenTelemetry trace export over OTLP/HTTP with default
  `BatchSpanProcessor`. T10 latency calibration documented at
  ≤ 1.20× overhead.
* 23 consolidator phase spans with `phase_outcome` attribute
  (`succeeded`/`partial`/`failed`).
* Three Prometheus families: `forge_phase_duration_seconds` (Histogram),
  `forge_phase_persistence_errors_total` (Counter),
  `forge_layer_freshness_seconds` (Gauge).
* Five Grafana operator dashboards in `deploy/grafana/`,
  templated via `${DS_PROMETHEUS}` and `${DS_SQLITE}` data-source
  variables for portable import.
* Three phase-error alerts in `forge-alerts.yml` with linked
  runbooks under `docs/operations/runbooks/`.
* Nine alert runbook stubs.
* `observability-slos.md` SLO registry.
* OTLP validation procedure documented at
  `docs/observability/otlp-validation.md`.

#### Harness & release engineering

* `scripts/check-harness-sync.sh` — scans every JSON method name +
  Rust `Request::*` ref + CLI subcommand ref across `plugin.json`,
  `hooks.json`, `skills/*.md`, `agents/*.md`. Cross-checks against
  authoritative sources. Warn-only mode + 14-day amnesty timer.
  Auto-flips warn → enforce on a baked-in cutover date.
* `scripts/check-protocol-hash.sh` — keeps
  `crates/core/src/protocol/request.rs` in sync with
  `.claude-plugin/plugin.json::protocol_hash`.
* `scripts/check-license-manifest.sh` — SPDX sidecar manifest
  (`.claude-plugin/LICENSES.yaml`) with per-file SPDX + commit
  reference. CI validates manifest covers every JSON file in
  `.claude-plugin/` + `hooks/`.
* `scripts/check-review-artifacts.sh` — evidence-gated YAML audit
  contract for `docs/superpowers/reviews/*.yaml`. Schema validation
  + complete-coverage check + `HIGH+CRITICAL == 0` gate on every
  PR touching skills/agents/hooks.
* RPATH bake-in for forge-daemon — Linux binaries no longer need
  `LD_LIBRARY_PATH` on glibc<2.38 systems with bundled ONNX Runtime.
* `forge-next` CLI groups ~100+ subcommands in `--help` via
  `after_long_help = COMMAND_CATEGORIES`.
* Pre-release audit — six parallel audits (docs vs reality, CLI
  feature completeness, dead-code sweep, harness drift, DB schema /
  migration / recovery, observability + first-run UX) producing 147
  findings, every one closed or dispositioned with rationale (80
  fixed + 1 already-fixed + 4 documented-defer + 41 v0.6.1 deferred
  + 21 won't-fix-or-false-positive).

### Changed

* `<code-structure>` XML attribute renamed `reality=` → `project=`.
  The unscoped path no longer emits `project=` at all.
* `compile-context` now honors `--project <name>` correctly — pre-Z2
  the inner SQL ignored the parameter and rendered the
  most-recently indexed project for ANY caller.
* `forge-setup` skill rewritten — drops references to a non-existent
  `forge` CLI binary, points new users at `project init` and
  `compile-context --dry-run`.
* `detect-reality` (now `project detect`) accepts positional `<path>`
  in addition to `--path` flag.
* `.claude-plugin/plugin.json` no longer hard-references
  `./hooks/hooks.json` — Claude Code auto-loads that path; the
  duplicate reference was breaking `/plugin list` for forge.
* `forge-next doctor` now reports backup hygiene (`*.bak` accumulation
  in `~/.forge/`) and CLI-vs-daemon `git_sha` drift even when
  `CARGO_PKG_VERSION` matches.
* CLI message previews + sync truncation now char-boundary-safe via
  shared `commands/util::truncate_preview` helper (no UTF-8 panics
  on multibyte input).
* `act-notification` now requires either `--approve` or `--reject`
  (both-set / neither-set both exit with a clear message).
* `composite_score` in the bench scoring path now `assert!`s in
  release mode — bad weight vectors no longer silently skew the
  composite.
* Shape responses share a common envelope (window, effective
  filter, effective group-by, stale flag, truncated flag) across
  all six observability shapes.

### Fixed

* Cross-project recall scoping (`W29`/`W30`) — sentinel replacement
  for soft-scope SQL leak. `WHERE col=? OR col IS NULL OR col=''`
  + buggy upstream tagging combinations no longer leak rows
  across projects.
* Indexer freshness on `.rs` edits (`W32`) — fast-tick (60s) +
  cheap mtime/seq probe + heavy work only when inputs changed
  pattern. Resolves "lower the interval to fix lag" anti-pattern.
* Force-index cold-start latency (`I-2`/`I-3`) — async dispatch
  off the request path; WAL-lock investigation closed.
* Auto-create write path under read-only routing (`X1`) —
  `Request::*` variants in `is_read_only()` get `state.conn`
  opened `SQLITE_OPEN_READ_ONLY`; INSERT/UPDATE/DELETE now correctly
  routes through `writer_tx` or ad-hoc `Connection::open`.
* `INSERT OR REPLACE` data-loss on non-PK unique indexes (`X1.fw1`)
  — auto-create paths now use `INSERT OR IGNORE` to avoid
  destroying existing rows that happen to conflict on an unrelated
  unique index.
* SQLite migration silent-no-op (`feedback_sqlite_no_reverse_silent_migration_failure`)
  — `let _ = conn.execute(...)` swallowed `no such function:
  REVERSE`. Migrations now propagate errors with `?`.
* Decode-fallback loop infinite walk (`I-7`) — when a "find project
  from encoded name" loop walks parent prefixes looking for a real
  directory, lossy decoding made the candidate shrink until it
  landed at `/mnt` or `/`. Depth-floor heuristic (≥4 slashes)
  + marker-file guard added.
* `clap conflicts_with` stack overflow on bool fields — pattern
  documented; runtime mutual-exclusion replaces clap-level
  `conflicts_with` for the two affected flags.
* Contradiction-detector dual-detector asymmetry (`F18`) — value-shape
  + content-shape detectors now share gating contracts (valence +
  intensity floors + opposition rule).
* FTS5 trigger self-update corruption — `AFTER INSERT` triggers
  that `UPDATE` the just-inserted row corrupted the FTS5
  external-content index even with `recursive_triggers=OFF`.
  Replaced with application-layer enforcement.
* SessionStart hook silent-failure path — hook now parses stdin
  JSON for `cwd` + `session_id` (HIGH per cc-voice Round 2).
* Heartbeat-aware reaper — adding intermediate session states
  required sweeping every `WHERE status = 'X'` query for
  silent-exclusion failure modes; closed.

### Deprecated

* The bundled `forge-bench` binary is functional but its CLI
  surface is still iterating; pin to the v0.6.0 form and consult
  `docs/benchmarks/` for the supported entry points.

### Documentation

* `docs/getting-started.md` — quick-start, install methods,
  config defaults, first-run flow.
* `docs/api-reference.md` — every protocol method, request shape,
  response shape, error path.
* `docs/cli-reference.md` — every `forge-next` subcommand with
  flag-by-flag documentation; categories aligned with `--help`
  groupings.
* `docs/operations.md` — daemon ops, diagnostics, healing,
  Prometheus families, worker-healthy gauge, runbook index.
* `docs/security.md` — threat model, secret handling, audit log.
* `docs/cloud-deployment.md` — Docker, Helm, K8s, Litestream,
  Prometheus + Grafana, JWT/OIDC, RBAC.
* `docs/agent-development.md` — building agents on Forge,
  HTTP/socket/gRPC client surfaces, FISP message passing.
* `docs/observability/otlp-validation.md` — manual OTLP validation
  procedure with Jaeger setup.
* `docs/architecture/` — kpi_events namespace registry,
  events-namespace doc, instrumentation decisions.

### Infrastructure

* CI: `.github/workflows/ci.yml` runs fmt + clippy + tests on
  Linux + macOS for every PR.
* CI: `.github/workflows/release.yml` produces multi-arch binaries
  on every `v*` tag.
* CI: `.github/workflows/bench-fast.yml` runs the bench-fast
  composite on every PR (gate promotion deferred per
  `docs/operations/v0.6.0-pre-iteration-deferrals.md`).
* CI: `.github/workflows/bench-regression.yml` opens a GitHub
  Issue when bench composite drops ≥ 5% on master.
* GitHub repo governance: `.github/CODEOWNERS`,
  `.github/dependabot.yml`, issue templates, PR template.
* SPDX sidecar manifest at `.claude-plugin/LICENSES.yaml`.

### Notable commits

Phase close-outs (each pins the HEAD that ended a major sub-phase):

* `1dfb552` — pre-release audit Phases 1-11 closed; halt for #101.
* `5328de5` — Phase 10 adversarial review verdict + 2-item fix-wave.
* `b73401d` — tracking ledger reflects Phases 5-11 closure.
* `b2452f5` — Phase 11 LOW + NIT triage matrix (55-item).
* `fe8054d` — Phase 10G residual E + A MEDs.
* `eabcea4` — Phase 10D CLI MEDs.
* `49e2ae7` — Phase 10E observability/UX MEDs.
* `7c5dc4d` — Phase 10F code-quality MEDs.
* `1cf9a7d` — ZR sequence (`reality` → `project` rename) closed.
* `06e1f4c` — Wave A+B close (W1.29 SIGTERM JoinSet + W1.30 dispatch flag).
* `1d5109b` — Wave X close (auto-create + RPATH bake-in).
* `1b0e8f0` — Wave Y close (cc-voice Round 2 surfaces).
* `38d7acc` — Wave Z close (CC voice first-run unblock).
* `848f140` — P3-4 W1 iteration phase closed.
* `519e6ee` — P3-3.11 W34 close (cross-project scope investigation).

Pre-release audit fix-wave (Phase 1-4):

* `386d32f` — 5 CRITICAL.
* `12e0466` — 5 CLI HIGH.
* `b8f7fb9` — 4 DB HIGH.
* `5432641` — 5 observability/UX HIGH.

Run `git log v0.5.0..v0.6.0 --oneline --no-merges` for the full
~1,100-commit log once both tags exist locally. The tracking ledger
at `docs/superpowers/audits/2026-04-27-tracking-ledger.md` enumerates
every audit finding's disposition with commit-SHA evidence.

## Pre-v0.6.0

Forge has shipped continuously since the v0.1.x line; the project
operated under a rolling `[Unreleased]` policy until v0.6.0 froze
the surface for the first tagged release. Earlier development is
recoverable via `git log` and the per-phase HANDOFF rewrites under
`docs/superpowers/`.
