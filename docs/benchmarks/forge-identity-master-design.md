# Forge-Identity — Phase 2A-4 Master Design

**Status:** DRAFT — awaiting adversarial reviews (Claude + codex CLI).
**Parent plan:** [phase-2-plan.md §2A-4](./phase-2-plan.md) "Memory is identity — agents develop persistent personality that compounds across sessions."
**Methodology:** Same 7-gate pattern as Forge-Consolidation: design → adversarial reviews → implementation plan → TDD subagent cycles → dogfood → results doc → memory handoff.

---

## 1. Thesis

Memory systems that treat preferences as static data lose information the moment the user changes their mind. **Identity is the compounding residue** — who the agent works for, how they think, what they've learned to do without being asked. Forge's daemon must prove on a benchmark that it can:

- Track explicit user preferences time-ordered with most-recent-wins semantics
- Detect and **flip valence** when the user changes their mind (without losing history)
- Weight recent preferences more heavily than stale (a 6-month-old preference is measurably weaker than yesterday's)
- **Infer behavioral skills** from observed recurring patterns (not just explicit memory statements)
- Persist identity facets coherently across sessions and surface them in compiled context
- Bound disposition drift to ±0.05/cycle under evidence-driven updates
- Flip the LongMemEval single-session-preference story arc from weakness to strength

If these hold measurably, Forge has an identity-tracking moat no MemPalace recipe replicates.

---

## 2. Scope — 4 sub-phases under option (ii)

The full Phase 2A-4 deliverable decomposes into three daemon-feature builds followed by one benchmark that validates them end-to-end:

| Sub-phase | Deliverable | Composite score? | Ships independently |
|-----------|-------------|------------------|---------------------|
| **2A-4a** | Valence Flipping feature | No (unit tests only) | Yes — dogfoodable on its own |
| **2A-4b** | Recency-weighted Preference Decay feature | No (unit tests only) | Yes |
| **2A-4c** | Behavioral Skill Inference feature | No (unit tests only) | Yes |
| **2A-4d** | Forge-Identity bench + results + narrative doc | **Yes (0.95 on 5 seeds)** | Final — depends on a/b/c |

Sequencing is serial: **a → b → c → d**. Each sub-phase has its own design doc, 2 adversarial reviews, implementation plan, subagent-driven TDD execution, and dogfood gate. The final bench (2A-4d) exercises all three features plus existing identity infrastructure.

---

## 3. Measurement framework (bench 2A-4d)

Six scored dimensions, weighted composite, pass at ≥ 0.95 across 5 seeds.

| # | Dimension | Weight | What it measures | Feature under test |
|---|-----------|--------|------------------|---------------------|
| 1 | Identity facet persistence | 0.15 | Store N facets in session A, verify full recovery in session B with correct strengths | existing daemon |
| 2 | Disposition drift bound | 0.15 | ±0.05/cycle cap holds across 10 synthetic cycles with scripted evidence | existing daemon |
| 3 | Preference time-ordering | 0.20 | Three same-topic preferences at t = −180d / −90d / −1d — recall returns t = −1d first | 2A-4b |
| 4 | Valence flipping correctness | 0.15 | `FlipPreference(id)` marks old as flipped (not deleted); new pref active; history accessible via `ListFlipped` | 2A-4a |
| 5 | Behavioral skill inference | 0.15 | Scripted pattern repeating in N ≥ 3 sessions appears in `skill` table with `inferred_from={session_ids}` | 2A-4c |
| 6 | Preference staleness recall | 0.20 | Two semantically similar prefs (t = −180d vs t = −1d) — t = −1d ranks higher with `score(−1d) ≥ 2 × score(−180d)` (minimum margin — exact margin locked during 2A-4d design) | 2A-4b |

**Pass gate:** composite ≥ 0.95 on all 5 seeds {1, 2, 3, 42, 100}. Below 0.95 on any seed = fail.

**Infrastructure assertions:** ~10 pre-scoring gates (schema/config/Request variants present). Any infra failure = bench immediately FAIL before dimension scoring.

Weights chosen so (a) existing daemon gets 0.30 baseline (identity + disposition are considered production-solid), (b) valence flipping and behavioral skill inference each get 0.15 (focused single-feature tests), (c) recency/staleness gets 0.40 because it's tested from two angles (binary ordering + continuous ranking gap).

---

## 4. Sub-phase responsibilities

### 2A-4a — Valence Flipping (daemon feature)

**What ships:**
- New columns on `memory` table: `valence_flipped_at TEXT NULL`, `flipped_to_id TEXT NULL`
- New Request variant: `Request::FlipPreference { memory_id, new_valence, new_intensity, reason }` → creates a new preference memory with opposite valence, marks old as flipped, stores pointer
- New Request variant: `Request::ListFlipped { agent: Option<String>, limit: Option<usize> }` → history of flipped preferences
- Consolidator Phase 9a enhancement: auto-flip detected opposite-valence pairs when contradiction confidence > 0.7 (configurable threshold)
- `CompileContext` XML: new `<preferences-flipped>` child listing recent flips (last 5) so agents know "user changed their mind"

**Out of scope:** Automatic semantic detection of flips across non-valence-tagged memories. Manual flip only + valence-tagged auto-flip.

### 2A-4b — Recency-weighted Preference Decay (daemon feature)

**What ships:**
- New config in `config.toml`: `preference_half_life_days = 14` (distinct from universal `memory_decay_rate = 0.03`)
- New decay formula for `memory_type = 'preference'`: `confidence × 2^(-days_since_access / half_life)` — half-life of 14 days vs. universal memory half-life of ~23 days (`ln(2) / 0.03`) means prefs fade ~1.65× faster
- Recall ranking: per-result `recency_boost = 2^(-days_since_access / half_life)` multiplier applied to preference-type hits only, composed with existing BM25/vector scoring
- `CompileContext` XML: `<preferences>` section now includes `age="<bucket>"` attribute where buckets are "1d" / "1w" / "1mo" / "6mo+"

**Out of scope:** Per-topic staleness. Universal preference half-life only.

### 2A-4c — Behavioral Skill Inference (daemon feature)

**What ships:**
- New consolidator phase (Phase 23): `infer_skills_from_behavior` — mines `session_message` history
- Detection signal: agent tool sequence repeated in ≥ `skill_inference_min_sessions` (default 3) distinct sessions without user correction flags
- Elevation: create row in `skill` table with `name = "<inferred pattern>"`, `domain = <inferred from tools>`, `inferred_from TEXT NOT NULL` (JSON array of session_ids)
- Deduplication: if a skill with same (name, domain) exists, append new session_id to `inferred_from` instead of creating duplicate
- New columns on `skill` table: `inferred_from TEXT NULL` (JSON array), `success_count INTEGER DEFAULT 0`, `inferred_at TEXT NULL`
- New configs: `skill_inference_min_sessions = 3`, `skill_inference_success_threshold = 0.8`

**Out of scope:** Skill retirement (a separate later concern). LLM-powered pattern naming — use deterministic templated names for now.

### 2A-4d — Forge-Identity Benchmark

**What ships:**
- In-process harness file `crates/daemon/src/bench/forge_identity.rs` (matches forge_consolidation.rs pattern)
- `ConsolidationBenchConfig`-analogue struct with seed, output dir, 6 expected-score fields
- 6 dataset generators (one per dimension), each producing deterministic synthetic scenarios via ChaCha20 seed + SHA-256 token pattern
- 6 audit functions computing per-dimension scores
- Composite scorer producing `IdentityScore { composite, dimensions[6], infrastructure_checks[], pass }`
- CLI subcommand in `forge-bench.rs`: `forge-bench forge-identity --seed N --output DIR`
- Integration test at `crates/daemon/tests/forge_identity_harness.rs`
- Calibration loop producing 1.0 composite on 5 seeds (expect 2-3 daemon bugs surfaced during calibration)
- Results doc at `docs/benchmarks/results/forge-identity-YYYY-MM-DD.md`
- **Master summary doc** at `docs/benchmarks/forge-identity-master-summary.md` — tying the narrative across a/b/c/d for investor pitch

---

## 5. Temporal model

Synthetic time via **direct SQL manipulation** of `created_at` / `accessed_at` columns (matches Forge-Consolidation's `backdate_memory_timestamp` helper). No real-clock waiting.

Session boundaries: bench seeds N memories with `session_id` = `"bench-sess-{i}"` for i in 0..N. Each "session" represents one day of simulated activity unless otherwise specified. 6-month staleness = 180-day backdate via `UPDATE memory SET created_at = ?1, accessed_at = ?1 WHERE id = ?2`.

Consolidator cycles triggered deterministically via `Request::ForceConsolidate`. Disposition worker stepped via a new test-only hook `Request::StepDispositionOnce` (introduced under an `#[cfg(any(test, feature = "bench"))]` gate) — avoids modifying production disposition timing.

---

## 6. Infrastructure assertions (bench gate)

Before any dimension is scored, bench asserts the following prerequisites exist. Any failure = immediate FAIL with diagnostic output. Exact list locked during 2A-4d design, but draft:

1. `identity` table schema has columns {id, agent, facet, description, strength, source, active, created_at, user_id}
2. `disposition.rs:MAX_DELTA == 0.05`
3. `memory` table has columns `valence_flipped_at TEXT NULL`, `flipped_to_id TEXT NULL` (post 2A-4a)
4. `Request::FlipPreference` variant exists with correct field shape (post 2A-4a)
5. `Request::ListFlipped` variant exists (post 2A-4a)
6. `config.toml` has `preference_half_life_days` with non-zero value (post 2A-4b)
7. `skill` table has columns `inferred_from`, `success_count`, `inferred_at` (post 2A-4c)
8. Consolidator phase count ≥ 23 (post 2A-4c) — or a named `infer_skills_from_behavior` probe phase is callable
9. `CompileContext` response XML contains `<preferences>` element with `age` attribute (post 2A-4b)
10. `CompileContext` response XML contains `<preferences-flipped>` element (post 2A-4a)

---

## 7. Harness architecture

Matches Forge-Consolidation:

- **In-process:** `DaemonState::new(":memory:")` + direct `handle_request()` calls
- **No subprocess, no HTTP** — bench tests the library, not the binary (recovery/persistence characteristics already covered by Forge-Persist)
- **Deterministic seeds:** ChaCha20 RNG from `u64` parameter; all randomness derives from this
- **Synthetic embeddings:** 768-dim unit vectors via Gram-Schmidt perturbation (shared helper from forge_consolidation.rs — factor into `common.rs` if needed)
- **Content token strategy:** SHA-256 hex tokens in memory content to avoid Phase 2 semantic dedup catching bench fixtures (Forge-Consolidation lesson)

---

## 8. Non-goals (explicit)

- **Multi-user isolation** — different `user_id` values — Phase 2A-6 Forge-Transfer will own that
- **Real session logs** from Claude Code — synthetic only for reproducibility/privacy
- **LLM-extracted preferences** — use explicit `Remember()` calls; extraction quality is a separate concern
- **Cross-agent identity** — facets shared across agent instances — scope explicitly to single agent (`claude-code`)
- **Emergent behavioral patterns** from real agent actions — scripted only; real-observation testing deferred
- **Disposition traits beyond caution/thoroughness** — Autonomy / Verbosity / Creativity exist in the daemon but are not scored in this bench
- **Preference staleness beyond 180 days** — cap simulation at 6 months; 1-year+ extrapolation is not tested

---

## 9. Deliverables per sub-phase

Each of 2A-4a / 2A-4b / 2A-4c ships:

1. Detailed design doc at `docs/superpowers/specs/2026-04-17-<name>-design.md` (feature spec, not benchmark spec)
2. Two adversarial reviews (Claude `feature-dev:code-reviewer` + codex CLI) on the design
3. Implementation plan at `docs/superpowers/plans/2026-04-17-<name>.md`
4. TDD cycles via `superpowers:subagent-driven-development` with per-task spec + code-quality reviews
5. Dogfood run on live daemon (rebuild + restart + verify)
6. Memory handoff file noting feature-complete status + any known gaps

2A-4d additionally ships:

7. Bench design doc at `docs/benchmarks/forge-identity-design.md`
8. Implementation plan at `docs/superpowers/plans/2026-04-17-forge-identity-bench.md`
9. Results doc at `docs/benchmarks/results/forge-identity-2026-04-XX.md`
10. Master summary doc at `docs/benchmarks/forge-identity-master-summary.md` — narrative across a/b/c/d, investor-ready

---

## 10. Success criteria

- All 3 features (a/b/c) ship with TDD, `cargo clippy --workspace -- -W clippy::all -D warnings` clean, `cargo test --workspace` green
- Forge-Identity bench composite ≥ 0.95 across all 5 seeds
- Narrative gate passed: results doc includes LongMemEval single-session-preference comparison row showing Forge-Identity dominance on paraphrased preference tracking
- **Bench-driven improvement loop holds:** 2-3 real daemon bugs surfaced and fixed during calibration (expected per the pattern proven in Forge-Consolidation)
- Master summary tells the "memory is identity" story end-to-end with reproducible commands
- Each sub-phase dogfoods cleanly on the user's live daemon before moving to the next

---

## 11. Known risks

- **Behavioral skill inference complexity (2A-4c)** is the biggest unknown. Pattern detection from session_message history may need iteration. If the 8-12 task estimate blows up, we split 2A-4c into c1 (detection heuristic) and c2 (skill elevation).
- **Phase 23 interaction with existing Phase 17** (protocol extraction) needs careful design so they don't produce duplicate/conflicting entries. Resolved during 2A-4c design.
- **Bench flakiness risk from tiebreaker non-determinism** — learned from Forge-Consolidation that `LIMIT N ORDER BY col DESC` ties cause seed-dependent pass/fail. Bench must isolate scoring candidates by giving them unique sort keys.
- **Auto-flip heuristic threshold (2A-4a)** — 0.7 confidence is a guess; may need calibration during 2A-4d bench.
- **Recency multiplier composition with BM25** (2A-4b) — may interact with hybrid retrieval in unexpected ways. Test surface area must include recall on mixed queries (preferences + non-preferences).

---

## 12. Open decisions (resolved during sub-phase design)

Flagged here so adversarial review can push back early:

- **D1 — Auto-flip threshold:** 0.7 contradiction confidence. Could be 0.5 (aggressive) or 0.9 (conservative). Decide during 2A-4a.
- **D2 — Preference half-life:** 14 days. Could be 7 (fast-fade) or 30 (slow-fade). Decide during 2A-4b.
- **D3 — Skill inference min sessions:** 3. Could be 2 (eager) or 5 (cautious). Decide during 2A-4c.
- **D4 — CompileContext preferences section:** Include or omit when empty? Decide during 2A-4b.
- **D5 — Flipped-memory ranking:** Do flipped memories still surface in recall (for history access) or are they filtered? Decide during 2A-4a.
- **D6 — Bench temporal simulation depth:** Is 6 months enough, or do we also test 2+ years for long-horizon behavior? Decide during 2A-4d.
