# Handoff — Post-Compact Continuation (2026-04-24 PM)

**Public HEAD (chaosmaximus/forge):** `0546a69` (see `git log -1` for current).
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (post-2P-1a prune).
**Current version:** **v0.5.0** — not tagged on GitHub (parked until product complete per user decision 2026-04-24).

## State in one paragraph

Phase **2A-4d.1 (Instrumentation tier of Forge-Identity Observability)** is IN PROGRESS — spec LOCKED at `b2dfa20` after 3 rounds of adversarial review + targeted convergence check, plan file at `docs/superpowers/plans/2026-04-24-forge-identity-observability.md`. Implementation partially landed: T1 recon ✅, T2+T4 instrumentation helper + 3 new Prometheus families ✅, T3 all 23 consolidator phases wrapped in `info_span!` ✅, T5 `docs/architecture/` + `kpi_events` namespace register ✅, T6.1–T6.3 eprintln→tracing convergence for reaper / mod / embedder / perception / watcher / disposition / diagnostics / extractor ✅ (122 of 184 sites converted). T6 remaining: **64 sites across indexer.rs (33) + consolidator.rs leftovers (31)**. T7 CI guard, T8 adversarial reviews, T10 latency baseline, T11 live-daemon dogfood still pending.

Phase **2P-1a (plugin surface migration)** and **2P-1b partial (11 of 18 items)** and **2A-4c2 (Behavioral Skill Inference)** SHIPPED pre-session. Phase **2A-4d.2 + 2A-4d.3** blocked on 2A-4d.1 completion.

Release / marketplace / macOS dogfood are **PARKED by user directive** — won't resume until product is complete (everything in Stages S1–S5 of the SOA roadmap).

## First actions after `/compact`

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git status --short                                      # working tree should be clean except for .gitignore drift
cargo fmt --all --check                                 # should pass
cargo clippy --workspace -- -W clippy::all -D warnings  # should be 0 warnings
cargo test -p forge-daemon --lib instrumentation        # 4 passing (1 was ignored pre-T3, now un-ignored)
cargo test -p forge-daemon --lib test_forge_metrics_new_registers_all_families  # 10 families
grep -c 'eprintln!\|println!' crates/daemon/src/workers/*.rs | grep -v ':0$'     # indexer: 33, consolidator: 31, rest: 0
```

If all pass, resume **2A-4d.1 T6 convergence** where it left off (indexer.rs next).

## Resume plan for 2A-4d.1

### T6 eprintln→tracing convergence (in progress)

**Remaining files:**
| File | Sites | Commit |
|------|-------|--------|
| `crates/daemon/src/workers/indexer.rs` | 33 | T6.4 — still pending |
| `crates/daemon/src/workers/consolidator.rs` | 31 leftover (in helper fns below `run_all_phases`, e.g. `synthesize_contradictions`, `reweave_memories`, `detect_content_contradictions`, `heal_*`) | T6.5 — still pending |

**Conversion pattern** (stable across all commits — already established in T6.1–T6.3):
- `eprintln!("[W] X")` → `tracing::info!(target: "forge::W", "X")`
- `eprintln!("[W] error: {e}")` → `tracing::error!(target: "forge::W", error = %e, "…")`
- `eprintln!("[W] WARN X")` → `tracing::warn!(target: "forge::W", "X")`

Target: `forge::indexer`, `forge::consolidator`. Structured fields replace interpolation (file, path, error, session_id, memory_id, etc.) where useful.

**Acceptance:** `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs | grep -v '#\[cfg(test'` returns 0.

### T7 — CI span-integrity + `tokio::spawn` guard (after T6 done)

Add to the `check` job (NOT plugin-surface) of `.github/workflows/ci.yml`:

```yaml
- name: Span integrity guard
  run: |
    set -euo pipefail
    count=$(grep -c 'info_span!("phase_' crates/daemon/src/workers/consolidator.rs)
    [ "$count" = "23" ] || { echo "span count $count != 23"; exit 1; }
    ! grep -n 'tokio::spawn' crates/daemon/src/workers/consolidator.rs
    ! grep -n 'tokio::spawn' crates/daemon/src/db/ops.rs
```

### T8 — Two adversarial reviews on T1–T7 diff

Claude general-purpose + Codex codex-rescue, inverted prompts. Probe angles per plan §Task 8. Diff range: all commits since `7ac1f71` (spec + plan + T2+T4 + T3 + T5 + T6 + T7).

### T9 — Address findings

One commit per finding. `fix(2A-4d.1 T9): address <severity>-<n>-<slug>`.

### T10 — Latency baseline (MoM N=5 using deterministic `forge_consolidation_harness`)

Commit baseline file BEFORE any T3 code was landed — need to reconstruct via `git checkout 2668b6d -- crates/daemon/src/workers/consolidator.rs` (pre-T3) ephemerally, measure, revert, measure post-T3, diff. Budget per spec §3.7: cold-start OTLP-off ≤ 20 ms, OTLP-on ≤ 100 ms, steady-state CPU ≤ 2%, `force_consolidate` on seeded 100-mem DB ≤ 10 ms.

### T11 — Live-daemon dogfood + results doc

- Rebuild release daemon at HEAD post-T10.
- `docker run -d -p 16686:16686 -p 4317:4317 jaegertracing/all-in-one:latest`
- `FORGE_OTLP_ENABLED=true FORGE_OTLP_ENDPOINT=http://localhost:4317 FORGE_DIR=/tmp/forge-t1-dogfood …/forge-daemon &`
- Seed + `force_consolidate`.
- Verify: `/metrics` has 10 families with non-zero; `kpi_events` has 23 rows per pass with `metadata_schema_version: 1`; Jaeger shows trace with 23 child spans under `consolidate_pass`.
- Results doc: `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md`.
- Update HANDOFF changelog.

## Session commits (2026-04-24 PM)

| SHA | Summary |
|-----|---------|
| `d30eaab` | docs(2A-4d.1): Instrumentation design v1 |
| `65ebdf3` | docs(2A-4d.1): revise to v2 (R1 review fixes) |
| `7ed071e` | docs(2A-4d.1): v3 (R2 review fixes) |
| `b2dfa20` | docs(2A-4d.1): v4 lock-ready (R3 review fixes) |
| `86492ec` | docs(2A-4d.1): LOCK spec v4 + write execution plan |
| `2668b6d` | feat(2A-4d.1 T2+T4): workers::instrumentation helper + 3 Prometheus families |
| `99de50e` | feat(2A-4d.1 T3): wrap 23 consolidator phases with info_span! + PhaseOutcome |
| `7ac1f71` | docs(2A-4d.1 T5): docs/architecture/ + kpi_events namespace register |
| `06755f9` | chore(2A-4d.1 T6.1): convert eprintln! → tracing in small workers (reaper/mod/embedder/perception/watcher, 35 sites) |
| `ed89161` | chore(2A-4d.1 T6.2): disposition + diagnostics (25 sites) |
| `0546a69` | chore(2A-4d.1 T6.3): extractor (22 sites) |

Earlier in the session (before spec v1):
| SHA | Summary |
|-----|---------|
| `6ee6e9f` | feat(2P-1b §15): expose skills_inferred in ConsolidationComplete + tracing |
| `c61d926` | fix(2P-1b §10): hook-level latent bugs (4 of 5) |
| `ab88450` | feat(2P-1b §1): harness-sync CI drift detector |
| `0563873` | docs(2P-1b §3 + §8): SPDX sidecar + sideload migration guide |
| `030711b` | feat(2P-1b §10): wire post-edit + post-bash hooks to record_tool_use |
| `d9fda72` | chore(2P-1b §9 + §12 + §17): CODEOWNERS + dependabot + retire stale validators + json_valid guard |
| `a26ac7a` | docs(2P-1b §5): rollback playbook |
| `baea19b` | test(2P-1b §11): un-ignore hook e2e tests via TestDaemon helper |
| `f0fccf3` | fix(2P-1b §14): windowed pruning of inferred_from — replace not merge |
| `d9dc8e6` | docs(stream C + 2P-1b): close orphan plan, log B progress |

## Environment prerequisites

- Ubuntu 22.04 (glibc 2.35) or newer. `.cargo/config.toml` + `scripts/with-ort.sh` wire ORT transparently.
- `sudo apt-get install -y pkg-config libssl-dev` (one-time).
- For running the daemon directly (outside cargo): `export LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib"`.

## Files of interest

| File | Why |
|------|-----|
| `CLAUDE.md` | Project identity, harness philosophy, git workflow. |
| `HANDOFF.md` | This file. |
| `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md` | 2A-4d.1 LOCKED spec v4. |
| `docs/superpowers/plans/2026-04-24-forge-identity-observability.md` | 2A-4d.1 task plan (T1-T5 ✅, T6.1-T6.3 ✅, T6.4 + T6.5 pending, T7-T11 pending). |
| `crates/daemon/src/workers/instrumentation.rs` | `PhaseOutcome` + `PHASE_SPAN_NAMES` + `record()` helper. |
| `crates/daemon/src/workers/consolidator.rs` | 23 `info_span!("phase_N_…")` call sites wrapping each phase (`run_all_phases`). 31 eprintln left in helper fns BELOW `run_all_phases`. |
| `crates/daemon/src/server/metrics.rs` | 10 metric families (7 existing + 3 new: `forge_phase_duration_seconds`, `forge_phase_output_rows_total`, `forge_table_rows`). |
| `crates/daemon/src/db/schema.rs:255-266` | `kpi_events` table (Tier 1 is first writer). |
| `docs/architecture/kpi_events-namespace.md` | Namespace register. `phase_completed` claimed by 2A-4d.1. |
| `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md` | 2A-4c2 task plan (SHIPPED). |
| `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md` | 2A-4c2 dogfood results. |

## Parked (user decision, 2026-04-24)

Do NOT resume until the product is "complete" (Stages S1–S5 done):

- §4 macOS dogfood matrix.
- §6 marketplace publication.
- §13 cut v0.5.0 GitHub release (tag + push).

`Formula/forge.rb` and `scripts/install.sh` URL templates will 404 until §13 ships — acceptable per user directive.

## SOA roadmap (agreed 2026-04-24)

Three-tier architecture for Forge-Identity Observability:

- **2A-4d.1 Instrumentation** (in progress) — spans, metrics, OTLP export, `kpi_events` writes, eprintln convergence.
- **2A-4d.2 Observability API** — `/inspect {layer, shape, window}` + SSE + `forge-next observe` CLI + HUD drift + `forge_layer_freshness_seconds` + `kpi_events` retention reaper.
- **2A-4d.3 Bench harness** — `forge-bench identity` + fixtures v1 + `bench_runs` table + ablation flags + CI per-commit + leaderboard.

Each tier ships independently with its own 2 adversarial reviews + dogfood before the next starts.

## Phase 2P-1b backlog (harden — 11 of 18 shipped, 7 remaining)

Shipped: §1 §3 §5 §8 §9 §10 (5/5) §11 §12 §14 §15 §17.

Remaining (parked or design-heavy):
- §2 evidence-gated audit contract (YAML reviews, bigger design).
- §4 expanded dogfood matrix (parked).
- §6 marketplace publication (parked, needs Anthropic).
- §7 2A-4d interlock — time-gated (flips 2026-05-08 when §1 goes fail-closed).
- §13 v0.5.0 release (parked).
- §16 shape-vs-behavior fingerprint split (substantial design; 2A-4d refinement).
- §18 Phase 23 numbering alignment (design call).

## Process rules

- Work on `master` directly (no feature branches by default).
- **Do not use git worktrees without explicit per-task permission.**
- `cargo fmt --all` + `cargo clippy --workspace -- -W clippy::all -D warnings` 0-warnings at every commit boundary.
- `cargo test --workspace` green after each GREEN phase.
- Two adversarial reviews before any merge of a design spec; same for any diff that ships to master.
- Commit prefixes: `feat(<phase> <task>):`, `fix(...)`, `chore(...)`, `docs(...)`, `test(...)`. Co-author trailer `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`. New commits, never `--amend`.

## What NOT to redo

- 2P-1a — SHIPPED.
- 2A-4c2 — SHIPPED. Spec + plan LOCKED. Carry-forwards live in 2P-1b §14-18.
- 2A-4c1, 2A-4b, 2A-4a — SHIPPED.
- 2A-4d.1 spec — LOCKED at v4 / `b2dfa20`. Don't revise again; verify recon at implementation time per Task 1.
- 2P-1b §1 §3 §5 §8 §9 §10 (5/5) §11 §12 §14 §15 §17 — SHIPPED.
- Don't re-derive the scrub lexicon, migration tooling, or inventory from scratch — `scripts/migrate-*.sh`, `tests/fixtures/scrub/`, `docs/superpowers/plans/2P-1a-inventory.md` are authoritative.
- Don't create new git branches (master-direct workflow).
- Don't try to push v0.5.0 tag yet (billing blocked + user parked release gate).
