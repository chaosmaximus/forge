# 2026-04-28 — Adversarial review: Phases 10D + 10E + 10F + 10G

## Verdict

**lockable-as-is** — 0 BLOCKER, 0 HIGH, 4 LOW (3 documented limitations + 1
test-coverage gap), 1 NIT. Every commit's body claim was cross-checked against
`git show` and matches the actual diff. The two design-tradeoff items
(F-MED-11 partial-wiring, A-009 alias coverage) are explicitly acknowledged
in the commit messages with rationale, and neither blocks v0.6.0-rc.3
release-lock. All 4 sanity gates green; daemon lib tests 1,576 passed
(unchanged); workspace build + clippy clean.

## 1. Adversarial inputs

### 1.1 `truncate_preview` (B-MED-2 helper)

| Input | `max_bytes` | Pre-fix raw `&s[..max]` | Post-fix `truncate_preview` | Verdict |
|---|---|---|---|---|
| `"hello"` | 10 | `"hello"` | `"hello"` (≤max early-return) | OK |
| `"hello world"` | 5 | `"hello"` | `"hello…"` | OK |
| `"日本語"` (9 bytes UTF-8) | 4 | **panic** (mid-codepoint) | `"日…"` | OK |
| `"Привет, мир"` | 5 | **panic** (mid-codepoint) | `"Пр…"` | OK |
| `""` | 80 | `""` | `""` | OK |
| `"abc"` | 3 | `"abc"` | `"abc"` (no ellipsis on exact match) | OK |
| `"日本"` (6 bytes) | 0 | panic | `"…"` (zero-len prefix + ellipsis) | OK — `is_char_boundary(0)` returns `true`, `&s[..0]` is empty string |

The implementation is sound. `is_char_boundary(end)` is the canonical Rust
API for this; the loop terminates because byte 0 is always a boundary. No
unsafe assumption.

### 1.2 F-MED-11 freeze-on-zero — does `ForgeWorkerDown` actually fire?

`refresh_gauges_impl` (`crates/daemon/src/server/metrics.rs:574-588`):

```rust
let g = metrics.worker_healthy.with_label_values(&[worker]);
if g.get() != 0 { g.set(1); }
```

Sequence: embedder shutdown → `set_worker_unhealthy("embedder")` → gauge
flips to 0 → next scrape's `refresh_gauges_impl` sees `g.get() == 0` and
**skips** the `set(1)` clobber. The 0 persists across scrapes. The
`ForgeWorkerDown` alert (`forge_worker_healthy == 0` for 5m) can fire.

Caveat (NOT a HIGH but worth noting): only the **embedder** path is wired.
The other 7 workers (watcher / extractor / consolidator / indexer /
perception / disposition / diagnostics) never call `set_worker_unhealthy`,
so the alert remains effectively un-triggerable for them. The commit body
explicitly calls this out as "minimum viable wiring as of Phase 10E" with
"full propagation … is Phase 10E follow-up." LOW-1 below documents it.

Race window: a worker could call `set_worker_unhealthy` between
`refresh_gauges_impl`'s `g.get()` and `g.set(1)`, but Prometheus IntGauge
operations are independent atomic loads/stores and the next scrape
restores the 0. The race-window false-positive is at most one scrape
(15s default), well below the 5-minute alert threshold.

### 1.3 F-MED-12 first-run config seed

`crates/daemon/src/main.rs:244-288`. Order:

1. `let dir = forge_dir();` — resolves `~/.forge/` (or `$FORGE_DIR`).
2. `std::fs::create_dir_all(&dir)` (line 246) — creates the dir if missing;
   exits 1 on failure (cwd-permission case is handled correctly here).
3. `set_permissions(&dir, 0o700)` (line 255) — best-effort, only warns.
4. `let config_path = format!("{dir}/config.toml");` (line 267).
5. `if !Path::new(&config_path).exists() { fs::write(...) }` (line 268).

The seed runs **after** mkdir, **after** the permission set. If
`create_dir_all` fails (e.g. read-only `/`), the daemon exits before reaching
the seed. If the dir exists but is read-only, the `fs::write` fails into the
`tracing::warn!` branch and the daemon continues (graceful degradation).
`include_str!("../../../config/default.toml")` resolves at compile time, so
no runtime fs read on the binary side. **No issue.**

### 1.4 E-17 `line_start` backfill

Migration: `UPDATE code_symbol SET line_start = 1 WHERE line_start IS NULL`.

- **Idempotent?** Yes — re-running the same UPDATE on a clean DB has zero
  rows matching the `WHERE`, no error, no row touched.
- **Downstream sentinel coupling?** `crates/daemon/src/workers/indexer.rs:631`:
  `let line_0 = sym.line_start.saturating_sub(1);`. With pre-fix `line_start=0`,
  `line_0=0`. With post-fix `line_start=1`, `line_0=0`. **No behavioral change.**
  The LSP comment at the same site states "line_start is 1-based" — the
  pre-fix `0` was a buggy state; the post-fix `1` corrects the data.
- `crates/daemon/src/db/ops.rs:1496` `unwrap_or(1)` matches the migration
  sentinel. **No issue.**
- `?` propagation per `feedback_sqlite_no_reverse_silent_migration_failure.md`
  — verified: `conn.execute(..., [])?` (not `let _ =`).

### 1.5 B-MED-8 `act-notification` doc-comment vs parser

Pre-10D the `#[arg(long, conflicts_with = "reject")]` triggered Y6's
clap stack overflow. Post-10D the attribute is removed; doc-comment
explicitly states "mutual exclusion … enforced in handler" with a
forward-reference to the runtime check. Parser now accepts:

- `--approve` alone → handler sets `approved=true`.
- `--reject` alone → handler sets `approved=false`.
- both → handler exit 2 with mutual-exclusion error.
- neither → handler exit 2 with required-arg error.

Doc-comment matches parser semantics. **OK.**

### 1.6 A-009 dual-write test gap

Match arm: `["reality", "auto_detect"] | ["project", "auto_detect"]` writes
the same `config.reality.auto_detect` field. Verified for all 4 keys. The
existing test `test_reality_config_update_at` (config.rs:2957) only exercises
the `reality.*` route. **No test pins both routes write the same field.**
LOW-2 below.

## 2. Spec compliance — per-finding

| Finding | Claim | Verified? | Evidence |
|---|---|---|---|
| **B-MED-1** | structured render for meeting vote/result | ✅ | `git show eabcea4 -- crates/cli/src/main.rs:2421-2510` |
| **B-MED-2** | char-boundary truncate at 4 sites | ✅ | `system.rs:1200`, `teams.rs:103,820`, `sync.rs:374` all delegate to `util::truncate_preview` |
| **B-MED-4** | `--format ndjson` real impl + format validation | ✅ | `system.rs:284-380`, `_kind` discriminator + exit 2 on unknown format |
| **B-MED-5** | `agent-template update` CLI surface | ✅ | `main.rs:1284-1316`, `teams.rs::update_agent_template` |
| **B-MED-6** | `team-template list` top-level CLI | ✅ | `main.rs:778-784`, `teams.rs::list_team_templates` |
| **B-MED-7** | record-tool-use stderr WARN on JSON parse fail | ✅ | `system.rs:917-936` |
| **B-MED-8** | act-notification mutual exclusion in handler | ✅ | `main.rs:2521-2548` |
| **F-MED-1** | RPATH no-op | ✅ verified-no-op | `.cargo/config.toml` already uses relocatable RPATH per X2 |
| **F-MED-2** | install.sh forge-cli → forge-next | ✅ | `scripts/install.sh:34` one-token swap |
| **F-MED-3** | grafana datasource templating | ✅ | All 9 datasource refs templated; `__inputs` block declares `DS_PROMETHEUS` + `DS_SQLITE` |
| **F-MED-4** | otlp-validation.md fictional `service restart` | ✅ | `docs/observability/otlp-validation.md:33-41` real `pkill` / systemctl / launchctl forms |
| **F-MED-6** | docs note plugin install/uninstall doesn't exist | ✅ | `docs/getting-started.md:301-309` |
| **F-MED-7** | "N layers" → "N XML sections" | ✅ | `commands/manas.rs:354-364` |
| **F-MED-8** | doctor warns embedder stall path | ✅ | `handler.rs:1773-1783` |
| **F-MED-9** | OTLP empty-endpoint warn line | ✅ | `daemon/src/main.rs:215-228` (eprintln, not tracing::warn — see NIT-1) |
| **F-MED-10** | observe row-count empty-snapshot diagnostic | ✅ | `commands/observe.rs:235-263` |
| **F-MED-11** | `set_worker_unhealthy` + freeze-on-zero | ✅ partial | embedder only; documented limitation |
| **F-MED-12** | first-run seed `~/.forge/config.toml` | ✅ | `daemon/src/main.rs:266-288`, ordering OK |
| **C-MED-1** | delete `check_quality_guard` | ✅ | -44 LOC from `extraction/router.rs` |
| **C-MED-5** | `expire_diagnostics` → `#[cfg(test)] fn` | ✅ | `db/diagnostics.rs:83-90` |
| **C-MED-6** | `count_pending` + `should_surface` → cfg(test) | ✅ | `notifications.rs:266`, `proactive.rs:375` |
| **C-MED-7** | drop duplicate Python+Go `extract_imports_*` | ✅ | -91 LOC from `lsp/regex_python.rs` + `lsp/regex_go.rs` |
| **E-14** | doc-only — explain why pattern works | ✅ | `sync.rs:425-440` comment block; no behavior change |
| **E-17** | line_start backfill + `unwrap_or(1)` | ✅ | `schema.rs:1180-1191` migration + `ops.rs:1496` decoder |
| **A-009** | `["project", X]` aliases (dual-write) | ✅ | `config.rs:1722-1738`, doc table updated |
| **A-010** | task_completion_check api-reference section | ✅ | `docs/api-reference.md:2479-2520` |
| **A-012** | drop "Auto-installs" from plugin.json | ✅ | `.claude-plugin/plugin.json:4`; marketplace.json never had it (audit cited wrong file) |
| **A-014** | replace internal phase tags in user docs | ✅ | `docs/cli-reference.md:778-783` + `docs/api-reference.md:2634-2639` + `docs/operations.md:375` |

### Deferred (per commit body, with rationale)

- **C-MED-2** (recall.rs wrapper-rot): commit-body claim ✅ — verified
  `hybrid_recall_scoped` is called from prod `hybrid_recall_with_globals`
  and `compile_*_prefix/suffix` from prod `compile_context`. Audit was wrong.
- **C-MED-3** (consolidator.rs wrapper-rot): deferred to fix-wave.
- **C-MED-4** (ProjectEngine premature trait): v0.6.1 backlog.
- **E-10** (audit-log read-tracking): v0.6.1 — write-rate doubling concern.
- **E-11** (backup pruner): paired with #218 in v0.6.1.
- **E-18** (notification.reality_id wire-shape): false-positive — verified
  the wire `Notification` struct has 16 fields, none is `reality_id`; the
  column exists in DB but never crosses the wire.

## 3. Live state

```text
$ cargo build --workspace 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.56s

$ cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.76s

$ cargo test -p forge-daemon --lib 2>&1 | tail -3
test result: ok. 1576 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;
finished in 187.49s

$ bash scripts/check-harness-sync.sh
harness-sync: OK — 158 JSON methods + 109 CLI subcommands authoritative, no drift

$ bash scripts/check-protocol-hash.sh
protocol-hash: OK — crates/core/src/protocol/request.rs ↔ .claude-plugin/plugin.json in sync

$ bash scripts/check-license-manifest.sh
license-manifest: OK — 3 file(s) declared, coverage clean

$ bash scripts/check-review-artifacts.sh
review-artifacts: OK — 29 review(s) valid, no open blocking findings
```

Per the prompt: `cargo test -p forge-cli --bin forge-next` parser-stack overflow
is pre-existing (Phase 11 backlog), not introduced by 10D-G.

## 4. Findings

### LOW-1 — F-MED-11 partial wiring (acknowledged)

Only the embedder calls `set_worker_unhealthy`. The other 7 workers
(watcher / extractor / consolidator / indexer / perception / disposition /
diagnostics) never flip to 0, so `ForgeWorkerDown` is still effectively
un-triggerable for them. The freeze-on-zero plumbing is correct and the
embedder path closes the audit's specific case ("alert can never fire");
expanding to the other workers is mechanical (`spawn_supervised`-style
shutdown hooks) but explicitly deferred. **Not a release-blocker** —
the alert at least works for one worker, which makes the dashboard
functional, and the partial wiring is documented in the commit body and
in the doc-comment on `set_worker_unhealthy`.

### LOW-2 — A-009 dual-write test coverage gap

`update_config_at`'s match arm is `["reality", X] | ["project", X]` for 4
keys. Existing test `test_reality_config_update_at` exercises only the
legacy `reality.*` route. There's no test that calls
`update_config_at(path, "project.auto_detect", "false")` and asserts
`cfg.reality.auto_detect == false`. The match-arm is verified correct by
inspection, but a regression test would prevent silent drift if either
arm is later split. Recommended fix-wave addition (one ~15-line test).

### LOW-3 — B-MED-8 mutual-exclusion not pinned by test

The runtime mutual-exclusion check at `main.rs:2528-2548` is sound but
has no regression test. A test would have to live outside `main.rs`
(parser-stack overflow per Phase 11 backlog). Defer or write as a
shell-level integration test against the binary.

### LOW-4 — E-14 is doc-only, not a behavioral fix

The audit's claim was that `conn.execute` inside an `unchecked_transaction`
body might not be transactional. The commit response is a 12-line comment
block explaining why it IS transactional. This is the correct response
(SQLite's one-tx-per-conn invariant makes the pattern functionally
identical to `tx.execute`), but the close is "docs-only" — no behavior
change. The commit body is honest about this. Logged here so future
reviewers see it was a deliberate doc-only close.

### NIT-1 — F-MED-9 docs say `tracing::warn!`, code uses `eprintln!`

`docs/observability/otlp-validation.md:91` says "the daemon emits a
`tracing::warn!` line"; the actual code at `daemon/src/main.rs:225` uses
`eprintln!("[daemon] WARN: ...")` because the tracing subscriber isn't
installed yet at that point in startup. The commit body acknowledges this
("Tracing subscriber isn't installed yet, so we use eprintln!"). The doc
text is wrong but not user-confusing — operators grepping for "OTLP
enabled but FORGE_OTLP_ENDPOINT empty" find the line either way.

## Summary for fix-wave (optional, NOT release-blocking)

1. (LOW) Add a `test_project_config_update_at` test mirroring
   `test_reality_config_update_at` but using the `project.*` route, asserting
   the same field is written.
2. (LOW) Document or schedule the F-MED-11 worker-propagation follow-up
   (the other 7 workers' `set_worker_unhealthy` wiring).
3. (NIT) Update `docs/observability/otlp-validation.md:91` to say
   `eprintln!` (or "stderr WARN line") instead of `tracing::warn!`.

None of these block locking 10D-G. **Verdict: lockable-as-is.**
