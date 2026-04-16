# Forge Phase 2+ — best-in-class roadmap

**Status:** ACTIVE — approved by founder 2026-04-14 after Wave 1 sign-off.
**Predecessor:** [improvement-roadmap-2026-04-13.md](./improvement-roadmap-2026-04-13.md) (Wave 1 COMPLETE — hybrid BM25+KNN raw search landed in commits 401b606..e6413d5)
**Ambition:** make Forge best-in-class on both standard retrieval benchmarks AND on the axes where its extraction pipeline proves unique value.

## Background

Wave 1 of the benchmark improvement initiative closed most of the retrieval-recall gap to MemPalace:
- **LongMemEval** 500-Q R@5: 0.9520 → 0.9640 (now -0.2 pp from MemPalace 0.9660)
- **LoCoMo** 1986-QA R@10: 0.8746 → 0.9456 (now +2.2 pp OVER MemPalace bge-large hybrid)

But Forge's 8-layer extraction pipeline (Manas consolidation, semantic dedup, linking, decay, promotion, reconsolidation, entity extraction, portability, protocol) is retrieval-useless on these benchmarks — the day-1 50-Q four-mode comparison showed extract loses ~18 pp R@5 to raw, consolidation doesn't recover, hybrid-mode (raw + extract RRF) still underperforms pure raw.

**The question this plan exists to answer:** does Forge's extraction pipeline earn its existence on the non-retrieval axes the standard benchmarks don't measure? Tool recall. Identity persistence. Multi-agent coordination. Behavioral pattern extraction. Domain transfer isolation. Daemon restart persistence.

If yes, Forge has a moat that no MemPalace recipe can touch. If no, extraction is architectural cost that should be cut. Either answer is publishable and useful.

## Principles (locked)

1. **Moat first, floor second.** The five custom Forge-* benchmarks take precedence over further standard-benchmark tuning. Phase 2A runs before Phase 2B.
2. **Quality gates, not time gates.** No deliverable has a time estimate. Each has an explicit quality bar (see §"Quality gate definition" below). Phases advance when the bar is met, not when the week ends.
3. **Complete deliveries.** A benchmark isn't shipped until: design approved + code + tests + adversarial review + honest results doc + reproducible repro.sh + JSONL artifact + MEMORY index + dogfood pass.
4. **Both dimensions best-in-class.** The retrieval floor (LongMemEval, LoCoMo, ConvoMem, MemBench) must stay competitive. The moat ceiling (Forge-*) must prove unique value. Neither dimension skipped.
5. **TDD + adversarial review mandatory.** The Wave 1 pattern caught 3 real bugs (filter truncation, empty-query propagation, locomo repro.sh template). Repeat it for every Phase 2 deliverable.
6. **Audience calibration.** Every deliverable serves at least one of: open-source evaluators (reproducibility, honesty), paying customers (works, is fast, no leaks), investor deck (moat story, metrics, positioning).
7. **No speculative builds.** Later phases are committed to only after earlier phases' findings. Wave 2 is a good example — LoCoMo already overshot its target, so the bge-large decision waits until Wave 3 numbers land.
8. **Interweave where safe.** Parallel tracks are used to maintain breadth (observability runs alongside custom benches because they share the KPI schema). Serial tracks only where a real dependency exists.

## Quality gate definition

Every deliverable passes through this seven-gate sequence before being marked complete:

1. **Design gate.** Written shape (what's being built + why + how it's scored) reviewed and approved by founder BEFORE code. For benchmarks: dataset shape, scoring rubric, expected-result examples.
2. **TDD gate.** Every new function has a failing test watched RED, minimal GREEN, refactor. No production code without a failing test first.
3. **Clippy + fmt gate.** `cargo fmt --all` clean, `cargo clippy --workspace -- -W clippy::all -D warnings` zero-warnings, full library test suite passing.
4. **Adversarial review gate.** Diff reviewed by `feature-dev:code-reviewer` subagent (or equivalent). Every finding at confidence ≥ 80 either fixed or explicitly documented as accepted technical debt.
5. **Documentation gate.** Results doc published; includes setup, metrics, per-category breakdown (if applicable), reproduction command, honest limitations, comparison to published baselines (if applicable). No cherry-picking per honesty rail (plan.md §7.3).
6. **Reproduction gate.** `repro.sh` in the bench result directory recreates the run end-to-end from a clean checkout. Founder runs it once manually to confirm.
7. **Dogfood gate.** If the deliverable touches the running daemon, the founder dogfoods it for ≥ 1 calendar day before merge. Issues surfaced during dogfood block merge.

A deliverable that fails any gate returns to design-gate rework, not patchwork. No shortcuts.

---

## Phase 2A — Moat: six custom Forge-* benchmarks

Each benchmark is a complete, standalone deliverable that ships independently (results published, commit landed, memory indexed) before the next begins. Order selected for: framework risk first (validate the custom-bench framework), proactive intelligence next (highest product-narrative weight), then self-healing and identity (core differentiators), multi-agent coordination (biggest moat), and enterprise isolation last.

**Revision note (2026-04-16):** expanded from 5 to 6 benchmarks after deep product-vision analysis. Each benchmark now maps to a specific product claim from the investor/customer pitch. Forge-Tool renamed to Forge-Context (scope broadened from skill search to full proactive intelligence lifecycle). Forge-Consolidation added to prove the "self-healing memory" differentiator — the #1 claim in every elevator pitch.

### 2A-1 — Forge-Persist ✅ COMPLETE (2026-04-15)

**Product claim:** "Crash-proof substrate — local-first means nothing if the daemon can't survive a restart."

**The thesis:** "Local-first cognitive infrastructure is meaningless if the daemon cannot survive abrupt termination and replay correctly."

**Results:** 5-seed calibration sweep, all seeds 1.0/1.0 recovery/consistency, ~271ms recovery time (18× headroom on 5000ms threshold). See `docs/benchmarks/results/forge-persist-2026-04-15.md`.

**What it measures:**
- State persistence across `SIGKILL → restart` cycles
- WAL replay correctness after abrupt shutdown
- Memory-table + raw-layer + session-message integrity
- Recovery time (spawn → first healthy response)

**Scoring rubric:**
- **Recovery rate** (≥ 0.99): fraction of pre-kill acked ops visible post-restart
- **Consistency rate** (= 1.00): fraction of recovered state with byte-identical content hash
- **Recovery time** (< 5000 ms): wall time from second spawn to first health OK

**Deliverables (landed):**
- `crates/daemon/src/bench/forge_persist.rs` harness (3000+ LOC)
- `crates/daemon/src/bin/forge-bench.rs` CLI subcommand
- `docs/benchmarks/results/forge-persist-2026-04-15.md` results doc
- `crates/daemon/tests/forge_persist_harness.rs` integration tests
- 23 commits on master (cycles a–k), all 7 quality gates green

### 2A-2 — Forge-Context

**Product claim:** "Surfaces the right knowledge at the right moment — the agent thinks, Forge does everything else."

**The thesis:** "Proactive intelligence is the daemon's primary value proposition. Given a context (file being edited, command being run, agent claiming done, session being compiled), the daemon must surface the right procedural knowledge with the right relevance at the right moment in the agent lifecycle."

This bench validates the full proactive intelligence stack: Prajna matrix scoring, CompileContext assembly with focus filtering, guardrails routing, completion intelligence, and layer-specific recall. It is NOT a text-search benchmark — it tests whether the daemon's multi-path recall system produces contextually appropriate results across the 9 hook events that drive a real agent session.

**What it measures (4 scoring dimensions):**

1. **Context assembly precision** — Does `CompileContext` with `focus` filter produce relevant skills, decisions, and lessons? Does tool-availability filtering correctly exclude skills for missing tools?
2. **Proactive guardrails accuracy** — Given a file edit (`PostEditCheck`) or bash command (`PreBashCheck`), does the daemon surface the right skills, anti-patterns, and blast-radius warnings? Does `GuardrailsCheck` correctly identify decisions linked to the file?
3. **Completion intelligence** — When an agent claims done (`CompletionCheck`) or marks a shipping task (`TaskCompletionCheck`), does the daemon surface relevant testing/deployment lessons?
4. **Layer recall precision** — Does `Recall { layer: "skill" }` return the right skills? Does `Recall { layer: "domain_dna" }` return the right project conventions?

**Harness architecture:** In-process (DaemonState with in-memory SQLite). No subprocess — retrieval quality doesn't need process isolation.

**Dataset shape:**
- Deterministic seed-based generation of tools, skills, domain DNA, memories (decisions, lessons, patterns)
- Each item tagged with ground-truth contexts where it should surface
- Query bank: file paths, bash commands, completion claims, focus topics — each with expected results
- Ground-truth mapping: for each (query, hook_event) pair, the set of items that SHOULD appear in the response

**Scoring rubric:**
- **Precision@K** per recall path: fraction of top-K returned items that are in the ground truth set
- **Recall@K** per recall path: fraction of ground truth items that appear in top-K
- **Prajna relevance correctness**: does the daemon's hook_event × knowledge_type scoring produce injections at or above the 0.3 relevance threshold for the right pairs?
- **Tool-availability filtering accuracy**: when a tool is marked unavailable, do its dependent skills get correctly excluded from CompileContext?
- **Composite score**: weighted mean of per-path scores

**Pass thresholds:** set during calibration (no a priori guess — the daemon's actual capability determines the bar, then the bar is locked for regression detection).

**Open decision D1 (resolved):** Synthetic dataset, not real Claude Code logs. Real logs introduce privacy/reproducibility concerns; synthetic with deterministic seed gives clean signal.

### 2A-3 — Forge-Consolidation

**Product claim:** "Self-healing memory that gets smarter while you sleep — 22-phase consolidation is what no competitor has."

**The thesis:** "A memory system that only stores is a database. Forge's 22-phase consolidation cycle must demonstrably IMPROVE retrieval quality over time — deduplication reduces noise without losing signal, reweave enriches old memories with new context, contradiction detection resolves conflicting information, and quality scoring correctly prioritizes high-value memories."

This is the benchmark that proves the #1 differentiator in every elevator pitch. If consolidation doesn't improve quality, the extraction pipeline is architectural cost that should be cut (per the Phase 2 plan's original question).

**What it measures:**

1. **Dedup quality** — Does exact + semantic dedup reduce memory count without losing distinct information? (False positive = lost signal. False negative = noise remains.)
2. **Reweave quality** — Do memories enriched by `reweave_memories` have higher recall relevance than their pre-reweave versions?
3. **Contradiction detection accuracy** — Are contradicting memories correctly identified? Does resolution keep the right winner?
4. **Quality scoring accuracy** — Do the 4-dimension quality scores (Freshness, Utility, Completeness, Activation) correctly rank memories by value?
5. **Consolidation-then-recall improvement** — Given a noisy initial corpus, does running `ForceConsolidate` + re-querying produce measurably better recall results than querying the raw uncleaned corpus?

**Harness architecture:** In-process. Seed noisy corpus → snapshot recall baseline → run consolidation → measure recall improvement.

**Dataset shape:**
- Corpus with deliberate noise: exact duplicates, near-duplicates (paraphrased), contradictions, stale memories, high/low quality mix
- Ground-truth annotations: which pairs are duplicates, which contradict, which should survive consolidation
- Pre/post recall queries with expected ranking changes

**Scoring rubric:**
- **Dedup precision/recall** against ground-truth duplicate pairs
- **Contradiction detection F1** against ground-truth contradiction pairs
- **Recall improvement delta**: post-consolidation recall@K minus pre-consolidation recall@K (must be positive)
- **Signal preservation rate**: fraction of unique (non-duplicate) memories that survive consolidation intact

### 2A-4 — Forge-Identity

**Product claim:** "Memory is identity — agents develop persistent personality that compounds across sessions."

**The thesis:** "Memory systems that treat preferences as static lose information the moment the user changes their mind. Identity and preference-tracking is where Forge's consolidation phases (contradiction detection, reconsolidation, valence flipping) are supposed to pay off."

**This is also the bench that flips the Wave 1 single-session-preference regression story into a win on our own axis.** MemPalace's LongMemEval weakness is paraphrased preference questions; Forge-Identity is the bench where Forge should dominate.

**What it measures:**
- Time-ordered preference tracking (most-recent wins when the user changes their mind)
- Contradiction detection (opposing preferences across sessions)
- Valence flipping (like → dislike transitions handled)
- Identity facet persistence and influence on context compilation
- Disposition drift accuracy (caution/confidence ±0.05/cycle cap)
- Behavioral skill extraction ("learns how you think" — observed patterns become skills)
- Preference staleness (preferences from 6 months ago weaker than yesterday)

**Dataset shape:**
- Time-stamped conversation logs with explicit preference statements at varying points
- Preference categories: tools, workflow, communication style, code conventions
- Contradictions injected at known timestamps
- Identity facets with strength scores across multiple sessions
- Behavioral patterns repeated across sessions (should become skills)
- Question bank: "what's my current preference for X?" / "has my preference for Y changed?" / "what's my agent's expertise?"

**Scoring:** preference accuracy + temporal correctness + contradiction-resolution accuracy + identity coherence.

**Narrative gate:** results doc must include a comparison row against LongMemEval single-session-preference showing the story arc.

### 2A-5 — Forge-Multi

**Product claim:** "Multi-agent coordination that competitors literally cannot run."

**The thesis:** "Agents sharing context via FISP is a Forge concept competitors literally cannot replicate. Biggest moat play."

**What it measures:**
- FISP message delivery and ordering guarantees (SessionSend → SessionMessages → SessionRespond)
- Meeting protocol correctness (CreateMeeting → participant responses → MeetingSynthesize → MeetingDecide)
- Team orchestration (RunTeam topology, agent spawning, budget enforcement)
- Cross-session context sharing (agent A's decision visible in agent B's CompileContext)
- A2A permission enforcement (GrantPermission / RevokePermission)
- Budget enforcement (RecordAgentCost → BudgetStatus → exceeded flag)

**Harness architecture:** Subprocess-based (like Forge-Persist). Multi-agent coordination requires real process-level session management.

**Dataset shape:**
- Multi-agent scenario simulator: spawn N agent sessions, script their FISP interactions
- Meeting protocol traces with expected outcomes
- Edge cases: disconnected agents, high-frequency writes, permission violations, budget overruns

**Scoring:** correctness rate on scripted interactions + delivery latency percentiles + meeting protocol completion rate.

**Narrative gate:** results doc frames this as "the bench competitors literally cannot run" — honestly, just the facts, no hype.

### 2A-6 — Forge-Transfer

**Product claim:** "Enterprise-grade domain isolation — your agent's brain stays in your network."

**The thesis:** "Domain isolation is an enterprise/security concern that also catches cross-tenant bugs. Scoped configuration cascade, reality detection, and organizational boundaries must work correctly."

**What it measures:**
- Project/session isolation (memories from project A don't leak to project B)
- Scoped config cascade correctness (org → team → project → user, with locked keys and ceilings)
- Reality detection accuracy (auto-identifies Rust vs Node vs Python codebase)
- Cross-tenant leakage vectors (same user, different projects/orgs)
- Multi-reality context compilation (CompileContext for project A returns project A's context, not B's)
- HUD config inheritance (org-level defaults cascade to teams)

**Dataset shape:**
- Multi-project, multi-org corpus with ground-truth isolation boundaries
- Deliberate probing queries designed to flush out cross-project leaks
- Scoped config with locked overrides and ceiling values
- Multiple realities with different domain fingerprints

**Scoring:** isolation rate (no false positives) + cascade correctness + reality detection accuracy.

**Security-review gate:** the results of this bench are a prerequisite for any enterprise pitch. Phase 2C-5 (security audit) must see these results.

### Phase 2A KPI validation tests (CI-integrated, lighter weight)

In addition to the six Forge-* benchmarks, these KPI tests validate specific product claims from the pitch. They run as part of CI, not as standalone calibration runs.

| KPI Claim | Test | Threshold |
|-----------|------|-----------|
| "Bootstrap 100 memories in 60 seconds" | Timed ingestion of 100 Remember ops | < 60s wall time |
| "Memory recall latency: <50ms" | P95 hybrid_recall latency on 500-memory corpus | < 50ms |
| "Tool auto-discovery: 95% accuracy" | detect_and_store_tools vs known PATH ground truth | ≥ 95% |
| "KV-cache-aware layout saves 40-60% tokens" | static_prefix reuse rate across 10 CompileContext calls | ≥ 40% chars stable |
| "Consolidation cycle: <30s" | ForceConsolidate on 500-memory corpus | < 30s wall time |

### Phase 2A exit gate

All 6 custom benches published with honest results docs. KPI validation tests passing in CI. Framework (dataset generators, scoring helpers, forge-bench subcommand pattern) extracted into reusable modules. MEMORY.md updated with pointers to each result. Every bench passes the full 7-gate quality checklist.

---

## Phase 2B — Floor: standard benchmark completion

This phase is NOT committed in advance. Each wave is a decision made after seeing the prior wave's numbers. Wave 3 runs first because it closes the known Wave 1 LongMemEval regression AND aligns with the Forge-Identity story — once Forge-Identity (2A-3) has demonstrated that Forge CAN handle preferences on its own axis, Wave 3 lifts the LongMemEval number to match that demonstrated capability.

### 2B-1 — Wave 3 (preference sidecars + query features)

**What it lands:** MemPalace hybrid v3's 16 preference regex patterns as sidecar documents synthesized at ingest time. Plus the 50-word stopword list, kw-overlap fusion at weight 0.30 (LME) / 0.50 (LoCoMo), quoted-phrase detector, temporal date anchors, person-name boost.

**Expected impact:** LongMemEval single-session-preference 0.8000 → 0.93+. Overall LME R@5 0.9640 → 0.978.

**Deliverables:** code + tests + adversarial review + bench run + results doc update on both LME and LoCoMo. Plus: updated comparison tables in the Forge-Identity results doc showing the LME parity story.

**Gate to 2B-2:** bench numbers published. If LME lands above 0.97 and the preference category above 0.92, 2B-1 is complete.

### 2B-2 — Wave 2 (bge-large embedder) — CONDITIONAL

**Runs only if:** after Wave 3, LongMemEval is below 0.97 AND there is a reason to believe bge-large would close the remaining gap.

**Why it might NOT run:** LoCoMo target (0.918) was projected for Wave 2 but we already scored 0.9456 on MiniLM hybrid. The LoCoMo side is moot. LME side is a +0.7 pp projection — small and possibly unnecessary after Wave 3.

**Decision log entry:** after Wave 3, re-evaluate. If skipping, document the reasoning in the results hub.

### 2B-3 — Wave 4 (LLM rerank opt-in tier) — CONDITIONAL

**Runs if:** the founder wants the "matches the 0.984 MemPalace clean ceiling" narrative for the investor deck AND the rerank tier can be shipped with honest opt-in semantics (default off, no silent API dependency).

**Why it might NOT run:** the "memory works without a cloud call at query time" positioning is stronger than the rerank number. If the founder prefers positioning over raw numbers, skip.

### 2B-4 — ConvoMem + MemBench harnesses

**What it lands:** two new bench subcommands for ConvoMem (Salesforce, 75k+ QAs) and MemBench (ACL 2025, 8.5k+ items, 11 categories). Raw KNN + Hybrid rows on each.

**Why:** these are table-stakes standard benches that competitors publish on. Publishing them is the minimum bar for open-source evaluator credibility.

**Scope fence:** raw mode only, both strategies. No four-mode comparison. No judge scoring. Just retrieval metrics.

### Phase 2B exit gate

LongMemEval ≥ 0.97 R@5 published. LoCoMo ≥ 0.94 R@10 published. At least one of ConvoMem/MemBench published. Results hub aggregates all benches with honest per-bench limitations.

---

## Phase 2C — Working product vetting

This phase makes Forge a "working product thoroughly vetted in every dimension." Runs partially in parallel with Phase 2A/2B where possible (see §Sequencing).

### 2C-1 — Observability layer (plan.md §11-13)

**What it lands:**
- `workers/kpi_collector.rs` non-blocking event emitter
- `kpi/emit.rs` typed API for worker instrumentation
- Prometheus metrics: `forge_bench_latest{benchmark,mode,metric}`, `forge_uat_last_passed{story}`, latency percentiles, worker backlogs
- New CLI groups: `forge-next kpi show|history`, `bench latest|history`, `telemetry enable|disable`
- KPI tables already exist in schema.rs (`kpi_events`, `kpi_snapshots`, `kpi_benchmarks`, `uat_stories`) — just need the collector wiring
- Brain Health panel in canvas app

**Why it runs in parallel with Phase 2A:** the KPI schema is already there. Each custom bench in 2A can emit KPI events from day one, populating the tables as they run.

### 2C-2 — UAT framework

**What it lands:**
- 7 user stories (one per benchmark: LME, LoCoMo, Persist, Tool, Identity, Multi, Transfer)
- `forge-next uat run` / `uat status` CLI
- Gate: every release must pass all 7
- Users verify on their own machine (local-first by design)

### 2C-3 — Dogfood-driven bug fixing

**Known bugs to close before the "working product" milestone (from memory):**
- Session 14 dogfood issues (6 bugs + 4 improvements)
- Stale PID lock file prevents daemon restart after crash (Forge gap #10)
- Session cleanup TTL / auto-prune (Forge gap #4)
- Tmux session discovery should filter for `forge-` prefix
- Web pivot adversarial review CRITICALs (3) + HIGHs (4)
- Integration gaps (13 gaps, 26 files, 25/25 endpoints) — verify still closed
- A2A messaging API name mismatch between `daemon.ts` and `bridge.ts`
- Plugin version bug — `forge:*` skills broken by stale version in `installed_plugins.json`

**Meta-bug:** whatever dogfooding surfaces during Phase 2A. Each custom bench is an implicit dogfood run of the extraction pipeline.

### 2C-4 — CI bench regression gate

**What it lands:** `.github/workflows/bench.yml` that runs a 50-Q LME subset + all custom Forge-* benches on every PR. Regression > 2 pt R@5 fails build.

**Why:** prevents future regressions in the published numbers. Without this, every dependency bump is a bench-quality risk.

### 2C-5 — Security audit

**Scope:**
- Input validation surfaces (especially after Wave 1 FTS5 finding)
- Authentication paths (daemon HTTP API)
- SQL injection vectors (all user-input paths)
- Cross-tenant isolation (backstops Forge-Transfer at the code level)
- Secret handling (API keys, JWT secrets, Litestream creds)
- Prompt injection surfaces (extraction prompt, recall prompts)

**Deliverables:** audit report in `docs/security/audit-2026-XX.md` + any fixes landed in separate commits.

### 2C-6 — Ops polish

**Scope:**
- Install script tested on clean macOS + clean Ubuntu (VMs)
- Upgrade path: v0.4 → v0.5 migration tested
- Disaster recovery runbook: "daemon is wedged, database is corrupted, how do I recover"
- Litestream backup/restore verified end-to-end
- Systemd and launchd unit files tested

### Phase 2C exit gate

All 7 UAT stories passing. CI bench gate green for 2 consecutive weeks (the only time-gate in this plan, because regression surfaces are inherently time-gated). Security audit findings all closed or explicitly accepted. Ops runbook tested end-to-end on a fresh machine.

---

## Phase 2D — Launch readiness

This phase is the last mile before public launch. Everything that makes Forge presentable to all three audiences.

### 2D-1 — Results hub consolidation

Single index page at `docs/benchmarks/index.md` linking every benchmark result with a headline row per bench. Formatted so an investor can look at it and understand the moat in 60 seconds.

### 2D-2 — Canvas app final polish

- Landing page deploy (currently pending auth per memory)
- Onboarding flow walkthrough (first 5 minutes of the app)
- Pricing page aligned with Dodo Payments MoR plan
- Polish pass on the v5 Miro-style canvas redesign

### 2D-3 — Documentation pass

- Getting started tested on a fresh machine (not the founder's dev machine)
- API reference complete for every endpoint
- CLI reference complete for every subcommand
- Security + operations docs current
- Benchmark plan + all results docs cross-linked

### 2D-4 — Investor deck materials

Pull numbers from the results hub. Update `finances-v2`, growth deck, `roadmap-v2`. The moat story now has real data behind it (Phase 2A). The floor story now has competitive data (Phase 2B).

### 2D-5 — Go-live rehearsal

End-to-end dry run of the public launch. Support materials ready. Discord/forum set up. Bug tracker ready for public issues. Press/HN post drafted.

### Phase 2D exit gate

The founder can onboard a new user from "heard about Forge" to "first real memory retrieved" without manual hand-holding, on a machine they've never touched before.

---

## Sequencing

Because there's no time gating, the sequencing is about dependencies and interweave opportunities, not wall clock.

**Strict dependencies:**
- 2A-1 (Forge-Persist) must land before any other 2A bench (framework shakedown)
- 2B-1 (Wave 3) must land before 2B-2 (Wave 2 decision)
- 2D (launch prep) must run last

**Parallel opportunities:**
- 2C-1 (KPI collector) starts in parallel with 2A-1 — they share the schema
- 2C-3 (bug fixing) runs continuously in the background — every dogfood cycle surfaces bugs
- 2C-5 (security audit) starts any time after Forge-Transfer (2A-5) lands — code surfaces are known by then
- 2B-4 (ConvoMem + MemBench) can run in parallel with late 2A benches
- Documentation (2C + 2D-3) runs incrementally with every deliverable

**Recommended overall shape:**

```
Phase 2A ──────────────────────────────────────────┐
                                                   │
  Forge-Persist ─→ Forge-Context ─→ Forge-Consolidation ─→ Forge-Identity ─→ Forge-Multi ─→ Forge-Transfer
  (substrate) ✅   (proactive)     (self-healing)         (personality)      (big moat)     (enterprise)
       │                                                                                          │
       ├─── 2C-1 observability (parallel from here, emits KPI natively)
       │                                                                                          │
       ├─── 2C-3 bug fixing (continuous from here)                                                │
       │                                                                                          │
       │                                        ├─── Wave 3 (after Forge-Identity)                │
       │                                        │                                                 │
       │                                        │                                                 ├─── 2C-5 security audit
       │
       ├─── KPI validation tests (CI, parallel from Forge-Context onward)
       │
Phase 2B ──────────────────────────┐
  Wave 3 ──→ (Wave 2?) ──→ (Wave 4?) ──→ ConvoMem/MemBench
  
Phase 2C tail ─────────────────────┐
  C2 UAT ──→ C4 CI gate ──→ C6 ops polish
  
Phase 2D ──────────────────────────┐
  D1 results hub ──→ D2 canvas polish ──→ D3 docs ──→ D4 investor ──→ D5 rehearsal
```

---

## Decision log (Wave 1 findings → Phase 2 priorities)

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-14 | Phase 2 runs moat-first (2A before 2B) | Founder precedence: unique value > benchmark numbers |
| 2026-04-14 | Phase 2A order: Persist → Tool → Identity → Multi → Transfer | Framework risk → narrative weight → moat strength |
| 2026-04-16 | Phase 2A expanded to 6 benchmarks, renamed Forge-Tool → Forge-Context, added Forge-Consolidation | Deep product-vision analysis revealed "self-healing memory" is #1 differentiator in every pitch; "proactive intelligence" is broader than skill search. Each bench now maps to a product claim. |
| 2026-04-16 | Phase 2A order updated: Persist → Context → Consolidation → Identity → Multi → Transfer | Context (proactive intelligence) is highest product-narrative weight; Consolidation proves #1 differentiator; Identity builds on Consolidation's quality guarantees |
| 2026-04-16 | Forge-Context harness is in-process (not subprocess) | Testing retrieval quality, not crash recovery — 10× faster calibration, no incremental signal from subprocess overhead |
| 2026-04-16 | KPI validation tests added as CI-integrated lightweight benchmarks | Validate specific pitch claims (bootstrap speed, recall latency, tool discovery accuracy, KV-cache savings, consolidation speed) |
| 2026-04-14 | No time estimates anywhere in the plan | Founder directive: plan for quality and complete delivery, not speed |
| 2026-04-14 | Wave 2 (bge-large) is now conditional | LoCoMo target already overshot on MiniLM hybrid — re-evaluate after Wave 3 |
| 2026-04-14 | Wave 3 is the next standard-bench commitment after Phase 2A | Closes the Wave 1 regression + aligns with Forge-Identity story |
| 2026-04-14 | Phase 2C observability runs in parallel with 2A | KPI schema already there; each bench can emit natively |
| 2026-04-14 | TDD + adversarial review pattern mandatory for every Phase 2 deliverable | Wave 1 caught 3 real bugs via this pattern; proven method |
| 2026-04-14 | Interweave work across phases where safe | Founder directive: maximize breadth of progress |

## Open decision points

These need founder input at the right gate but don't block Phase 2A start:

- **D1:** Forge-Tool dataset scope — user's actual Claude Code workflow vs synthetic representative set. Decide at Forge-Tool design gate.
- **D2:** Forge-Identity preference categories — reflect the user's actual preferences (dogfood) or a synthetic persona. Decide at Forge-Identity design gate.
- **D3:** Forge-Multi scenario scale — how many agents, how many sessions each. Decide at Forge-Multi design gate.
- **D4:** Wave 2 bge-large — run or skip. Decide after Wave 3 numbers land.
- **D5:** Wave 4 LLM rerank — run or skip. Decide after Wave 3 + ConvoMem numbers land.
- **D6:** Phase 2D investor deck contents — which numbers lead. Decide after Phase 2A + Wave 3 land.

## Non-goals (explicit exclusions)

Things NOT to do in Phase 2 without the founder asking:
- Full 500-Q four-mode LongMemEval (extract/consolidate/hybrid) — the 50-Q subset already answered the question
- Judge-based end-to-end QA scoring (F1, LLM-judge accuracy) — retrieval metrics only
- Building a reader stage for any benchmark
- Wing v3 closets, Stella V5 1.5B, Zep graph, Supermemory ASMR, MemPalace Diary mode — all dominated per Wave 1 research
- Chasing MemPalace's contaminated 100% number — realistic ceiling is 0.978-0.982
- ConvoMem / MemBench four-mode comparisons — only raw-mode rows in Phase 2B-4

---

## How to resume Phase 2 in a new session

Any future session should:

1. Read this plan (`docs/benchmarks/phase-2-plan.md`)
2. Read the matching handoff memory (`~/.claude/projects/-Users-dsskonuru-workspace-playground-forge/memory/project_phase_2_handoff_2026_04_14.md`)
3. Read the Wave 1 results updates in `docs/benchmarks/results/longmemeval-2026-04-13.md` and `locomo-2026-04-13.md`
4. Pick up at the currently-active phase item based on the decision log's last entry
5. Start with a **design gate** for the current item BEFORE any code — founder approves the shape first
6. Only then begin TDD cycles per §"Quality gate definition"

No Phase 2 code lands without going through the full 7-gate sequence. No gate is optional.
