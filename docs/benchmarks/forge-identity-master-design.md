# Forge-Identity — Phase 2A-4 Master Design (v3)

**Status:** DRAFT v3 — addresses v2 review blockers (Claude second-pass: 4 CRITICAL / 10 HIGH; codex second-pass: 1 CRITICAL / 6 HIGH). v3 targets only the 5 master-level blockers that affect bench validity and pushes all code-reality-drift findings to sub-phase design docs where they will be grounded by direct code exploration.
**Parent plan:** [phase-2-plan.md §2A-4](./phase-2-plan.md) "Memory is identity — agents develop persistent personality that compounds across sessions."
**Methodology:** Same 7-gate pattern as Forge-Consolidation: design → adversarial reviews → implementation plan → TDD subagent cycles → dogfood → results doc → memory handoff.
**v1 → v2 diff summary:** Auto-flip simplified to explicit-only; decay anchor changed from `accessed_at` to `created_at` with explicit `touch()` exemption for preferences; 2A-4c split into c1 (tool-use schema) + c2 (Phase 23); `<preferences>` XML reclassified as greenfield; Dim 6 replaced with gradient test; Dim 3/6 independence resolved; Phase 17/23 ownership rule; per-dim minimums added; LongMemEval gate downgraded to narrative goal.

---

## 1. Thesis

Memory systems that treat preferences as static data lose information the moment the user changes their mind. **Identity is the compounding residue** — who the agent works for, how they think, what they've learned to do without being asked. Forge's daemon must prove on a benchmark that it can:

- Track explicit user preferences time-ordered with most-recent-wins semantics, anchored to when the user **stated** the preference, not when the system retrieved it
- Explicitly **flip preference valence** via a user/agent-initiated API when the user changes their mind (without losing history)
- Weight recent preferences more heavily than stale (a 6-month-old preference is measurably weaker than yesterday's)
- **Infer behavioral skills** from recorded agent tool-use (not from inter-session FISP messages, which are the wrong signal)
- Persist identity facets coherently across sessions and surface them in compiled context
- Bound disposition drift to ±0.05/cycle under scripted session-duration evidence
- Flip the LongMemEval single-session-preference story arc from weakness to narrative strength (narrative goal, not a pass gate)

If these hold measurably, Forge has an identity-tracking moat no MemPalace recipe replicates.

---

## 2. Scope — 5 sub-phases under option (ii), revised

The full Phase 2A-4 deliverable decomposes into four daemon-feature builds followed by one benchmark that validates them end-to-end:

| Sub-phase | Deliverable | Composite score? | Ships independently |
|-----------|-------------|------------------|---------------------|
| **2A-4a** | Valence Flipping feature (explicit API only) | No (unit tests only) | Yes — dogfoodable on its own |
| **2A-4b** | Recency-weighted Preference Decay feature | No (unit tests only) | Yes |
| **2A-4c1** | Tool-use recording schema + `Request::RecordToolUse` + ingestion hook | No (unit tests only) | Yes — enables 2A-4c2 + future benches |
| **2A-4c2** | Phase 23 Behavioral Skill Inference feature | No (unit tests only) | Yes — depends on c1 |
| **2A-4d** | Forge-Identity bench + results + narrative doc | **Yes (composite + per-dim minimums)** | Final — depends on a/b/c1/c2 |

Sequencing is serial: **a → b → c1 → c2 → d**. Each sub-phase has its own design doc, 2 adversarial reviews, implementation plan, subagent-driven TDD execution, and dogfood gate. The final bench (2A-4d) exercises all four feature builds plus existing identity infrastructure.

---

## 3. Timestamp semantics (foundational — locked at master level)

Because preference staleness is central to Dim 3 and Dim 6, and because `accessed_at` self-refreshes on recall (`touch()` at `crates/daemon/src/db/ops.rs:637-652`), preference age must be anchored to an **immutable or user-controlled timestamp**:

- **Primary anchor:** `created_at` (immutable after insert)
- **User-controlled anchor (future-proof):** a new nullable column `reaffirmed_at` (written only by explicit user/agent action, never by `touch()`). If null, falls back to `created_at`.
- **`touch()` exemption for preferences:** `touch()` is skipped entirely for `memory_type = 'preference'`. The exemption lives inside `crates/daemon/src/db/ops.rs:touch()` (the actual mutation point reached from `writer.rs`), implemented as a SQL predicate: `UPDATE memory SET accessed_at = ... WHERE id IN (...) AND memory_type != 'preference'`. This is the only sound architectural layer: `writer.rs` receives `ids: Vec<String>` without memory types, so the type check must happen at the DB-ops layer where the UPDATE is built.
- **Preference decay formula (2A-4b):** `confidence × 2^(-days_since_pref_age / half_life)` where `days_since_pref_age = now - coalesce(reaffirmed_at, created_at)`.
- **Preference-fade exemption:** preferences are exempt from the universal hard-fade threshold of 0.1. The `decay_memories` function skips the `UPDATE memory SET status='faded'` branch when `memory_type = 'preference'`. Preferences always remain `status='active'` with whatever decayed confidence they have; recall ranking naturally de-boosts them via the type-dispatched recency multiplier (no separate threshold config needed).
- **Recall-side recency weighting (2A-4b):** Applied post-RRF at `recall.rs:404-413`. The CURRENT code pattern there is `result.score *= 1.0 + recency_boost * 0.5` where `recency_boost = exp(-0.1 * days_old)`. **The 2A-4b change replaces this pattern entirely** with a direct type-dispatched multiplier: `result.score *= recency_factor(memory)` where `recency_factor` is `exp(-0.1 * days_old)` for non-preferences (preserving the current *shape* of decay while dropping the `1.0 + ... * 0.5` envelope) OR `2^(-days / 14)` for preferences. The change in structure (direct multiplier vs envelope) is a deliberate simplification — the 2A-4b regression-guard (see §5) re-calibrates Forge-Context and Forge-Consolidation against the new structure; any score shift must be documented and resolved before 2A-4b merges.

---

## 4. Measurement framework (bench 2A-4d)

Six scored dimensions with per-dim minimums, weighted composite, pass at composite ≥ 0.95 **AND every dimension ≥ 0.80** (no single dim below 0.80, regardless of composite).

| # | Dimension | Weight | What it measures | Minimum |
|---|-----------|--------|------------------|---------|
| 1 | Identity facet persistence | 0.15 | Store N facets in session A, verify full recovery in session B with strengths within ±0.001 (exact, no identity-worker updates between sessions) | 0.85 |
| 2 | Disposition drift bound | 0.15 | Scripted session-duration fixtures across 10 cycles; every cycle's per-trait delta ≤ 0.05 exactly; final values match expected trajectory within ±0.01 | 0.85 |
| 3 | Preference time-ordering (pure-preference recall) | 0.15 | Three same-topic preferences at `created_at` = −180d / −90d / −1d — pure-preference query returns in order [−1d, −90d, −180d] | 0.80 |
| 4 | Valence flipping correctness | 0.15 | `FlipPreference(id, new_valence)` marks old as flipped (status='superseded' + `valence_flipped_at` metadata); new preference active; `ListFlipped` returns old; default recall filters flipped; explicit `include_flipped=true` surfaces both | 0.85 |
| 5 | Behavioral skill inference | 0.15 | Recorded tool-use pattern (via `Request::RecordToolUse`) repeating in N ≥ 3 distinct sessions with identical canonical fingerprint → appears in `skill` table with `inferred_from={session_ids}`; no duplicate skill row for the same canonical fingerprint | 0.80 |
| 6 | Preference staleness ratio-correctness + mixed-corpus precision | 0.25 | **(6a, weight 0.15, floor 0.75)** Four same-topic preferences at `created_at` = now − {1, 14, 90, 180} days. Fixture requires **RRF-identity** — all 4 prefs share: (i) identical 768-dim embedding `v_pref` (not similar — byte-identical), (ii) identical `title` string (so BM25 produces identical lexical ranks), (iii) identical seeded `confidence = 0.9`. With RRF-identity, BM25 ranks tie and vector ranks tie, so the RRF-fused score is identical for all 4; only the type-dispatched recency multiplier differs. Compute final post-RRF scores from Recall. Assert ratio invariants (these are the TRUE values to 4 decimal places — verify in bench code): `score(−1d)/score(−14d)` ∈ [1.85, 2.00] (expected `2^(13/14)` = **1.9034**), `score(−14d)/score(−90d)` ∈ [40.5, 45.5] (expected `2^(76/14)` = **43.0688**), `score(−90d)/score(−180d)` ∈ [81, 91] (expected `2^(90/14)` = **86.1376**). AND strict ordering score(−1d) > score(−14d) > score(−90d) > score(−180d). **(6b, weight 0.10, floor 0.75)** Mixed-corpus test: same 4 prefs + 4 non-preference distractors (lessons/decisions) with similar but distinct embedding. Expected top-8: (i) 4 prefs in recency order, (ii) ≥ 1 non-preference in top-5 (recency multiplier doesn't crowd out non-prefs), (iii) rank of −180d pref ≥ 5. **Parent score:** `dim6 = (0.15 × score_6a + 0.10 × score_6b) / 0.25`. Parent minimum 0.80 applies to that quotient. Both sub-minimums (0.75 each) must also independently hold. | 0.80 |

**Pass gate:** composite ≥ 0.95 AND every dimension ≥ its minimum AND all infrastructure assertions pass. Any failure = bench FAIL. No "weighted-average bailout" where one broken dim hides behind high scores elsewhere.

Weights balance: existing daemon gets 0.30 (dims 1+2); new-feature dims get 0.70 (dim 3=0.15 for ordering; dim 4=0.15 for valence flip; dim 5=0.15 for skill inference; dim 6=0.25 for the richer staleness gradient that is the product's strongest identity claim).

**Dim 3 vs Dim 6 independence (addresses Codex H1):** Dim 3 is a binary ordering test on 3 preferences — checks ranking direction. Dim 6 is a calibration test on 4 preferences — checks ranking shape. A bug that passes Dim 3 (correct direction) can still fail Dim 6 (wrong slope). Conversely, a bug that gets the slope right but inverts order fails Dim 3. These test different failure modes of the same feature and are considered independent for weighting purposes. Accepted overlap noted; weights reflect Dim 6's strictness.

---

## 5. Sub-phase responsibilities

### 2A-4a — Valence Flipping (daemon feature, explicit API only)

**What ships:**
- New columns on `memory` table: `valence_flipped_at TEXT NULL`, `flipped_to_id TEXT NULL`
- New Request variant: `Request::FlipPreference { memory_id: String, new_valence: String, new_intensity: f64, reason: Option<String> }` — creates a new preference memory with opposite valence, marks old as `status='superseded'` **and** sets `valence_flipped_at`/`flipped_to_id` (additive to supersede semantics, not replacing)
- New Request variant: `Request::ListFlipped { agent: Option<String>, limit: Option<usize> }` — returns memories with `valence_flipped_at IS NOT NULL`, ordered by flip timestamp descending
- New Recall query parameter: `include_flipped: bool` (default `false`) — controls whether flipped memories appear alongside active
- `CompileContext` XML: new **greenfield** `<preferences-flipped>` child listing up to 5 most-recent flips. Budget-accounted: part of dynamic-suffix quota, takes at most 800 bytes of the `context_budget`. If empty, element omitted (cleaner XML).
- Phase 9a consolidator behavior: **unchanged**. Auto-flip is NOT part of 2A-4a. The daemon never auto-flips; flipping is user/agent-initiated only. Phase 9a continues to produce contradiction diagnostics; the `<guardrails>` section surfaces them; the agent can then call FlipPreference.

**Relationship to Supersede (addresses Claude I1):** `FlipPreference` internally calls `supersede_memory()` to set `status='superseded'` and `superseded_by`, then adds `valence_flipped_at` and `flipped_to_id` as additional metadata. `ListFlipped` queries `WHERE valence_flipped_at IS NOT NULL ORDER BY valence_flipped_at DESC`. Non-preference memories can be superseded but never flipped (FlipPreference validates `memory_type = 'preference'` and errors otherwise).

**Out of scope:** Auto-flip heuristic, semantic detection of flips across non-valence-tagged memories, `ReviveFlipped`/undo API, multi-agent flip broadcasting. Deferred explicitly.

### 2A-4b — Recency-weighted Preference Decay (daemon feature)

**Regression-guard scope (master-level mandate):** The new type-dispatched post-RRF recency multiplier changes absolute scores for ALL memories (not just preferences). Prior benches Forge-Context (2A-2) and Forge-Consolidation (2A-3) calibrated 1.0 composites against the current universal recency formula. The 2A-4b implementation plan MUST include re-running both benches' full 5-seed calibration sweeps after the formula change and BEFORE 2A-4b merges. Any non-trivial score regression in prior benches blocks 2A-4b until resolved (either by tuning the new formula, by anchoring a compatibility mode, or by updating prior benches' expected-score ranges with documented justification). The 2A-4b results-doc template must include a "prior-bench regression table" showing before/after composites for Forge-Context and Forge-Consolidation.

**What ships:**
- New config in `config.toml`: `preference_half_life_days = 14` (default; validated 1..=365)
- New decay formula in `ops::decay_memories` for `memory_type = 'preference'`: `confidence × 2^(-days_since_pref_age / half_life)` where `days_since_pref_age = now_utc - coalesce(reaffirmed_at, created_at)`. For non-preferences, `decay_memories` continues to use its existing `accessed_at`-based `× exp(-0.03 × days)` formula — unchanged.
- Preferences exempt from universal hard-fade at 0.1 (see §3 above). No `preference_fade_threshold` config — hard-fade exemption alone is sufficient.
- `recall.rs:404-413` recency weighting becomes type-dispatched. **Structural change:** the existing `result.score *= 1.0 + recency_boost * 0.5` envelope is **replaced** with a direct multiplicative factor: `result.score *= recency_factor(memory)` where `recency_factor(memory) = if memory.memory_type == Preference { 2^(-days_since_pref_age / 14) } else { exp(-0.1 * days_since_created) }`. Non-preferences keep the same 0.1 exponent as the current code but lose the `1.0 + ... * 0.5` envelope. **Composition with RRF:** applied AFTER `rrf_merge` in `recall.rs:280`, on the final `score_map` scores. Applied uniformly to graph-expanded rows too (so 1-hop neighbor preferences decay under the same rule).
- `touch()` exemption: implemented at the mutation point in `db/ops.rs:touch()` (reached via `writer.rs`), via SQL predicate `AND memory_type != 'preference'` on the UPDATE. Preferences' `accessed_at` is informational only and not updated by recall.
- `CompileContext` XML: new greenfield `<preferences>` section with `<pref age="1d|1w|1mo|6mo+">...</pref>` children. Budget-accounted like `<preferences-flipped>`. Age buckets use `coalesce(reaffirmed_at, created_at)`. Element **always emitted, even empty** (bare `<preferences/>`) — satisfies infrastructure assertion 10 and keeps the XML schema stable regardless of corpus state.
- New Request variant: `Request::ReaffirmPreference { memory_id: String }` — sets `reaffirmed_at = now_utc`. Used when user re-states a preference (e.g., "yes, still prefer vim"). Valence-only re-statement doesn't create a new memory.

**Out of scope:** Per-topic half-life, per-user half-life, semantic reaffirmation detection (deferred — requires extraction pipeline changes).

### 2A-4c1 — Tool-use recording (daemon schema + ingestion)

**What ships:**
- New table `session_tool_call`:
  ```
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  agent TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  tool_args TEXT,               -- JSON
  tool_result_summary TEXT,     -- JSON or short text
  success INTEGER NOT NULL,     -- 0 or 1
  user_correction_flag INTEGER DEFAULT 0,  -- 1 if user corrected this tool call in the same or adjacent turn
  created_at TEXT NOT NULL,
  INDEX idx_session_tool_session (session_id, created_at),
  INDEX idx_session_tool_name_agent (agent, tool_name)
  ```
- New Request variant: `Request::RecordToolUse { session_id, agent, tool_name, tool_args, tool_result_summary, success, user_correction_flag }` — inserts a row
- New Request variant: `Request::ListToolCalls { session_id: Option<String>, agent: Option<String>, limit: Option<usize> }` — for observability
- Ingestion: Claude Code hook plumbing deferred to 2A-4c2's dogfood phase (non-blocking for unit tests)
- Migration: alter adds table + indexes

**Out of scope:** Retroactive tool-use import from transcripts, multi-agent tool-use correlation, tool-use deduplication beyond what c2 does.

### 2A-4c2 — Phase 23 Behavioral Skill Inference (daemon feature)

**Prerequisite renderer update (master-level mandate):** The existing `<skills>` renderer at `crates/daemon/src/recall.rs:1058-1152` filters rows with `success_count > 0`. Phase 23 inserts new skills with `success_count = 0`. Without updating the renderer, newly-inferred skills will be invisible to `CompileContext` — Dim 5 would silently fail. The 2A-4c2 implementation plan MUST include a task that changes the renderer to also include rows where `inferred_at IS NOT NULL` (i.e., Phase 23 rows). Alternative resolution (defer to 2A-4c2 detailed design): insert Phase 23 rows with `success_count = 1` at creation time and increment on each observed successful use. Either path must be chosen and locked in 2A-4c2 design, and infrastructure assertion 12 must verify Phase 23 rows surface in `<skills>`.

**What ships:**
- New consolidator phase (Phase 23): `infer_skills_from_behavior` — runs after Phase 17 (protocol extraction)
- Detection signal: tool-call sequences from `session_tool_call` grouped by agent, canonicalized as sorted-unique-tool-name fingerprint, repeated in ≥ `skill_inference_min_sessions` distinct sessions with no `user_correction_flag=1` rows in the sessions in question
- Canonical fingerprint (addresses Codex H5): `sha256(sort(unique(tool_names)) + sort(tool_arg_shapes))` — where `tool_arg_shapes` is a normalized structural hash of each tool call's args (e.g., which arg keys were present, not their values). This prevents "different templates for the same underlying sequence" from creating duplicates.
- Elevation: INSERT INTO `skill` with `name = templated_name(fingerprint)`, `domain = infer_domain(tool_names)`, `inferred_from = '[session_id_a, session_id_b, ...]'` (JSON), `success_count = 0`, `inferred_at = now_utc`
- Deduplication: ON (agent, fingerprint) conflict → UPDATE skill SET `inferred_from = merge(existing.inferred_from, new_sessions)` instead of INSERT
- New columns on `skill` table: `agent TEXT NOT NULL DEFAULT 'claude-code'`, `fingerprint TEXT NOT NULL DEFAULT ''`, `inferred_from TEXT NOT NULL DEFAULT '[]'`, `success_count INTEGER DEFAULT 0`, `inferred_at TEXT NULL`. Unique index on `(agent, fingerprint)` for dedup.
- New configs: `skill_inference_min_sessions = 3` (1..=20), `skill_inference_tool_name_similarity_threshold = 1.0` (future-proofing for fuzzy fingerprinting; default strict)

**Phase 17 vs Phase 23 ownership (addresses Claude I2, Codex H4):**
- **Phase 17 (Protocol):** user-declared process rules. Input = explicit `Remember()` memories of type `Preference` OR with `"Behavioral:"` title prefix OR with process-signal content ("always", "never", "must", "require"). Output = `memory(type='protocol')` rows. Represents "user says I should do X."
- **Phase 23 (Skill):** demonstrated reusable agent capabilities. Input = `session_tool_call` observations. Output = `skill` table rows. Represents "agent has successfully done Y across sessions."
- **Overlap handling:** a recurring pattern can produce BOTH a Protocol (from user statement) AND a Skill (from agent observation). These are distinct attributions and both are kept. If either row's `topic` overlaps the other's by ≥ 0.8 Jaccard similarity, an `'informed_by'` edge is created between them for observability, but neither row is deleted or merged.
- **CompileContext rendering:** `<active-protocols>` for Phase 17 output (unchanged); new `<skills>` section for Phase 23 output (greenfield; budget-accounted). Agent sees both when applicable.

**Out of scope:** Skill retirement, success_count updates, fuzzy fingerprinting, cross-agent skill attribution, LLM-based skill naming.

### 2A-4d — Forge-Identity Benchmark

**What ships:**
- In-process harness file `crates/daemon/src/bench/forge_identity.rs` (matches `forge_consolidation.rs` pattern)
- Bench config struct with seed, output dir, 6 expected-score fields, 6 per-dim minimums
- 6 dataset generators (one per dimension), each producing deterministic synthetic scenarios via ChaCha20 seed + SHA-256 token pattern
- 6 audit functions computing per-dimension scores
- Composite scorer producing `IdentityScore { composite, dimensions[6], per_dim_minimums_met[6], infrastructure_checks[], pass }`
- CLI subcommand in `forge-bench.rs`: `forge-bench forge-identity --seed N --output DIR [--expected-composite 0.95]`
- Integration test at `crates/daemon/tests/forge_identity_harness.rs`
- Calibration loop producing 1.0 composite on 5 seeds (expect real daemon bugs surfaced during calibration — count is emergent, not a target)
- Results doc at `docs/benchmarks/results/forge-identity-YYYY-MM-DD.md`
- Master summary doc at `docs/benchmarks/forge-identity-master-summary.md` — ties narrative across a/b/c1/c2/d for the product story (not gated on LongMemEval)

---

## 6. Infrastructure assertions (bench gate)

Before any dimension is scored, bench asserts the following prerequisites exist. Any failure = immediate FAIL with diagnostic output.

1. `identity` table schema has columns {id, agent, facet, description, strength, source, active, created_at, user_id}
2. `disposition.rs:MAX_DELTA` compiles to exactly `0.05` (compile-time constant assertion via `const_assert!(MAX_DELTA == 0.05);`)
3. `memory` table has columns `valence_flipped_at TEXT NULL`, `flipped_to_id TEXT NULL`, `reaffirmed_at TEXT NULL` (post 2A-4a and 2A-4b)
4. `Request::FlipPreference`, `Request::ListFlipped`, `Request::ReaffirmPreference` variants exist with correct field shapes (post 2A-4a, 2A-4b)
5. `Request::RecordToolUse`, `Request::ListToolCalls` variants exist (post 2A-4c1)
6. Config values: `preference_half_life_days` ∈ 1..=365, `skill_inference_min_sessions` ∈ 1..=20 (post 2A-4b, 2A-4c2)
7. `session_tool_call` table exists with specified columns and per-session/per-agent indexes (non-unique — tool calls can repeat) (post 2A-4c1)
8. `skill` table has columns `agent`, `fingerprint`, `inferred_from`, `success_count`, `inferred_at`; unique index on `(agent, fingerprint)` (post 2A-4c2)
9. Phase 23 is registered in the consolidator and executes after Phase 17 — verified by sourcing a compile-time check or a `Request::ProbePhase { phase_name: "infer_skills_from_behavior" }` probe (exact mechanism locked in 2A-4c2 design per §13)
10. `CompileContext` response XML contains `<preferences>` element (present or empty — D4 resolved: always emit, even empty) (post 2A-4b)
11. `CompileContext` response XML contains `<preferences-flipped>` element (may be absent if empty) (post 2A-4a)
12. `CompileContext` response XML: after seeding a Phase 23 skill (via `RecordToolUse` ≥ 3 sessions + ForceConsolidate), `<skills>` contains at least one `<skill>` child with the seeded skill's identifying token — verifies Phase 23 rows actually surface (not just that the element exists) (post 2A-4c2)
13. `touch()` exemption for preferences implemented in `db/ops.rs:touch()` SQL predicate (`AND memory_type != 'preference'`); verified by parity test confirming preference accessed_at does not update across a Recall call
14. `recall.rs:404-413` recency pattern `result.score *= 1.0 + recency_boost * 0.5` has been replaced by direct type-dispatched multiplier `result.score *= recency_factor(memory)` — verified by source-level assertion that the string `"1.0 + recency_boost * 0.5"` does not appear and `recency_factor` is called

---

## 7. Harness architecture

Matches Forge-Consolidation:

- **In-process:** `DaemonState::new(":memory:")` + direct `handle_request()` calls
- **No subprocess, no HTTP** — bench tests the library, not the binary (recovery/persistence characteristics already covered by Forge-Persist)
- **Deterministic seeds:** ChaCha20 RNG from `u64` parameter; all randomness derives from this
- **Synthetic embeddings:** 768-dim unit vectors via Gram-Schmidt perturbation (shared helper from `forge_consolidation.rs` — refactored into `common.rs` if not already there)
- **Content token strategy:** SHA-256 hex tokens in memory content to avoid Phase 2 semantic dedup catching bench fixtures (Forge-Consolidation lesson — explicitly applied to **every** dimension's generators, including Dim 3 and Dim 6 where same-topic preferences are deliberately similar)
- **Semantic similarity spec for Dim 6 (addresses Claude I5):** All four preferences in Dim 6 share identical embedding vector `v_pref` (not semantically similar — literally identical). Query embedding `v_q` has `cosine_similarity(v_q, v_pref) = 0.95` (computed via controlled Gram-Schmidt perturbation). This isolates ranking differences to the recency factor, not embedding noise.
- **Consolidator-run policy for scoring (addresses Claude I11):** Dim 3 and Dim 6 score BEFORE any consolidator run (so Phase 4 decay can't pre-fade test fixtures). Dim 1, Dim 2, Dim 4, Dim 5 score AFTER a single `Request::ForceConsolidate` run (so consolidator interactions like Phase 9a contradiction diagnostics and Phase 23 skill elevation are measurable).

---

## 8. Non-goals (explicit, expanded)

- **Multi-user isolation** — different `user_id` values — Phase 2A-6 Forge-Transfer will own that
- **Real session logs** from Claude Code — synthetic only for reproducibility/privacy
- **LLM-extracted preferences** — use explicit `Remember()` calls; extraction quality is a separate concern
- **Cross-agent identity** — facets shared across agent instances — scope explicitly to single agent (`claude-code`)
- **Emergent behavioral patterns** from real agent actions — scripted tool-use only via `RecordToolUse`; real-observation testing deferred to when Claude Code hook plumbing is in place
- **Disposition traits beyond caution/thoroughness** — Autonomy / Verbosity / Creativity exist in the daemon but are not scored in this bench
- **Preference staleness beyond 180 days** — cap simulation at 6 months; 1-year+ extrapolation is not tested
- **Auto-flip heuristics** — Phase 9a remains a diagnostics path; flipping is user/agent-initiated via `FlipPreference` only (deferred automation)
- **Bench self-contamination via retrieval feedback (NEW)** — preference `accessed_at` is exempt from `touch()`; bench also never re-reads a row via Recall between seed and score. Any feature that re-introduces self-contamination must be documented as a pending issue.
- **LongMemEval dominance as a pass gate** — downgraded to narrative goal per Codex M3; results doc may reference it, but bench pass/fail does not depend on beating MemPalace on paraphrased-preference accuracy
- **Parity-check-free bench-only hooks (NEW)** — every bench-only helper (`StepDispositionOnce`, `backdate_memory_timestamp`) must have a parity test confirming it calls the same underlying production logic as the normal request/worker path
- **Skill retirement / success_count updates** — deferred; Phase 23 only creates skill rows
- **Valence flipping on non-preference memory types** — only `MemoryType::Preference` rows are flippable

---

## 9. Deliverables per sub-phase

Each of 2A-4a / 2A-4b / 2A-4c1 / 2A-4c2 ships:

1. Detailed design doc at `docs/superpowers/specs/2026-04-17-<name>-design.md`
2. Two adversarial reviews (Claude + codex CLI) on the design
3. Implementation plan at `docs/superpowers/plans/2026-04-17-<name>.md`
4. TDD cycles via `superpowers:subagent-driven-development` with per-task spec + code-quality reviews
5. **Parity tests** for any bench-only hooks introduced (addresses Codex H9)
6. Dogfood run on live daemon (rebuild + restart + verify)
7. Memory handoff file noting feature-complete status + any known gaps
8. **Schema migration rollback recipe** — reverse-DDL script for the migration, tested in a fresh DB (addresses Claude N8)

2A-4d additionally ships:

9. Bench design doc at `docs/benchmarks/forge-identity-design.md`
10. Implementation plan at `docs/superpowers/plans/2026-04-17-forge-identity-bench.md`
11. Results doc at `docs/benchmarks/results/forge-identity-YYYY-MM-DD.md`
12. Master summary doc at `docs/benchmarks/forge-identity-master-summary.md` — narrative across a/b/c1/c2/d

---

## 10. Success criteria

- All 4 features (a/b/c1/c2) ship with TDD, `cargo clippy --workspace -- -W clippy::all -D warnings` clean, `cargo test --workspace` green
- Forge-Identity bench composite ≥ 0.95 across all 5 seeds AND every dimension ≥ its per-dim minimum on every seed
- All 14 infrastructure assertions pass on every seed
- Parity tests green for every bench-only hook
- Calibration loop terminates (1.0 composite on all 5 seeds) with bench-driven improvements documented in results doc
- Master summary tells the "memory is identity" story end-to-end with reproducible commands
- Each sub-phase dogfoods cleanly on the user's live daemon before moving to the next
- LongMemEval single-session-preference comparison row included in results doc for narrative — NOT gated, no numeric threshold required

---

## 11. Known risks (expanded)

- **2A-4c2 Phase 23 complexity** — canonical fingerprint + deduplication is the biggest unknown. If the 8-12 task estimate blows up, split c2 further (detection heuristic vs elevation logic).
- **`touch()` exemption side effects** — exemption lives in `db/ops.rs:touch()` SQL predicate (§3). Audit during 2A-4b for any other mutation path that updates `accessed_at` outside `touch()` (e.g., direct UPDATEs in other ops functions); ensure they all respect the preference exemption.
- **Type-dispatched recency interactions with graph expansion** — `recall.rs:279-280` RRF fusion, then graph expansion, then post-RRF recency. If graph expansion surfaces a preference via graph traversal, the type-dispatched recency still applies. Test: ensure graph-surfaced preferences decay by the same rule as query-matched ones.
- **Auto-flip deferral** — we're explicitly NOT building auto-detection in 2A-4a. If product/UX later wants auto-flip, Phase 9a must gain a confidence score (D1 returns as a future decision).
- **Bench timing sensitivity** — Dim 6 calibrated ratio bands are narrow. If Phase 4 decay OR universal recency changes formula in a future sprint, bench breaks. Mitigation: bench tracks the formula parameters in its summary.json so a regression is traceable.
- **Schema churn cost** — this phase adds ≥ 6 new columns across `memory`, `skill`, plus a new `session_tool_call` table. Migration order matters. Mitigation: each sub-phase's migration is independently reversible.
- **Retrieval feedback risk (revisited)** — `touch()` exemption resolves the immediate issue for preferences, but other memory types still self-refresh. Document as non-goal; future Phase 2A-n may exempt all types.
- **Dim 6 ratio calibration may need adjustment after first bench run** — the v4 bands ([1.85, 2.00], [40.5, 45.5], [81, 91]) are derived from the theoretical values 1.9034, 43.0688, 86.1376 with ~2–5% margin on each side to absorb floating-point + RRF rank-ordering noise. If 2A-4d calibration observes ratios drifting further, bands should be re-calibrated then locked. Summary.json records observed ratios per-seed for regression traceability.

---

## 12. Open decisions (v2 — after review resolution)

Resolved in v2:
- ~~D1 — Auto-flip threshold~~ → **RESOLVED:** no auto-flip in 2A-4; explicit API only
- ~~D4 — CompileContext preferences section~~ → **RESOLVED:** always emit `<preferences>` (even empty) to satisfy assertion 10; `<preferences-flipped>` and `<skills>` omitted when empty
- ~~D5 — Flipped-memory ranking~~ → **RESOLVED:** flipped memories filtered by default in recall; surfaced with `include_flipped: true` query param

Still open, to be resolved during sub-phase design:
- **D2 — Preference half-life (2A-4b):** 14 days default. Could be 7 (aggressive) or 30 (conservative). Decide during 2A-4b design + potentially tuned during 2A-4d calibration.
- **D3 — Skill inference min sessions (2A-4c2):** 3. Could be 2 (eager) or 5 (cautious). Decide during 2A-4c2 design.
- **D6 — Bench temporal simulation depth:** **RESOLVED:** 180 days fixed per non-goal §8; no 2+ year simulation. Cap is enforced in bench generators.
- **D7 — Recency composition order (2A-4b):** **RESOLVED:** Post-RRF multiplicative, applied uniformly to graph-expanded rows (§3 line 51 and §5 2A-4b). Graph-surfaced preferences decay under the same rule as query-matched ones.
- **D8 — Parity test pattern (2A-4a, 2A-4b, 2A-4c1):** What's the idiom? (Recommend: each bench-only hook has a `#[test]` that calls both the hook and the production path with matching inputs, asserts output equivalence. Lock in 2A-4a.)

---

---

## 13. Sub-phase resolution index

All non-master-level findings from v2 adversarial review are assigned here to the sub-phase whose detailed design doc must resolve them. Each resolution must reference the finding and show the chosen resolution before the sub-phase's design-gate passes.

**Resolve in 2A-4a (Valence Flipping) detailed design:**
- `supersede_memory()` helper extraction from existing `handler.rs:718-768` inline SQL (Claude N-H3). First task of 2A-4a is to refactor-extract the helper, then FlipPreference calls it.
- `flipped_to_id` vs `superseded_by` overlap semantics for pref flips (Claude I1 partial). Decide: always identical for pref flips, or divergent?
- XML emit policy consistency across `<preferences>` (always emit), `<preferences-flipped>` (omit empty), `<skills>` (omit empty) — either align all three or document rationale for the split (Claude N-H6, Codex L1).

**Resolve in 2A-4b (Recency-weighted Decay) detailed design:**
- `touch()` exemption architectural layer — must be in `db/ops.rs:touch()` with SQL predicate `AND memory_type != 'preference'`, NOT in `writer.rs` (which doesn't see memory_type) (Claude N-H1).
- Non-preference decay rate constants: reconcile `db/ops.rs:562` (0.03 for fader, uses accessed_at) vs `recall.rs:412` (0.1 for ranker, uses created_at). Pick correct rates, document them, ensure v3 master quotes the right numbers (Claude N-H8).
- Graph-expanded result recency composition (open decision D7) — recommend yes, apply same type-dispatched multiplier (Codex PARTIAL [6]).

**Resolve in 2A-4c1 (Tool-use schema) detailed design:**
- `session_tool_call` uniqueness: align table definition (non-unique indexes) with infrastructure assertion 7 (which required "unique index"). Decision: drop the "unique" word from assertion 7 — tool calls can repeat (Codex H4).
- `user_correction_flag` producer specification: either (a) Claude Code hook heuristic marks at record time, (b) new `Request::FlagToolUseCorrection { tool_call_id }` retrofits, or (c) explicit bench-only seeding. Lock one (Claude N-H5).
- `user_correction_flag` row-level vs session-level: Phase 23 filter "no user_correction_flag=1 rows in the sessions in question" means any corrected tool call poisons its entire session for skill inference. Decide: keep session-level (permissive) or narrow to "sequence-adjacent only" (strict) (Codex H5).
- `id TEXT PRIMARY KEY` ID scheme for `session_tool_call`: specify ULID to match existing `memory.id` convention (Claude N-M5).

**Resolve in 2A-4c2 (Phase 23) detailed design:**
- Canonical fingerprint sequence/multiplicity: `sha256(sort(unique(tool_names)) + sort(tool_arg_shapes))` loses order and count. Decide: is order-preserving fingerprint better (e.g., `sha256(tool_sequence_in_order + arg_shape_sequence)`) or is unordered acceptable for the bench's use case? Trade-off documented in 2A-4c2 design (Claude N-H10, Codex H1).
- `templated_name(fingerprint)` definition — pin exact format, e.g., `format!("skill-{domain}-{}", &fingerprint[0..12])` (Claude N-H10, Codex unaddressed).
- `infer_domain(tool_names)` definition — pin exact rule, e.g., "first tool_name if homogeneous, else 'mixed'" (Claude N-H10).
- `phase_registry()` enforceability — either (a) refactor `run_all_phases` to expose a `Vec<PhaseFn>` registry (expands 2A-4c2 scope), or (b) replace assertion 9 with a source-level check via `include_str!` + pattern matching, or (c) add a runtime probe request `Request::ProbePhase { phase_name: "infer_skills_from_behavior" }` (Claude N-C2, Codex unaddressed).
- `informed_by` edge between Protocol and Skill rows at ≥ 0.8 topic Jaccard: define `topic` (recommend: lowercased title token set, stop-words removed, from `memory.title` or `skill.name`), define Jaccard tokenization, define edge storage location (recommend: existing `edge` table with `edge_type='informed_by'`) (Claude N-H2, Codex H6).
- `<skills>` renderer update (per master mandate above): lock the chosen resolution path (drop `success_count>0` filter, OR set `success_count` at insert).
- Phase 17 current behavior misdescription in v2 master: verify actual Phase 17 query and update master (Claude PARTIAL [8]).

**Resolve in 2A-4d (Forge-Identity Bench) detailed design:**
- Per-dimension DB isolation: each dim generator uses its own `DaemonState::new(":memory:")` instance, not a shared DB — prevents Dim 5 ForceConsolidate from polluting Dim 3/6 fixtures (Claude N-H7, Codex M3).
- Disposition worker bench fixtures: exact `session` row specs with `started_at` / `ended_at` timestamps, duration patterns (short <5min, long >30min) per cycle to drive short/long ratio computation. Spec `StepDispositionOnce { synthetic_sessions: Vec<SessionFixture> }` API or equivalent (Claude I3, Codex unaddressed).
- "Session" semantics in bench: memory-grouping only (session_id is a label for grouping memories by simulated session), not touching `session` table persistence (Claude I4, Codex unaddressed).
- `MAX_DELTA` visibility for const_assert: make `pub(crate)` in disposition.rs, import in bench (Claude N-H4).
- Bench-isolation invariant: "No generator calls Request::Recall or Request::CompileContext before scoring" — enforce via instrumented handle_request in bench mode (Claude N-M1).
- Dim 1 identity worker control: use `DaemonState::new_test()` (if exists; if not, introduce) that does not start workers (Claude N-M3).

**Resolve in any sub-phase (flexible):**
- `ReaffirmPreference` non-preference validation: ReaffirmPreference must validate `memory_type = 'preference'` like FlipPreference does. Add to 2A-4b task list (Codex M2).
- Migration rollback recipe acceptance criteria: "forward-migrate, populate 1 row per new column, rollback, verify rollback runs cleanly" (Claude N-M6).
- SHA-256 token pattern per-dimension enforcement: each dimension's generator documents its token usage in a tripwire comment (Codex Part C).

---

## Changelog

- **v1 (2026-04-17, commit 059be8d):** Initial master design.
- **v2 (2026-04-17, commit 084cc68):** Addresses 10 CRITICAL findings from first-pass adversarial reviews.
- **v3 (2026-04-17, commit 0cc369e):** Addresses v2 master-level blockers (5 items). Introduced 2 CRITICAL regressions flagged by third-pass review: (a) `preference_fade_threshold` removal was incomplete (still referenced in 3 places); (b) Dim 6a ratio math was numerically wrong (stated 1.950/41.95/83.90 vs true 1.9034/43.07/86.14).
- **v4 (2026-04-17, this revision):** Fixes v3 regressions + tightens code-reality grounding.
  - `preference_fade_threshold` **completely removed** from §3, §5 (2A-4b config list), and §6 (infrastructure assertion 6). Only hard-fade exemption remains, which is sufficient.
  - Dim 6a ratios **corrected** to true values (1.9034, 43.0688, 86.1376) with calibrated bands [1.85, 2.00], [40.5, 45.5], [81, 91] that allow symmetric ~3–6% drift.
  - RRF-identity fixture spec **strengthened** — prefs must share identical `title` strings (ensures identical BM25 ranks) in addition to identical embeddings.
  - `touch()` exemption location **corrected** in §3 from `writer.rs` to `db/ops.rs:touch()` with SQL predicate (the only sound architectural layer; `writer.rs` doesn't see memory_type).
  - Non-preference recency formula **corrected** from `exp(-0.03 × days)` to `exp(-0.1 × days_since_created)` (matches actual code at `recall.rs:412-413`).
  - Recency composition structure **pinned**: 2A-4b replaces the existing `result.score *= 1.0 + recency_boost * 0.5` envelope with a direct multiplicative factor (change documented so 2A-4b regression-guard against Forge-Context/Forge-Consolidation has a concrete "before/after" to compare).
  - Infrastructure assertion 12 **tightened** from "`<skills>` element exists" to "`<skills>` contains a Phase 23 seeded skill's token after ForceConsolidate" — prevents silent Dim-5 failure.
  - Infrastructure assertion 14 **tightened** to source-level check that the old `1.0 + recency_boost * 0.5` pattern is replaced.
  - §11 Known Risks **updated** from stale v2 bands (1.4–2.5, 2.0–10, 2.0–15) to current v4 bands with proper rationale.
  - §12 D6 and D7 **resolved** with references to their master-level resolutions.
  - §13 finding assignment preserved; assertion 7 "unique index" softened to "non-unique indexes" (tool calls repeat; see §13 line 303 for rationale).

- **v2 (2026-04-17, commit 084cc68):** Addresses 10 CRITICAL findings from Claude + codex adversarial reviews.
  - Split 2A-4c into 2A-4c1 (tool-use schema) + 2A-4c2 (Phase 23), addressing Claude C1 (session_message wrong target).
  - Anchored decay to `created_at`/`reaffirmed_at`, added `touch()` exemption for preferences (Claude C4, Codex C2).
  - Added `preference_fade_threshold` for soft-fade exemption (Claude C5).
  - Replaced Dim 6 with 4-point gradient test with calibrated ratio bands (Claude C6, Codex H2).
  - Locked recency × RRF composition as post-RRF type-dispatched multiplicative (Claude C7, Codex H3).
  - Dropped auto-flip "confidence" language; explicit API only (Claude C2, C8, Codex C1).
  - Reclassified `<preferences>` and `<preferences-flipped>` as greenfield XML sections (Claude C3).
  - Added per-dim minimums (Codex H7).
  - Specified Phase 17 vs Phase 23 ownership boundary (Claude I2, Codex H4).
  - Resolved D5 (flipped memory recall) at master level; added `include_flipped` Recall param (Codex H8).
  - Added parity-test requirement for bench-only hooks (Codex H9).
  - Added canonical fingerprint spec for skill deduplication (Codex H5).
  - Specified semantic similarity model for Dim 6 (Claude I5).
  - Specified consolidator-run policy for scoring (Claude I11).
  - Specified Dim 1 strength tolerance ±0.001 (Claude I10).
  - Downgraded LongMemEval dominance to narrative goal (Codex M3).
  - Added bench self-contamination non-goal (Codex M4).
  - Added schema migration rollback recipe to deliverables (Claude N8).
  - Added `ReaffirmPreference` Request variant for user-initiated recency reset (addresses the semantic gap around "user restates preference").
