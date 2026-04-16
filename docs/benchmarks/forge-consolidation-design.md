# Forge-Consolidation — design gate document

**Status:** DRAFT — design gate, pending founder approval AND adversarial review. No implementation begins until both gates pass.

**Scope:** Phase 2A-3 of [phase-2-plan.md](./phase-2-plan.md) — third of six custom Forge-* benchmarks.

**Predecessor:** Forge-Context (Phase 2A-2) landed 2026-04-16 with all 7 quality gates green, 1.0 composite across all 5 seeds, 3 daemon bugs caught and fixed (0.83→0.93→1.00 across three calibration cycles). See [forge-context results](./results/forge-context-2026-04-16.md).

---

## 1. Thesis

"A memory system that only stores is a database. Forge's 22-phase consolidation cycle must demonstrably IMPROVE retrieval quality over time — deduplication reduces noise without losing signal, reweave enriches old memories with new context, contradiction detection resolves conflicting information, quality scoring correctly prioritizes high-value memories, and the net effect is measurably better recall."

Forge-Consolidation tests the **#1 differentiator in every elevator pitch**: self-healing memory that gets smarter while you sleep. The 22-phase consolidation loop is what no competitor has. If consolidation does not demonstrably improve quality, the extraction pipeline is architectural cost that should be cut (per the Phase 2 plan's original question).

**Why this is third:** Forge-Persist (2A-1) proved the substrate works (crash durability). Forge-Context (2A-2) proved the primary value proposition works (proactive intelligence). Forge-Consolidation (2A-3) proves the self-healing narrative — that memory improves over time rather than just accumulating. It is the bench that separates Forge from every "RAG with SQLite" competitor.

**What this bench does NOT test:** crash durability (Forge-Persist), proactive intelligence precision (Forge-Context), identity persistence (Forge-Identity), multi-agent coordination (Forge-Multi), tenant isolation (Forge-Transfer), embedding model quality (LongMemEval/LoCoMo), extraction pipeline correctness (bench seeds data directly, no extraction). Each of those has its own bench or is explicitly out of scope for this benchmark suite.

---

## 2. Reconnaissance summary

Empirical facts from a reconnaissance pass over the daemon's consolidation system. File:line citations are load-bearing.

1. **Entry point** (`crates/daemon/src/workers/consolidator.rs:66`): `pub fn run_all_phases(conn: &Connection, config: &ConsolidationConfig) -> ConsolidationStats` executes all 22 phases sequentially on a single write connection. Returns aggregated per-phase counts via `ConsolidationStats` struct (lines 28-50).

2. **Trigger paths** (`consolidator.rs:1664`, `server/handler.rs:ForceConsolidate arm`): Three ways to run consolidation — (a) periodic background worker every `config.workers.consolidation_interval_secs` (default 1800 = 30 min), (b) on-demand via `Request::ForceConsolidate`, (c) once during daemon startup. All three invoke the same `run_all_phases` function.

3. **22-phase sequence** (`consolidator.rs:73-470`):
   - **Phase 1** (`line 73`): `ops::dedup_memories(conn)` — exact dedup via `ROW_NUMBER() PARTITION BY title, memory_type ORDER BY confidence DESC, created_at DESC` keeping rank 1. Returns deletion count.
   - **Phase 2** (`line 84`): `ops::semantic_dedup(conn, batch_limit)` — word-overlap merge. Formula: `weighted*0.5 + max(title_score, content_score) > 0.65` → supersede lower-confidence copy, create `supersedes` edge.
   - **Phase 3** (`line 95`): `ops::link_related_memories(conn, batch_limit)` — creates `related_to` edges for memory pairs sharing ≥2 tags.
   - **Phase 4** (`line 106`): `ops::decay_memories(conn, batch_limit)` — exponential decay `confidence * exp(-0.03 * days_old)`; memories with `effective_confidence < 0.1` transition to `faded` status.
   - **Phase 5** (`line 117`): `ops::promote_recurring_lessons(conn, batch_limit)` — clusters of 3+ lessons with >50% title overlap same project → Pattern memory with `confidence + 0.1` (capped 1.0), originals superseded.
   - **Phase 6** (`line 128`): `ops::find_reconsolidation_candidates(conn)` — memories with `access_count >= 5` get `confidence + 0.05` (capped 1.0). Returns top 5 by access count.
   - **Phase 7** (`line 154`): `ops::embedding_merge(conn)` — KNN search (k=10) on `memory_vec`; pairs with distance < 0.1 (cosine similarity > 0.9) and same type merge via `supersedes` edge. Batch 200 to limit N+1.
   - **Phase 8** (`line 165`): `ops::strengthen_active_edges(conn)` — edges where both endpoints accessed in last 24h get `strength + 0.1` (capped 1.0) stored in properties JSON.
   - **Phase 9a** (`line 177`): `ops::detect_contradictions(conn)` — valence-based: opposite valence + ≥2 shared tags + intensity > 0.5 → contradiction diagnostic + `contradicts` edge.
   - **Phase 9b** (`line 207`, `consolidator.rs:505`): `detect_content_contradictions(conn)` — content-based: same type, title Jaccard ≥ 0.5, content Jaccard < 0.3 → contradiction diagnostic + edge. Bounded to 5000 pairs.
   - **Phase 10** (`line 214`): `ops::decay_activation_levels(conn)` — `activation_level *= 0.95`; values < 0.01 zeroed out.
   - **Phase 11** (`line 224`): `crate::db::manas::detect_entities(conn)` — extracts proper nouns from titles, upserts entity rows.
   - **Phase 12** (`line 235`, `consolidator.rs:687`): `synthesize_contradictions(conn, batch_limit)` — for valence contradictions, creates resolution memory with title `"Resolution: {i} vs {j}"`, content `"Previously: {content_i}. Later: {content_j}. Later supersedes earlier."`, union of tags + "resolution", confidence `max(conf_i, conf_j)`. Originals marked `superseded` in IMMEDIATE transaction.
   - **Phase 13** (`line 242`, `consolidator.rs:846`): `detect_and_surface_gaps(conn)` — words appearing ≥3 times in titles without an entity row → perception with `kind='knowledge_gap'`, 24h TTL.
   - **Phase 14** (`line 249`, `consolidator.rs:905`): `reweave_memories(conn, batch_limit, reweave_limit)` — same type + project + org + ≥2 shared tags; appends newer to older as `"{older}\n\n[Update]: {newer}"`, marks newer `merged`. IMMEDIATE transaction. Capped at `reweave_limit` (default 50) per cycle.
   - **Phase 15** (`line 256`, `consolidator.rs:1064`): `score_memory_quality(conn, batch_limit)` — computes `quality_score = freshness*0.3 + utility*0.3 + completeness*0.2 + activation*0.2`. Formulas below.
   - **Phase 16** (`line 263`): `ops::classify_portability(conn, batch_limit)` — assigns portability labels to unknown memories.
   - **Phase 17** (`line 273`, `consolidator.rs:1145`): `extract_protocols(conn, batch_limit)` — two-tier: (Tier 1) all active preferences; (Tier 2) patterns with behavioral signals. Rust-side validation: content must contain process signal (`always`, `never`, `must`, `require`, `workflow`, `rule:`, or title starts with `behavioral:`) AND NOT observation signal (`discovered`, `observed that`, `validates`, `proved that`, `user goal`, `user dogfoods`, `pipeline`, `test pattern:`). Creates Protocol with confidence 0.8, quality 0.7, `promoted_to` edge.
   - **Phase 18** (`line 280`, `consolidator.rs:1249`): `tag_antipatterns(conn, batch_limit)` — lessons with negative signals (`don't`, `avoid`, `caused problem`, `broke`, `revert`, `never`, `bug found`, `fail`, or title with `don't`/`avoid`/`pitfall`) get `anti-pattern` tag appended.
   - **Phase 19a** (`line 289`): protocol suggestion notification if `protocols > 0`, throttled 3600s.
   - **Phase 19b** (`line 311`): contradiction alert notification if `contradictions > 0`, throttled 1800s.
   - **Phase 19c** (`line 331`): quality decline warning if `AVG(quality_score)` for last 7 days < 0.3, throttled 86400s.
   - **Phase 19d** (`line 365`): meeting timeout synthesis for meetings older than `config.meeting.timeout_secs`.
   - **Phase 20** (`line 455`, `consolidator.rs:1320`): `heal_topic_supersedes(conn, healing_config)` — KNN search (k=6) same type; cosine_similarity > 0.65 (distance < 0.35) AND word overlap in `[0.3, 0.7)` AND older confidence < 0.95 → older marked `superseded`. Logs to `healing_log`. Multi-tenant: same `organization_id` only.
   - **Phase 21** (`line 463`, `consolidator.rs:1547`): `heal_session_staleness(conn, healing_config)` — aggressive: `quality_score < 0.1 AND access_count = 0 AND age > 3 days`; normal: `quality_score < staleness_min_quality (0.2) AND access_count = 0 AND age > staleness_days (7 days)` → `faded`.
   - **Phase 22** (`line 470`, `consolidator.rs:1621`): `apply_quality_pressure(conn, healing_config)` — accelerated decay: `quality < 0.3 AND access_count = 0` → `quality -= max(0.15, decay_per_cycle)`; normal decay: `quality >= 0.3 AND access_count = 0` → `quality -= decay_per_cycle (0.1)`; boost: `access_count > 0 AND accessed_at > now - 1 day` → `quality += boost_per_access (0.05)` (capped 1.0).

4. **Quality score formula** (`consolidator.rs:1064`):
   ```
   freshness    = clamp(1.0 - (age_days / 7.0) * 0.1, 0.1, 1.0)
   utility      = clamp(access_count / 10.0, 0.0, 1.0)
   completeness = min(content.len() / 200.0, 1.0)
   activation   = clamp(activation_level, 0.0, 1.0)
   quality      = freshness*0.3 + utility*0.3 + completeness*0.2 + activation*0.2
   ```

5. **Protocol types** (`crates/core/src/protocol/request.rs:323`, `response.rs:440-459`):
   - Request: `ForceConsolidate` (unit variant, no params)
   - Response: `ResponseData::ConsolidationComplete` with 14 `usize` fields matching ConsolidationStats (4 marked `#[serde(default)]` for backward compat: synthesized, gaps_detected, reweaved, scored).

6. **ConsolidationStats struct** (`consolidator.rs:28-50`): 19 public usize fields covering exact_dedup, semantic_dedup, linked, faded, promoted, reconsolidated, embedding_merged, strengthened, contradictions, entities_detected, synthesized, gaps_detected, reweaved, scored, protocols_extracted, antipatterns_tagged, healed_superseded, healed_faded, healed_quality_adjusted.

7. **Configuration** (`crates/daemon/src/config.rs`):
   - `ConsolidationConfig { batch_limit: 200, reweave_limit: 50 }` — clamped `[1, 1000]` and `[1, 500]`.
   - `HealingConfig { enabled: true, cosine_threshold: 0.65, overlap_low: 0.3, overlap_high: 0.7, staleness_days: 7, staleness_min_quality: 0.2, quality_decay_per_cycle: 0.1, quality_boost_per_access: 0.05, batch_limit: 200 }`.

8. **Memory table columns** (`crates/daemon/src/db/schema.rs`):
   - Base: `id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, organization_id`
   - Added via migrations: `activation_level REAL DEFAULT 0.0`, `valence TEXT NOT NULL DEFAULT 'neutral'`, `intensity REAL NOT NULL DEFAULT 0.0`, `quality_score REAL DEFAULT 0.5`, `access_count`, `memory_type_extra`.

9. **Edge table** (`schema.rs`): `{ id, from_id, to_id, edge_type, properties (JSON), created_at, valid_from, valid_until }`. Edge types used by consolidation: `supersedes`, `related_to`, `contradicts`, `promoted_to`.

10. **Status transitions** (consolidator-wide invariant): `active` → `superseded | faded | merged | deleted`. Never back to `active`. Consolidation never UPDATEs `active` to `active`.

11. **Memory_vec virtual table** (sqlite-vec extension): Synthetic embeddings can be INSERTed directly via `INSERT INTO memory_vec (rowid, embedding) VALUES (?, ?)` where rowid matches memory's rowid. Requires the extension to be loaded in `DaemonState` initialization.

12. **Semantic dedup thresholds** (`ops.rs:1283`): `meaningful_words()` filters lowercase alphanumeric ≥3 chars. `(title_jaccard * 0.5 + content_jaccard * 0.5) * 0.5 + max(title_jaccard, content_jaccard) > 0.65` with stopwords filtered.

13. **Reweave transaction pattern** (`consolidator.rs:905`): IMMEDIATE transaction per pair to avoid TOCTOU. Re-reads older content inside transaction to prevent concurrent modification loss.

14. **Notification throttling** (`consolidator.rs:289-331`): Checks `MAX(created_at)` in notification table for topic-specific dedup. Clean in-memory DB has no prior notifications, so first run always fires if conditions met.

15. **Healing log** (`consolidator.rs:1320`): Phases 20-22 log entries to `healing_log` table for diagnostics. Includes candidate memory IDs, similarity scores, decisions made.

16. **Multi-tenant isolation** (consolidator-wide): All cross-memory operations (dedup, reweave, promotion, contradictions, topic supersede) check `organization_id` match. Default org is `"default"`.

17. **Existing bench harnesses** (`crates/daemon/src/bench/mod.rs`): `forge_persist.rs` (subprocess), `forge_context.rs` (in-process), `longmemeval.rs` (in-process), `locomo.rs` (in-process), `common.rs` (shared helpers: `bytes_to_hex`, `seeded_rng`, `sha256_hex`), `scoring.rs` (shared retrieval metrics). Forge-Context is the pattern to follow.

18. **Handler bypass rationale** (`server/handler.rs` ForceConsolidate arm): The handler arm is a thin wrapper that calls `run_all_phases(conn, &config)` and serializes the ConsolidationStats into the response. The bench can call `run_all_phases` directly for speed — it is the same code path.

---

## 3. Core architectural commitment: in-process harness with synthetic embeddings

**The single most important design decision:** Forge-Consolidation runs in-process using `DaemonState` with an in-memory SQLite database. Synthetic embeddings are INSERTed directly into `memory_vec` to test embedding-dependent phases (7, 20) with controlled distance values.

**Rationale:**
- We are measuring **consolidation logic correctness and quality improvement**, not **embedding pipeline quality** or **HTTP transport**. The embedding model and its training are tested elsewhere (LongMemEval, LoCoMo). The HTTP path is tested by 1441+ unit/integration tests.
- Synthetic embeddings let us plant near-duplicates at precise cosine distances (0.08 for merge, 0.15 for control) to validate exact threshold behavior. Real embeddings would introduce model-dependent noise and break determinism.
- In-process runs complete in <10 seconds with ~150 memories across 22 phases. Subprocess runs with embedding-worker polling would take minutes and be flaky.
- Forge-Context already proved the in-process pattern at 1.0 composite on a similarly complex scoring problem. Follow what works.

**The tradeoff we accept:** synthetic embeddings don't test the real embedding pipeline. This is accepted because:
1. Embedding pipeline quality is covered by LongMemEval and LoCoMo.
2. The consolidation code at Phases 7 and 20 operates on `memory_vec` rows — it does not care where vectors came from. It computes cosine distance and compares against thresholds. Synthetic vectors at controlled distances test this logic precisely.
3. A **separate real-embedding integration test** (`forge_consolidation_integration.rs`) spawns a real daemon, seeds data via HTTP, waits for the embedding worker, and validates Phases 7+20 with model-produced vectors. This is standalone pass/fail — NOT part of the scored composite. It catches embedding pipeline regressions without polluting bench determinism.

---

## 4. Dataset shape

All data is generated deterministically from a `seed: u64` via ChaCha20 PRNG, following the Forge-Context and Forge-Persist patterns. The corpus is seeded into a single project and organization (`default`) to isolate multi-tenant effects to their own tests.

**Total corpus: ~167 memories across 8 categories, each designed to trigger specific phases with controllable ground truth.**

### Category 1: Exact duplicates (12 memories → 6 pairs)

6 pairs with identical `(title, type)` but different confidence levels (e.g., 0.9 vs 0.7). Phase 1 should keep the higher-confidence copy and DELETE the other.

- 2 decision pairs, 2 lesson pairs, 2 pattern pairs
- Titles include full 64-char SHA-256 token derived from `(seed, pair_index)` to ensure no accidental collision with other categories
- **Ground truth:** 6 deletions, 6 survivors (the higher-confidence member of each pair)

### Category 2: Semantic near-duplicates (16 memories → 8 pairs)

8 pairs with paraphrased titles sharing >0.65 Jaccard word overlap by the consolidation semantic-dedup formula. Same type and project. Different confidence levels.

- Example: "Always run tests before deploying services" vs "Run tests before deploying every service always"
- Pairs designed to be paraphrases but NOT exact duplicates (would be caught by Phase 1 first)
- Word overlap tuned to > 0.65 threshold with margin
- **Ground truth:** 8 superseded (lower confidence), 8 active survivors, 8 `supersedes` edges

### Category 3: Embedding near-duplicates (12 memories → 4 merge pairs + 2 control pairs)

- 4 pairs with synthetic embeddings at cosine distance < 0.1 (similarity > 0.9), same type — should merge via Phase 7
- 2 control pairs at cosine distance 0.15 (similarity ≈ 0.85), same type — should NOT merge
- Titles designed to have <0.65 Jaccard word overlap so Phase 2 does NOT catch them — tests that Phase 7 catches what word-overlap misses
- **Ground truth:** 4 merged (lower confidence → superseded), 4 survivors, 2 control pairs both active, 4 `supersedes` edges

### Category 4: Contradictions (16 memories → 8 pairs)

- 4 valence-based pairs (Phase 9a): opposite valence ({positive, negative}), ≥2 shared tags, intensity > 0.5. Phase 12 should synthesize resolutions.
- 4 content-based pairs (Phase 9b): same type, title Jaccard ≥ 0.5, content Jaccard < 0.3.
- **Ground truth:**
  - 8 contradiction detections (diagnostics + `contradicts` edges)
  - 4 synthesis resolutions (new Resolution memory + both originals superseded + `supersedes` edges)
  - 4 content contradictions remain as diagnostics only (Phase 12 only synthesizes valence contradictions)

### Category 5: Reweave and enrichment candidates (30 memories)

- 10 reweave pairs (Phase 14): same type, project, org, ≥2 shared tags, different `created_at` — 20 memories total (10 older + 10 newer)
- 4 preferences with process signals ("always X", "never Y") for Phase 17 Tier 1 — 4 memories
- 3 patterns with `behavioral:` prefix + process signal for Phase 17 Tier 2 — 3 memories
- 3 lessons with negative signals ("don't", "avoid", "caused problem") for Phase 18 — 3 memories
- **Ground truth:** 10 reweaves (newer marked `merged`, older content appended with `"[Update]:"`), 7 new Protocol memories created (4 + 3) with `promoted_to` edges, 3 anti-pattern tags added

### Category 6: Lifecycle and quality (31 memories)

- 6 with old `created_at` (30+ days) for decay testing (Phase 4) — expected decay to known confidence values per formula — 6 memories
- 5 with `access_count >= 5` for reconsolidation (Phase 6) — confidence boost +0.05 — 5 memories
- 4 clusters of 3 lessons (12 lessons total) with >50% title overlap for promotion (Phase 5) — 4 new Pattern memories expected — 12 memories
- 8 with varied quality dimensions (access_count, age, content length, activation) for quality scoring validation (Phase 15) — expected `quality_score` per formula with ±0.01 tolerance — 8 memories
- **Ground truth:** 6 decayed confidences, 5 reconsolidated boosts, 4 pattern promotions (12 source lessons superseded), 8 computed quality scores (all tolerance ±0.01)

### Category 7: Self-healing targets (24 memories)

- 6 pairs for topic-aware auto-supersede (Phase 20): synthetic embeddings at distance < 0.35, word overlap 0.3-0.7, same type/org, older confidence < 0.95 — 12 memories (6 older + 6 newer)
- 6 low-quality (`quality_score < 0.2`), zero-access, 7+ day old memories for staleness fade (Phase 21) — 6 memories
- 6 mixed (Phase 22): 3 low-quality unaccessed (accelerated decay ≥0.15), 3 recently-accessed (quality boost +0.05) — 6 memories
- **Ground truth:** 6 topic-superseded (older members of pairs), 6 staleness-faded, 3 quality-decayed, 3 quality-boosted, 6+ `healing_log` entries with `action = 'topic_supersede'`

### Category 8: Infrastructure and side-effects (26 memories)

- 5 pairs sharing ≥2 tags for edge linking (Phase 3) — 10 memories, expected at least 5 new `related_to` edges
- 5 with `activation_level > 0.01` for activation decay (Phase 10) — 5 memories, expected activation × 0.95
- 8 with proper nouns in titles (e.g., "Kubernetes pods behavior", "PostgreSQL locking") for entity detection (Phase 11) — 8 memories, expected entity rows created
- 3 unknown portability memories for Phase 16 — 3 memories, expected portability labels assigned
- **Ground truth (pass/fail):** ≥5 `related_to` edges created, all 5 activations reduced to ~0.95× original, ≥N_expected entity rows (N computed from planted proper nouns), all 3 portability labels no longer 'unknown'

### Synthetic embedding generation

- Embeddings are deterministic unit vectors of dimension 384 (matching daemon's production dimension)
- For each memory: base vector computed from `seed_memory_id_hash`
- For near-duplicate pairs: second vector = base_vector + small perturbation such that cosine distance is in target range
- Perturbations use ChaCha20Rng from the seed for reproducibility
- `memory_vec` rows inserted AFTER memory rows using explicit rowid

### Recall query bank (for pre/post improvement delta)

~15 recall queries targeting information that should survive consolidation. Examples:

1. Query for a concept that appears in multiple exact duplicates — post-consolidation should return fewer, higher-confidence results
2. Query for a topic with contradictions — post-consolidation should return the resolution memory, not the conflicting originals
3. Query for a tag that appears in reweave candidates — post-consolidation should return enriched (longer) content
4. Query for a behavioral pattern — post-consolidation should return the newly-promoted Protocol memory
5. Query for a high-access memory — post-consolidation should rank it higher (boosted confidence + quality)

Queries are run via `Request::Recall { layer: None, query, k: 10 }` through `handle_request`. Expected ground truth is the set of memory titles that SHOULD appear in top-10.

---

## 5. Scoring rubric

5 scored dimensions mapped to the Phase 2A-3 plan, with weighted composite. 3 infrastructure phase groups evaluated as pass/fail assertions that gate the overall bench.

### Dimension 1: Dedup quality (weight 0.25)

Covers Phases 1 (exact dedup), 2 (semantic dedup), 7 (embedding merge).

- **Dedup precision** = `|superseded_or_deleted ∩ GT_duplicates| / |superseded_or_deleted|`
- **Dedup recall** = `|superseded_or_deleted ∩ GT_duplicates| / |GT_duplicates|`
- **F1** = harmonic mean of precision and recall
- **Signal preservation gate:** For every memory in `GT_unique` (memories NOT marked as duplicates), assert `status = 'active'` post-consolidation. If ANY unique memory is superseded/deleted, dimension score = 0 regardless of F1.
- **Score** = F1 (only if signal preservation gate passes, else 0.0)

### Dimension 2: Contradiction handling (weight 0.20)

Covers Phases 9a (valence contradictions), 9b (content contradictions), 12 (synthesis).

- **Detection precision** = `|detected_contradiction_pairs ∩ GT_contradictions| / |detected|`
- **Detection recall** = `|detected_contradiction_pairs ∩ GT_contradictions| / |GT_contradictions|`
- **Detection F1** = harmonic mean
- **Synthesis accuracy** = `|GT_valence_contradictions where resolution memory exists AND both originals superseded| / |GT_valence_contradictions|`
- **Score** = 0.5 * detection_F1 + 0.5 * synthesis_accuracy

Detected contradiction pairs are extracted from `diagnostic` table entries with type `contradiction` and from `edge` table entries with `edge_type = 'contradicts'`.

### Dimension 3: Reweave and enrichment (weight 0.15)

Covers Phases 5 (promotion), 14 (reweave), 17 (protocol extraction), 18 (anti-pattern tagging).

- **Reweave F1** = F1 over pairs where newer memory's `status = 'merged'` AND older memory's content contains `"[Update]:"` append
- **Promotion accuracy** = `|GT_lesson_clusters where new Pattern memory exists AND source lessons superseded| / |GT_lesson_clusters|`
- **Protocol accuracy** = `|GT_process_memories where Protocol copy exists AND promoted_to edge created| / |GT_process_memories|`
- **Anti-pattern accuracy** = `|GT_negative_lessons where 'anti-pattern' tag added| / |GT_negative_lessons|`
- **Score** = 0.30 * reweave_F1 + 0.25 * protocol_accuracy + 0.25 * antipattern_accuracy + 0.20 * promotion_accuracy

### Dimension 4: Quality lifecycle (weight 0.15)

Covers Phases 4 (decay), 6 (reconsolidation), 10 (activation decay), 15 (quality scoring), 21 (staleness fade), 22 (quality pressure).

Each memory in lifecycle categories has an expected post-consolidation value (confidence, quality_score, activation_level, or status). Assertions are binary (correct within ±0.01 tolerance for floats, exact match for status).

- **Decay correctness** = `|old_memories where abs(observed - expected) <= 0.01| / |old_memories|`
- **Reconsolidation correctness** = `|high_access_memories where confidence increased by ~0.05| / |high_access_memories|`
- **Quality score accuracy** = `|Q memories where abs(observed - expected) <= 0.01| / |Q memories|`
- **Activation decay** = `|activated memories where observed ≈ expected * 0.95| / |activated|`
- **Staleness fade** = `|stale candidates where status = 'faded'| / |stale candidates|`
- **Quality pressure** = `|pressure candidates where quality delta matches expected sign and magnitude| / |pressure candidates|`
- **Score** = unweighted mean of the 6 sub-accuracies (each already a fraction)

### Dimension 5: Recall improvement delta (weight 0.25) — HEADLINE METRIC

The bench that proves the thesis: consolidation improves recall.

- **Pre-consolidation recall@10** = run all queries on noisy corpus, compute `mean(|retrieved ∩ expected| / |expected|)`
- **Post-consolidation recall@10** = same queries after consolidation
- **Delta** = post - pre
- **Expected delta** = set during calibration (first calibration locks the target)
- **Normalized score** = `clamp(delta / expected_delta, 0.0, 1.0)`
- **Score** = normalized_score

If delta is negative (consolidation made recall worse), score = 0 and this triggers investigation per bench-driven improvement loop methodology.

### Infrastructure assertions (pass/fail gate, not weighted)

Covers Phases 3 (linking), 8 (edge strengthening), 11 (entity detection), 13 (gap detection), 16 (portability), 19a-d (notifications), 20 (topic supersede, also in Dimension 1).

- Phase 3 (linking): `count(edge where edge_type = 'related_to') >= 5` (from 5 Category 8 pairs sharing ≥2 tags)
- Phase 8 (strengthen): At least one `related_to` edge has `strength >= 0.2` in properties JSON. Requires Category 8 linking pairs to have `accessed_at` within 24h of `now` — enforced at seed time.
- Phase 11 (entities): `count(entity) >= 5` (8 Category 8 proper-noun memories may collapse if they share the same proper noun; assertion conservatively requires ≥5 unique entities)
- Phase 13 (gaps): `count(perception where kind = 'knowledge_gap') >= 1`. With 167 memories sharing common words in titles, organic gaps will surface; bench does not plant dedicated Phase 13 memories.
- Phase 16 (portability): No memory with `portability = 'unknown'` survives where the seed had set it to 'unknown' (all 3 Category 8 portability memories classified)
- Phase 19a (protocol notification): notification row with topic `protocol_suggestion` exists because Phase 17 promoted ≥1 protocol (Category 5 has 7 process memories)
- Phase 19b (contradiction alert): notification row with topic `contradiction` exists because Phase 9a+9b detected contradictions (Category 4 has 8 pairs)
- Phase 19c (quality decline): No quality decline warning row is created. Verified by: post-consolidation `AVG(quality_score)` for memories with `created_at > now - 7 days` ≥ 0.3 (corpus is seeded majority-healthy)
- Phase 19d (meeting timeout): No meeting rows seeded, so no timeout synthesis or alert. Assertion: `count(notification where topic LIKE '%meeting%') = 0`
- Phase 20 (topic supersede, also Dimension 1): `count(healing_log where action = 'topic_supersede') >= 6` (from 6 Category 7 pairs)

**If ANY infrastructure assertion fails, overall bench = FAIL regardless of composite score.**

### Composite

```
composite = 0.25 * dedup_quality
          + 0.20 * contradiction_handling
          + 0.15 * reweave_enrichment
          + 0.15 * quality_lifecycle
          + 0.25 * recall_improvement
```

### Pass thresholds (to be set during calibration)

Following the Forge-Context precedent, initial composite is expected below 1.0. Actual thresholds locked in after first calibration. Per bench-driven improvement loop methodology: investigate per-dimension failures, TDD fix the daemon (if a bug) or the bench (if ground truth error), re-calibrate until scores stabilize. Three cycles is normal.

---

## 6. Harness architecture

Single-function entry point following the Forge-Context pattern:

```rust
pub fn run(config: ConsolidationBenchConfig) -> Result<ConsolidationScore, String> {
    let state = DaemonState::new(":memory:")?;
    load_sqlite_vec_extension(&state.conn)?;

    let dataset = seed_corpus(&state, config.seed)?;
    let baseline = snapshot_recall_baseline(&state, &dataset.recall_queries)?;

    let stats = consolidator::run_all_phases(&state.conn, &default_consolidation_config());

    let audit = audit_state_transitions(&state, &dataset)?;
    let post = snapshot_recall_post(&state, &dataset.recall_queries)?;

    let score = compute_score(&baseline, &post, &audit, &stats);
    write_artifacts(&config.output_dir, &score, &stats, &baseline, &post)?;
    Ok(score)
}
```

### Key design decisions

1. **Direct `run_all_phases` invocation:** The bench calls `consolidator::run_all_phases(&state.conn, &config)` directly rather than going through `Request::ForceConsolidate`. The handler arm is a thin wrapper over this function — same code path. Skipping the handler saves HTTP/serde overhead.

2. **Synthetic embeddings for Phases 7 and 20:** After `seed_corpus` inserts memory rows, the bench inserts synthetic embeddings into `memory_vec` with carefully tuned cosine distances. The extension is loaded at `DaemonState` initialization time.

3. **Timestamp manipulation:** Decay, staleness, and reconsolidation phases depend on `created_at` and `accessed_at`. Bench seeds explicit past timestamps (e.g., `created_at = now - 30 days` for decay candidates). No real-clock dependency.

4. **Config overrides:** `batch_limit` set high (e.g., 500) to ensure all 150 memories are processed in one pass. `reweave_limit` set to 50+ to cover all planted reweave pairs. Default healing config otherwise.

5. **Notification baseline:** In-memory DB starts empty, so Phase 19 notifications fire if conditions met without throttle interference.

6. **Tool table:** Unlike Forge-Context, consolidation does not interact with the tool table. No clearing needed.

7. **Multi-seed calibration:** `forge-bench forge-consolidation --seed N` runs the full pipeline with seed N. Sweep across 5 seeds `{1, 2, 3, 42, 100}` following Forge-Context pattern.

### File structure

```
crates/daemon/src/bench/
├── common.rs                          — shared helpers (existing)
├── forge_consolidation.rs             — main harness (~2500 lines)
├── forge_context.rs                   — existing
├── forge_persist.rs                   — existing
├── mod.rs                             — add consolidation module

crates/daemon/src/bin/forge-bench.rs   — add ForgeConsolidation CLI subcommand

crates/daemon/tests/
├── forge_consolidation_harness.rs       — in-process integration test (scored)
├── forge_consolidation_integration.rs   — real-embedding integration test (pass/fail, NOT scored)

docs/benchmarks/
├── forge-consolidation-design.md                       — this doc
├── results/forge-consolidation-YYYY-MM-DD.md          — results doc post-calibration
```

### Output artifacts

Per-run directory `bench_results_consolidation_seed{N}/` contains:
- `summary.json` — composite score + per-dimension breakdown + per-phase stats
- `baseline.json` — pre-consolidation recall results
- `post.json` — post-consolidation recall results
- `audit.json` — per-phase ground truth assertions and outcomes
- `stats.json` — `ConsolidationStats` from `run_all_phases`
- `repro.sh` — exact reproduction command

---

## 7. Ground truth strategy

All ground truth is computed from the seed deterministically. No hardcoded literal seeds anywhere.

### GroundTruth annotation

Each memory gets a `GroundTruth` struct attached at seed time:

```rust
struct GroundTruth {
    memory_id: String,
    category: Category,              // which of the 8 categories
    expected_phases: Vec<u8>,        // which phase numbers should act on this memory
    expected_status: ExpectedStatus, // Active | Superseded | Faded | Merged | Deleted
    duplicate_of: Option<String>,    // partner in exact/semantic/embedding dedup pair
    contradicts: Option<String>,     // contradiction partner
    reweave_source: Option<String>,  // newer memory that should enrich this one
    expected_quality: Option<f64>,   // ±0.01 tolerance
    expected_confidence: Option<f64>,// ±0.01 tolerance
    expected_activation: Option<f64>,// ±0.01 tolerance
}
```

`SeededDataset` carries the seed + all GroundTruth records + recall query bank + expected edge counts.

### Phase ordering awareness

Consolidation runs phases 1→22 sequentially. Earlier phases mutate state that later phases operate on. Ground truth models the **cascading post-phase state**, not initial state. Specific cascades:

- Phase 1 DELETEs rows → Phase 2's word-overlap search sees a smaller set
- Phase 2 supersedes → Phases 7, 14, 20 skip superseded rows
- Phase 9 creates contradiction edges → Phase 12 reads those edges
- Phase 15 sets quality_score → Phases 21, 22 filter on quality_score
- Phase 17 creates Protocol memories → Phase 19a notification fires
- Phase 9 creates contradictions → Phase 19b notification fires
- Phase 20 supersedes → healing_log populated for Phase 20 assertion

The dataset design isolates categories (exact-dup pairs are NOT also in semantic-dup pairs) to minimize accidental cross-phase interference. Where cascades are intentional (9→12, 17→19a), ground truth follows the cascade.

### Response format coupling prevention

Consolidation phases operate on DB state, not response strings. But verification queries the DB via SQL and via `Request::Recall`. Ground truth uses:
- Memory status (string: "active", "superseded", "faded", "merged")
- Memory titles (for recall results)
- Confidence and quality_score values (with tolerance)
- Edge types and counts (`supersedes`, `related_to`, `contradicts`, `promoted_to`)
- Diagnostic and notification table rows

All values are checked against actual daemon behavior via unit tests BEFORE locking ground truth.

---

## 8. Gotcha prevention (learned from Forge-Persist and Forge-Context)

| Gotcha | Prevention |
|--------|-----------|
| Semantic dedup eats test data | Full 64-char SHA-256 token in titles of memories that should NOT be deduped. Near-duplicate pairs use controlled word overlap. |
| Ground truth format coupling | Verify actual daemon output format via unit tests BEFORE locking ground truth (learned from Forge-Context guardrails fix). |
| Hardcoded seed values | All ground truth derives from `dataset.seed`. Never use literal "42" in generator strings (learned from Forge-Context CRITICAL-100). |
| Phase ordering cascade errors | Ground truth models post-cascade state per §7 above. Dataset categories isolated to prevent accidental cross-phase interference. |
| Batch limit truncation | Config `batch_limit = 500` (above corpus size). Default 200 would miss memories. |
| Tool table contamination | N/A — consolidation does not touch tool table. |
| Reweave TOCTOU | Reweave uses IMMEDIATE transactions in daemon. Bench does not attempt to race. |
| Notification throttling | Clean in-memory DB has no prior throttle entries. First run fires. |
| Embedding merge prerequisites | synthetic embeddings INSERTed AFTER memory rows. Extension loaded at DaemonState initialization. |
| Activation decay floor | Bench activations set > 0.01 to avoid being zeroed out by the `<= 0.01 → 0` rule. |
| Organization isolation | All memories use `organization_id = 'default'`. Cross-org isolation tested by Forge-Transfer. |
| Protocol deduplication | Phase 17 skips protocols with existing-same-title rows. Bench ground truth accounts for this — seed process memories with unique titles. |
| Anti-pattern idempotency | Phase 18 checks if tag already present. Bench seeds negative-signal lessons WITHOUT the tag. |
| Quality scoring tolerance | Float comparisons use `abs(observed - expected) <= 0.01` — not exact equality. |
| Content-based contradiction O(n²) bound | Phase 9b limits to 5000 pairs. Bench seeds 4 content contradictions in a controlled subset — well below bound. |
| Reweave append format | `"{older_content}\n\n[Update]: {newer_content}"` — ground truth checks for `"[Update]:"` substring. |
| `healing_log` schema | Phase 20 writes to `healing_log` with specific columns. Bench queries this table for assertions — verify schema before writing queries. |
| Phase interaction: access_count | `access_count` affects Phase 6 (reconsolidation boost), Phase 15 (utility sub-score), Phase 21 (staleness fade filter), Phase 22 (quality pressure boost). Duplicate pair members must have `access_count < 5` so Phase 6 does not perturb Phase 7 merge tiebreakers. Lifecycle candidates are isolated in Category 6 with specific access_count values. |
| Phase interaction: quality_score | Phase 15 sets `quality_score`. Phases 21 and 22 filter on it. Dataset must ensure Dimension 4 sub-assertions remain distinguishable after cascade — staleness candidates use quality < 0.2, pressure accelerated-decay candidates use quality < 0.3, boost candidates use quality > 0.3 with recent access. |
| Phase 8 dependency on recent access | Phase 8 strengthens edges where both endpoints were accessed in the last 24h. Category 8 linking pairs are seeded with `accessed_at` within the last hour of `now` to guarantee Phase 8 has work to do. |
| Phase 11 entity collapse | If multiple memories mention the same proper noun (e.g., "Kubernetes" in 2 memories), `detect_entities` produces a single entity row with merged mention counts — not one per memory. Infrastructure assertion uses ≥5 unique entities, not ≥8 memories. |

---

## 9. Non-goals (explicit)

- **Real embedding quality:** tested by LongMemEval and LoCoMo. Synthetic embeddings test consolidation LOGIC, not vector quality.
- **HTTP transport correctness:** tested by 1441+ existing tests.
- **Multi-agent coordination:** Forge-Multi territory.
- **Crash durability during consolidation:** Forge-Persist territory. Consolidation here runs synchronously to completion.
- **Meetings (Phase 19d):** no meetings seeded. Phase 19d asserted to be a no-op on this corpus.
- **Portability correctness beyond "labels assigned":** Forge-Transfer territory.
- **Concurrent consolidation safety:** single-threaded bench execution.
- **Extraction pipeline:** bench seeds data via direct SQL. No extraction tested.

---

## 10. Quality gate plan

Following the same 7-gate sequence as Forge-Persist and Forge-Context:

1. **Design gate** — this doc + adversarial review + founder approval. No implementation before green.
2. **TDD gate** — every function starts with a failing test. No production code without RED.
3. **Clippy + fmt gate** — zero warnings, workspace test suite passing.
4. **Adversarial review gate** — codex review on design + dataset generator + full pipeline. Fix all findings ≥ confidence 80.
5. **Documentation gate** — results doc with honest calibration journey (expect initial < 1.0).
6. **Reproduction gate** — `forge-bench forge-consolidation --seed 42` runs clean from checkout.
7. **Dogfood gate** — run bench via CLI + `forge doctor` to verify daemon health.

### Bench-driven improvement loop (mandatory)

Per [`feedback_bench_driven_loop.md`](../../../../../.claude/projects/-Users-dsskonuru-workspace-playground-forge/memory/feedback_bench_driven_loop.md):

1. First calibration will score below 1.0. That is the bench doing its job.
2. Investigate per-dimension scores to find which dimension is low.
3. Trace expected vs actual for failing queries. Is this a daemon bug (fix) or ground truth error (fix bench)?
4. TDD fix daemon → re-calibrate → verify score improves.
5. Repeat until stable. Three cycles is normal.
6. Only defer truly structural improvements (e.g., "replace LIKE with FTS5").

Proven on Forge-Context: 0.83 → 0.93 → 1.00 across three cycles. Two daemon bugs found, one ground truth error fixed.

---

## 11. Implementation plan summary

7-task plan following Forge-Context structure:

1. **Task 1** — Extend `bench/common.rs` if needed (likely no changes — hex/RNG/SHA helpers already extracted)
2. **Task 2** — Dataset generator: 8 category generators + `seed_corpus()` orchestrator + `SeededDataset` struct
3. **Task 3** — Synthetic embedding generator + `memory_vec` insertion
4. **Task 4** — Recall query bank generator + baseline/post snapshot helpers
5. **Task 5** — Audit functions (one per dimension): state transition checks, quality computations, ground truth comparisons
6. **Task 6** — Scoring functions: precision/recall/F1 helpers (can reuse from forge_context.rs), per-dimension aggregators, composite calculator
7. **Task 7** — Orchestrator `run()` + CLI subcommand + integration test (both in-process scored and real-embedding standalone)

Detailed plan produced by writing-plans skill after this design is approved.

---

## 12. Open decisions

### D1 (resolved): Synthetic vs real embeddings for the scored bench

**Resolved:** Synthetic embeddings for the scored bench (determinism + threshold precision). Real-embedding integration test as separate pass/fail safety net. Rationale in §3.

### D2 (resolved): 5 scoring dimensions vs 22 per-phase scores

**Resolved:** 5 scoring dimensions per Phase 2A plan mapping, with infrastructure phases as pass/fail assertions. All 22 phases validated, but composite reflects product-meaningful dimensions. Rationale in §5 and prior brainstorming.

### D3 (resolved): In-process harness

**Resolved:** In-process following Forge-Context pattern. Direct `run_all_phases` invocation. Rationale in §3.

### D4 (resolved): Corpus size

**Resolved:** ~150 memories across 8 categories, sized for completeness not minimalism. Every phase has enough test cases to distinguish signal from noise.

### D5 (resolved): Pre/post recall uses `Request::Recall`

**Resolved:** Recall baseline and post snapshots use `Request::Recall { layer: None, query, k: 10 }` through `handle_request` (not direct SQL). This tests the retrieval path that customers actually use.

---

## 13. Summary

**What this bench proves:** Forge's 22-phase consolidation loop is not architectural cost. It demonstrably (a) reduces noise via dedup without losing signal, (b) detects and resolves contradictions, (c) enriches older memories via reweave, (d) promotes process knowledge to Protocols and tags anti-patterns, (e) correctly scores and maintains memory quality over time, and (f) produces measurably better recall than the raw noisy corpus.

**What it does not prove:** embedding model quality (LongMemEval), HTTP correctness (existing tests), crash durability during consolidation (Forge-Persist), multi-agent safety (Forge-Multi), tenant isolation (Forge-Transfer), extraction correctness (direct SQL seeding).

**Next step after this doc is approved:** adversarial review (codex) → founder approval → writing-plans skill produces implementation plan → TDD cycles → calibration → daemon fixes → 1.0 → dogfood.
