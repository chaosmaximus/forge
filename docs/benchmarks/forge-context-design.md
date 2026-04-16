# Forge-Context — design gate document

**Status:** DRAFT — design gate, pending founder approval AND adversarial review. No implementation begins until both gates pass.

**Scope:** Phase 2A-2 of [phase-2-plan.md](./phase-2-plan.md) — second of six custom Forge-* benchmarks.

**Predecessor:** Forge-Persist (Phase 2A-1) landed 2026-04-15 with all 7 quality gates green. Housekeeping cycle landed 2026-04-16 (Version endpoint, HttpClient timeout, session pagination, Doctor observability). See [forge-persist results](./results/forge-persist-2026-04-15.md).

---

## 1. Thesis

"The agent thinks. Forge does everything else."

Forge-Context tests the "everything else" — specifically, whether the daemon surfaces the **right procedural knowledge at the right moment** in the agent lifecycle. This is not a text-search benchmark. It tests whether the daemon's multi-path proactive intelligence system produces contextually appropriate results across the hook events that drive a real agent session.

**Why this is second:** Forge-Persist proved the substrate works (crash durability). Forge-Context proves the primary value proposition works (proactive intelligence). If the daemon can't surface the right skill when an agent edits a file, the right lesson when an agent claims "done," or the right warning when an agent runs a destructive command — the entire product narrative ("Forge does everything else") falls apart.

**What this bench does NOT test:** crash durability (Forge-Persist), consolidation quality (Forge-Consolidation), identity persistence (Forge-Identity), multi-agent coordination (Forge-Multi), tenant isolation (Forge-Transfer), LLM extraction quality (standard benchmarks). Each of those has its own bench.

---

## 2. Reconnaissance summary

Empirical facts from a reconnaissance pass over the daemon's proactive intelligence stack. File:line citations are load-bearing.

1. **Prajna matrix** (`crates/daemon/src/proactive.rs:39-62`): Bootstrap relevance scores for `(hook_event, knowledge_type)` pairs. 9 hook events × 7 knowledge types. Threshold 0.3. Learned overrides after ≥5 samples from `context_effectiveness` table. Knowledge types: `blast_radius`, `anti_pattern`, `uat_lesson`, `decision`, `test_reminder`, `skill`, `notification`.

2. **CompileContext** (`crates/daemon/src/recall.rs:519-612` static prefix, `1020-1120` dynamic suffix): 9-layer XML context assembly. Static prefix includes tools (count + top-10 names). Dynamic suffix includes skills filtered by tool availability via `available_tool_names()`. Focus parameter restricts to semantically related items via FTS5 MATCH.

3. **Tool-availability filtering** (`crates/daemon/src/recall.rs:1061-1097`): CompileContext's dynamic suffix filters skills via a HARDCODED list of 12 tool keywords (`docker`, `kubectl`, `terraform`, `npm`, `cargo`, `pip`, `gcloud`, `aws`, `ssh`, `make`, `scp`, `rsync`). For each keyword: if the skill's text (name+description+domain) contains the keyword AND the tool name is NOT in `available_tool_names()`, the skill is excluded. **CRITICAL:** `available_tool_names()` returns ALL tool names from the `tool` table regardless of health status (`manas.rs:541-546`). "Unavailable" in the filtering sense means "not registered in the DB at all," NOT `health = Unavailable`. Graceful degradation: if NO tools are registered, all skills pass through. Only the 12 hardcoded keywords trigger filtering — other tool names are invisible to this mechanism.

4. **PreBashCheck** (`crates/daemon/src/guardrails/check.rs:279-364`): Scans for destructive patterns, surfaces negative-valence lessons matching command name, queries skills by command name via `SELECT name, domain FROM skill WHERE success_count > 0 AND (description LIKE ? OR name LIKE ? OR domain LIKE ?) ORDER BY success_count DESC LIMIT 1`.

5. **PostEditCheck** (`crates/daemon/src/guardrails/check.rs:118-157`): Wrapper around GuardrailsCheck. Surfaces callers, lessons, skills, decisions, diagnostics. Includes Prajna proactive injection (`test_reminder` at 0.8, `skill` at 0.6 for PostEdit hook).

6. **GuardrailsCheck** (`crates/daemon/src/guardrails/check.rs:31-103`): Queries 4 layers: linked decisions (via `affects` edges), blast radius (caller count), relevant lessons + dangerous patterns (negative-valence memories), applicable skills (path-matched via `find_applicable_skills`).

7. **find_applicable_skills** (`crates/daemon/src/guardrails/check.rs:432-518`): Parses file path into search terms (filename, stem, parent dirs). SQL: `SELECT name, domain FROM skill WHERE success_count > 0 AND (description LIKE ? OR domain LIKE ? OR name LIKE ?) ORDER BY success_count DESC LIMIT 2`.

8. **CompletionCheck** (`crates/daemon/src/server/handler.rs:2166-2220`): When `claimed_done = true`, queries memories tagged with `%testing%`, `%production-readiness%`, `%anti-pattern%`, or `%uat%`. Returns top 3 by quality_score DESC, confidence DESC. Severity = "high" if any found.

9. **TaskCompletionCheck** (`crates/daemon/src/server/handler.rs:2221-2280`): Regex detects shipping tasks (`ship|deploy|release|production|merge|push`). Queries lessons tagged `%uat%`, `%production%`, or `%deployment%`.

10. **Recall with layer filter** (`crates/daemon/src/server/handler.rs:453-655`): Layer parameter routes to specific Manas layers. `layer: "skill"` returns skills as Pattern-type MemoryResults with `[skill:domain]` prefix. `layer: "domain_dna"` returns Domain DNA entries.

11. **Tool struct** (`crates/core/src/types/manas.rs:38-48`): `{ id, name, kind, capabilities, config, health, last_used, use_count, discovered_at }`. Kinds: `Cli`, `Mcp`, `Builtin`, `Plugin`. Health: `Healthy`, `Degraded`, `Unavailable`, `Unknown`.

12. **Skill struct** (`crates/core/src/types/manas.rs:55-79`): `{ id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids }`.

13. **Existing bench harnesses** (`crates/daemon/src/bench/mod.rs`): `forge_persist.rs` (subprocess), `longmemeval.rs` (in-process), `locomo.rs` (in-process), `scoring.rs` (shared metrics). LongMemEval/LoCoMo use `DaemonState::new(":memory:")` pattern — Forge-Context follows this.

14. **DaemonState construction** (`crates/daemon/src/server/handler.rs:100-113`): `DaemonState::new(":memory:")` creates a test state with `started_at: Instant::now()` and opens an in-memory SQLite connection with full schema (including FTS5 triggers that auto-populate `memory_fts` on INSERT). Used by all existing handler unit tests.

15. **Skills NOT filtered by CompileContext focus** (`crates/daemon/src/recall.rs:1062-1098`): The `focus` parameter in CompileContext restricts decisions and lessons via FTS5 MATCH on the `memory` table, but skills are fetched independently via `list_skills()` with NO focus clause. Skills are capped at `.take(5)` regardless of focus topic. **Implication:** CompileContext is NOT a valid path for testing skill-specific recall.

16. **Response format for applicable_skills** (`crates/daemon/src/guardrails/check.rs:514-515`): `find_applicable_skills` returns formatted strings `"Skill: {name} ({domain})"`, not raw skill IDs. `decisions_to_review` in `GuardrailsCheck` returns decision TITLES, not IDs. **Implication:** ground-truth matching must use formatted strings, not database IDs.

17. **FTS5 auto-population confirmed** (`crates/daemon/src/db/schema.rs:334-344`): `memory_fts_insert` trigger fires on every INSERT into the `memory` table. In-memory SQLite with schema applied has working FTS5 for focus-filtered queries on decisions/lessons.

---

## 3. Core architectural commitment: in-process harness

**The single most important design decision:** Forge-Context runs in-process using `DaemonState` with an in-memory SQLite database. No subprocess spawn, no port allocation, no HTTP transport.

**Rationale:**
- We are measuring **retrieval quality** (precision/recall of the proactive intelligence system), not **transport correctness** or **crash durability**. The handler + serde layer is tested by 1405+ unit/integration tests.
- In-process runs complete in <5 seconds. Subprocess runs take 30-60 seconds. Calibration requires many runs with different seeds — 10× speed difference compounds.
- The LongMemEval and LoCoMo harnesses already use this pattern successfully.
- Forge-Multi (Phase 2A-5) is the natural place for subprocess-based multi-agent benchmarking.

**The tradeoff we accept:** we don't test the HTTP serialization path or the tier gating. These are tested elsewhere (contract tests, tier tests, integration tests) and add no incremental signal for retrieval quality.

---

## 4. Dataset shape

All data is generated deterministically from a seed via ChaCha20 PRNG, following the Forge-Persist pattern.

### 4.1 Seeded knowledge corpus

| Category | Count | Purpose |
|----------|-------|---------|
| **Tools (present)** | 6 from the hardcoded-12 list | Registered in DB — their dependent skills pass CompileContext filter |
| **Tools (absent)** | 6 from the hardcoded-12 list | NOT registered — their dependent skills are excluded from CompileContext |
| **Skills** | 30 (20 procedural, 10 behavioral) | Test skill recall across guardrails + layer recall dimensions |
| **Memories** | 30 (10 decisions, 10 lessons, 10 patterns) | Test guardrails, completion intelligence, layer recall |
| **Domain DNA** | 5 | Test domain_dna layer recall |
| **Affects edges** | 10 (decisions → files) | Test GuardrailsCheck decision linking |
| **Test session** | 1 | Registered via `RegisterSession` for CompletionCheck queries |

Each item is generated with:
- **Unique content** — SHA-256 digest tokens in titles/descriptions to avoid semantic dedup (lesson from Forge-Persist cycle k)
- **Domain tags** — deterministic domain assignments from a vocabulary pool (e.g., "auth", "database", "networking", "testing", "deployment")
- **Ground-truth annotations** — each item has a list of queries where it SHOULD appear in results, using the EXACT response format (formatted strings, not raw IDs — see §2 item 16)

### 4.2 Tool generation

**CRITICAL design constraint (from adversarial review CRITICAL-1):** The daemon's tool-availability filter (`recall.rs:1077-1094`) uses a HARDCODED list of 12 tool keywords. "Unavailable" means the tool name is NOT in the `tool` table at all — NOT `health = Unavailable`. Only these 12 keywords trigger filtering: `docker`, `kubectl`, `terraform`, `npm`, `cargo`, `pip`, `gcloud`, `aws`, `ssh`, `make`, `scp`, `rsync`.

The dataset splits these 12 into two groups:
- **Present (6):** e.g., `cargo`, `docker`, `npm`, `ssh`, `make`, `pip` — inserted into tool table with `health: Healthy`
- **Absent (6):** e.g., `kubectl`, `terraform`, `gcloud`, `aws`, `scp`, `rsync` — NOT inserted. Skills mentioning these keywords should be filtered from CompileContext.

The split is deterministic from the seed (shuffled, then first 6 = present, last 6 = absent).

### 4.3 Skill generation

Each skill has:
- `name`: unique, domain-prefixed (e.g., `"auth-jwt-validation"`)
- `domain`: from domain vocabulary
- `description`: contains domain keywords + SHA-256 unique tokens
- `steps`: 3-5 procedural steps (procedural type) or empty (behavioral type)
- `success_count`: > 0 (required by `find_applicable_skills` SQL filter `WHERE success_count > 0`)
- `project`: None (global) — avoids project-scoping complications
- **Tool dependency**: 10 skills mention a "present" tool keyword in their description, 10 mention an "absent" tool keyword, 10 mention no hardcoded-12 keyword (always pass filter)

### 4.4 Memory generation

Decisions have:
- `memory_type: Decision`
- `title` + `content` with domain keywords
- `affects` edges to specific file paths from a file vocabulary

Lessons have:
- `memory_type: Lesson`
- `tags` containing `"testing"`, `"uat"`, `"deployment"`, `"anti-pattern"` (for CompletionCheck/TaskCompletionCheck)
- `quality_score` and `confidence` set to ensure deterministic ranking

Patterns have:
- `memory_type: Pattern`
- Domain-specific content for layer recall testing

### 4.5 Query bank

**Design constraint (from adversarial review CRITICAL-2):** CompileContext `focus` does NOT filter skills — only decisions/lessons go through the FTS5 focus clause. Skills are fetched independently and capped at 5. CompileContext is used for context assembly (decisions + tool filtering) but NOT for skill-specific recall.

**Design constraint (from adversarial review CRITICAL-3):** Response fields return formatted strings, not IDs. Ground truth must be expressed as the exact strings the daemon produces: `"Skill: {name} ({domain})"` for applicable_skills, decision titles for decisions_to_review, lesson titles for relevant_lessons.

| ID | Dimension | Endpoint | Query | Expected result set |
|----|-----------|----------|-------|---------------------|
| CA-1..CA-3 | Context assembly | `CompileContext { focus }` | 3 focus topics matching decision domains | Decisions with FTS5-matching titles/content in the compiled context XML |
| CA-4..CA-6 | Context assembly (tool filter) | `CompileContext {}` (no focus) | 3 runs with different tool registrations | Skills mentioning absent-tool keywords MUST be absent from XML; present-tool skills MUST appear |
| GR-1..GR-5 | Guardrails | `PostEditCheck { file }` | 5 file paths with known `affects` edges | Correct `decisions_to_review` (titles), `applicable_skills` (formatted strings) |
| GR-6..GR-8 | Guardrails | `PreBashCheck { command }` | 3 commands matching skill domains | Correct `relevant_skills` (skill names) |
| GR-9..GR-10 | Guardrails | `GuardrailsCheck { file, action }` | 2 files with known `affects` edges | Correct `decisions_affected` (titles), `applicable_skills` |
| CO-1..CO-3 | Completion | `CompletionCheck { session_id, claimed_done: true }` | 3 queries with seeded testing/uat/deployment lessons | Correct `relevant_lessons` (titles, top 3 by quality_score) |
| CO-4..CO-5 | Completion | `TaskCompletionCheck { session_id, task_subject }` | 2 shipping-keyword subjects ("deploy to production", "merge to main") | Correct `checklists` containing uat/production lessons |
| LR-1..LR-5 | Layer recall | `Recall { query, layer: "skill" }` | 5 domain queries using FTS-matchable keywords | Correct skills returned as MemoryResult with `[skill:domain]` prefix |
| LR-6..LR-8 | Layer recall | `Recall { query, layer: "domain_dna" }` | 3 convention queries | Correct DNA entries returned as MemoryResult |

**Total: ~28 queries with deterministic ground truth.**

**CompletionCheck session setup:** Before running CO-* queries, the harness registers a test session via `Request::RegisterSession { id: "forge-context-test", agent: "bench", project: None, cwd: None, capabilities: None, current_task: None }`. Seeded lessons have no `organization_id`, so `get_session_org_id` returns `None` and the `OR ?1 IS NULL` clause passes through all memories.

---

## 5. Scoring rubric

### 5.1 Per-query metrics

For each query `q` with expected result set `E_q` and actual result set `A_q`:

- **Precision**: `|A_q ∩ E_q| / |A_q|` (what fraction of returned items are relevant)
- **Recall**: `|A_q ∩ E_q| / |E_q|` (what fraction of relevant items were returned)

For empty responses where `|E_q| > 0`: Recall = 0.0. For responses where `|E_q| = 0`: Precision is not defined; score is 1.0 if `|A_q| = 0` (correctly returned nothing), 0.0 otherwise.

### 5.2 Per-dimension aggregates

| Dimension | Queries | Weight |
|-----------|---------|--------|
| Context assembly | CA-1..CA-8 | 0.30 |
| Guardrails | GR-1..GR-10 | 0.30 |
| Completion intelligence | CO-1..CO-5 | 0.20 |
| Layer recall | LR-1..LR-8 | 0.20 |

Per-dimension score = mean of per-query F1 scores (harmonic mean of precision and recall).

### 5.3 Tool-filter accuracy (standalone metric)

Fraction of absent-tool skills correctly excluded from CompileContext XML output. Computed across CA-4..CA-6 queries. An absent-tool skill is one whose description contains a hardcoded-12 keyword for a tool NOT registered in the DB. Score = `1 - (leaked_skills / total_absent_tool_skills_checked)`. Must be 1.00 (filtering is deterministic, not probabilistic).

### 5.4 Composite score

`composite = 0.30 × context_f1 + 0.30 × guardrails_f1 + 0.20 × completion_f1 + 0.20 × layer_recall_f1`

### 5.5 Pass thresholds

**Set during calibration** — the daemon's actual capability on this dataset determines the bar. No a priori guess. Once calibrated:
- `composite ≥ <calibrated_threshold>` (locked after first calibration)
- `tool_filter_accuracy = 1.00` (strict — tool filtering is deterministic, not probabilistic)

---

## 6. Harness architecture

### 6.1 Module location

`crates/daemon/src/bench/forge_context.rs` — new file alongside `forge_persist.rs`.

### 6.2 Shared infrastructure extraction

Forge-Persist's design doc (§12 Q7) deferred shared-module extraction to "Forge-Tool when we have a second call site." Forge-Context is that second call site. Extract into `crates/daemon/src/bench/common.rs`:

- `bytes_to_hex()` — SHA-256 hex encoding (used by both for unique content generation)
- `seeded_rng(seed: u64) -> ChaChaRng` — deterministic PRNG construction
- Scoring primitives that apply to both (if any — Forge-Context's precision/recall differs from Forge-Persist's recovery_rate)

Keep bench-specific code in the respective modules. Don't over-extract — only pull helpers that are truly shared.

### 6.3 Core types

```rust
/// Configuration for a Forge-Context benchmark run.
pub struct ContextConfig {
    pub seed: u64,
    pub tools: usize,        // total tools (half healthy, half unavailable)
    pub skills: usize,       // total skills
    pub memories: usize,     // total memories (split across types)
    pub domain_dna: usize,   // domain DNA entries
    pub output_dir: Option<PathBuf>,
}

/// Ground truth for a single query.
pub struct QueryGroundTruth {
    pub id: String,           // e.g., "CA-1"
    pub dimension: Dimension,
    pub request: Request,     // the daemon request to execute
    pub expected_ids: HashSet<String>,  // IDs that should appear in response
}

/// Scoring output for a single run.
pub struct ContextScore {
    pub context_assembly_f1: f64,
    pub guardrails_f1: f64,
    pub completion_f1: f64,
    pub layer_recall_f1: f64,
    pub tool_filter_accuracy: f64,
    pub composite: f64,
    pub per_query: Vec<QueryResult>,
}
```

### 6.4 Execution flow

```
1. Create in-memory DaemonState (DaemonState::new(":memory:"))
2. Generate corpus from seed (tools, skills, memories, domain DNA, affects edges)
3. Seed all items into the DaemonState's SQLite via direct handler calls
4. Generate query bank with ground-truth annotations
5. Execute each query via handle_request()
6. Extract result IDs from each response
7. Compute precision/recall/F1 per query
8. Aggregate per dimension
9. Compute composite + tool_filter_accuracy
10. Optionally write summary.json + repro.sh
```

### 6.5 Result extraction

Each endpoint returns results in a different shape. The harness needs a per-endpoint extractor:

| Endpoint | Response variant | Extract items from | Format |
|----------|-----------------|-------------------|--------|
| CompileContext | `CompiledContext { context }` | Parse XML dynamic suffix for `<skill>` tags + decision mentions | Skill names (raw text from XML), decision titles |
| PostEditCheck | `PostEditChecked { applicable_skills, decisions_to_review, relevant_lessons }` | All three `Vec<String>` fields | `applicable_skills`: `"Skill: {name} ({domain})"` format. Others: plain titles. |
| PreBashCheck | `PreBashChecked { relevant_skills }` | `relevant_skills: Vec<String>` | Plain skill description strings |
| GuardrailsCheck | `GuardrailsCheck { decisions_affected, relevant_lessons, applicable_skills }` | All three `Vec<String>` fields | `decisions_affected`: titles. `applicable_skills`: `"Skill: {name} ({domain})"`. |
| CompletionCheck | `CompletionCheckResult { relevant_lessons }` | `relevant_lessons: Vec<String>` | Plain lesson titles |
| TaskCompletionCheck | `TaskCompletionCheckResult { checklists }` | `checklists: Vec<String>` | Formatted checklist strings |
| Recall | `Memories { results }` | `results: Vec<MemoryResult>` | `memory.title` field on each MemoryResult |

**CRITICAL note (from adversarial review CRITICAL-3):** Ground truth must be expressed in the EXACT format the daemon produces. The generator must pre-compute expected response strings (e.g., `"Skill: auth-jwt-validation (auth)"`) and store them alongside the query, not raw IDs.

---

## 7. Integration test shape

One integration test in `crates/daemon/tests/forge_context_harness.rs`:

```rust
#[test]
fn test_context_harness_passes_on_clean_workload() {
    let config = ContextConfig {
        seed: 42,
        tools: 20,
        skills: 40,
        memories: 30,
        domain_dna: 5,
        output_dir: None,
    };
    let score = forge_context::run(config).expect("harness should not error");
    assert!(score.composite > 0.0, "composite must be positive on a seeded corpus");
    assert_eq!(score.tool_filter_accuracy, 1.0, "tool filtering must be exact");
}
```

---

## 8. CLI subcommand

Add `forge-bench forge-context` subcommand to `crates/daemon/src/bin/forge-bench.rs`:

```
forge-bench forge-context \
  --tools 20 \
  --skills 40 \
  --memories 30 \
  --domain-dna 5 \
  --seed 42 \
  --output bench_results
```

---

## 9. Reproduction

```bash
cargo build --release --bin forge-bench
./target/release/forge-bench forge-context --seed 42 --output bench_results
cat bench_results/summary.json
```

---

## 10. Limitations and honest scope

1. **Synthetic dataset, not real agent sessions.** The ground truth is hand-curated in the generator, not derived from actual Claude Code usage. Real sessions may have different distributions. The bench tests the recall MECHANISM, not the extraction QUALITY.

2. **In-process only.** Does not test HTTP transport, serde round-trips, or tier gating. These are tested elsewhere.

3. **No LLM in the scoring loop.** Precision/recall is computed via deterministic set intersection, not judge-model evaluation. This means the bench cannot detect "semantically correct but differently named" results — it requires exact ID matches.

4. **CompileContext XML parsing.** Extracting skill/decision IDs from the compiled XML context requires parsing the XML output. If the XML format changes, the parser breaks. Mitigation: pin the expected format in the test and fail loudly on format changes.

5. **Prajna matrix is NOT directly scored.** The benchmark measures end results, not intermediate relevance scores. A change to the Prajna bootstrap matrix that produces the same end results would not be detected as a regression.

6. **Session-scoped queries not tested.** `ContextRefresh` and `CompletionCheck` are session-scoped — they require a registered session. The harness seeds a test session but doesn't simulate a full multi-turn agent lifecycle.

---

## 11. Quality gates per phase-2-plan.md

1. **Design gate** — this document + adversarial review + founder approval
2. **TDD gate** — every new function RED→GREEN
3. **Clippy + fmt gate** — zero warnings
4. **Adversarial review gate** — `feature-dev:code-reviewer` on every major cycle
5. **Documentation gate** — results doc published
6. **Reproduction gate** — `repro.sh` verified
7. **Dogfood gate** — founder runs against live daemon

---

## 12. Open questions

**Q1:** Should the XML parser for CompileContext be strict (fail on format change) or lenient (best-effort extraction)?

**Recommendation:** Strict. Format changes should be caught, not silently degraded. A tripwire test pins the expected XML structure.

**Q2:** Should `CompletionCheck` queries seed a real registered session, or call the handler directly with a synthetic session_id?

**Recommendation:** Register a real test session via `Request::RegisterSession` before running completion queries. This exercises the session-scoped code path.

**Q3:** Should we extract shared helpers from Forge-Persist into `common.rs` in this cycle, or defer?

**Recommendation:** Extract in this cycle. `bytes_to_hex`, `seeded_rng`, and the output-writer helpers have two call sites now. The extraction boundary is clear.

---

## 13. Estimated implementation cycles

| Cycle | Scope |
|-------|-------|
| a | Extract shared helpers into `bench/common.rs` from forge_persist.rs |
| b | Dataset generator: tools, skills, memories, domain DNA, affects edges |
| c | Query bank generator with ground-truth annotations |
| d | Result extractors: per-endpoint response → item IDs |
| e | Scoring: precision, recall, F1, composite, tool_filter_accuracy |
| f | `pub fn run` orchestrator + CLI subcommand |
| g | Calibration sweep + results doc |
