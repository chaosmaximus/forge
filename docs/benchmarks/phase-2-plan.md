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

## Phase 2A — Moat: five custom Forge-* benchmarks

Each of these five benchmarks is a complete, standalone deliverable that ships independently (results published, commit landed, memory indexed) before the next begins. Order selected for: framework risk first (validate the custom-bench framework), narrative weight in the middle, biggest moat play toward the end once the framework is battle-tested.

### 2A-1 — Forge-Persist

**The thesis:** "Local-first means nothing if the daemon can't survive a restart and replay correctly." Simplest bench — useful for proving the custom-bench framework works before tackling harder ones.

**What it measures:**
- State persistence across `SIGKILL → restart` cycles
- WAL replay correctness after abrupt shutdown
- Memory-table + raw-layer integrity
- Session recovery
- Worker resumption

**Dataset shape:**
- Scripted workload: N memory inserts, K raw-chunk ingests, J FISP sessions
- Random interleaving (seeded for reproducibility)
- Mid-run SIGKILL at a configurable offset
- Expected post-restart state documented per seed

**Scoring rubric:**
- **Recovery rate** (%): fraction of pre-kill state visible post-restart
- **Consistency rate** (%): fraction of recovered state that matches pre-kill exactly (no corruption, no duplicates)
- **Recovery time** (seconds): wall time from `forge-next start` to first successful `forge-next health`
- **Pass threshold:** recovery rate ≥ 99%, consistency rate 100%, recovery time < 5 s

**Deliverables:**
- `crates/daemon/src/bench/forge_persist.rs` harness
- `crates/daemon/src/bin/forge-bench.rs` — new subcommand `forge-persist`
- Dataset generator at `crates/daemon/src/bench/datasets/persist.rs` with seed-based reproducibility
- `docs/benchmarks/results/forge-persist-<date>.md` honest results doc
- Integration test at `crates/daemon/tests/forge_persist_harness.rs`
- MEMORY.md index entry

**Framework-shakedown flag:** if the Forge-Persist harness is painful to build, the framework design needs rework before 2A-2 begins.

### 2A-2 — Forge-Tool

**The thesis:** "Agents should remember the procedures they've learned." Tool/skill recall is the most concrete non-retrieval axis where Forge's extraction pipeline earns its keep — the skill layer (Manas procedural memory) is a first-class Forge concept competitors don't have.

**What it measures:**
- Accuracy of tool-procedure recall across N+1 sessions given N prior sessions of usage
- Skill generalization (same tool used in a new context)
- Skill precedence (when multiple similar tools exist, does the right one surface?)
- Skill staleness (tools not used for a long time decay correctly)

**Dataset shape:**
- Synthetic session logs: each session uses a mix of tools (shell commands, API calls, editor operations)
- 5-10 distinct tools per corpus
- Question bank: "what command did I use to X?" / "what's my preferred way to Y?" / "show me the procedure for Z"
- Hand-curated ground truth for each question

**Scoring:**
- Recall@1 (did the right tool show up first)
- Recall@5 (did the right tool show up in top 5)
- Precision of the returned procedure (does the *instance* returned match the query context?)

**Dogfood extra:** the founder uses Forge-Tool on their own Claude Code session logs for ≥ 1 calendar day before merge.

### 2A-3 — Forge-Identity

**The thesis:** "Memory systems that treat preferences as static lose information the moment the user changes their mind." Identity and preference-tracking is where Forge's consolidation phases (contradiction detection, reconsolidation, valence flipping) are supposed to pay off.

**This is also the bench that flips the Wave 1 single-session-preference regression story into a win on our own axis.** MemPalace's LongMemEval weakness is paraphrased preference questions; Forge-Identity is the bench where Forge should dominate.

**What it measures:**
- Time-ordered preference tracking (most-recent wins when the user changes their mind)
- Contradiction detection (opposing preferences across sessions)
- Valence flipping (like → dislike transitions handled)
- Implicit preference inference (user behavior → inferred preference)
- Preference staleness (preferences from 6 months ago weaker than yesterday)

**Dataset shape:**
- Time-stamped conversation logs with explicit preference statements at varying points
- Preference categories: food, tools, workflow, communication style, aesthetic
- Contradictions injected at known timestamps
- Paraphrased preferences (same meaning, different words)
- Question bank: "what's my current preference for X?" / "has my preference for Y changed?" / "what did I prefer last month?"

**Scoring:** preference accuracy + temporal correctness + contradiction-resolution accuracy.

**Narrative gate:** results doc must include a comparison row against LongMemEval single-session-preference showing the story arc.

### 2A-4 — Forge-Multi

**The thesis:** "Agents sharing memories via FISP is a Forge concept competitors literally cannot run." Biggest moat play. Positioned fourth because by this point the framework is battle-tested and we can focus on FISP coordination semantics rather than framework risk.

**What it measures:**
- Cross-agent memory sharing via FISP
- Memory propagation latency (agent A writes → agent B sees)
- Access control (agent A's private memories don't leak to B unless shared)
- Conflict resolution (two agents write same key)
- Subscription semantics (agent B subscribed to agent A's domain gets updates)

**Dataset shape:**
- Multi-agent scenario simulator: spawn N agent sessions, script their memory interactions
- FISP message traces with expected outcomes
- Edge cases: disconnected agents, high-frequency writes, schema mismatches

**Scoring:** correctness rate on scripted interactions + latency percentiles.

**Narrative gate:** results doc frames this as "the bench competitors literally cannot run" — honestly, just the facts, no hype.

### 2A-5 — Forge-Transfer

**The thesis:** "Domain isolation is an enterprise/security concern that also catches cross-tenant bugs." Last in Phase 2A because smaller test surface but most important for enterprise credibility.

**What it measures:**
- Project/session isolation (memories from project A don't leak to project B)
- Cross-tenant leakage vectors (same user, different projects)
- Explicit transfer (when user wants to move memories across projects, does it work correctly?)
- Audit trail integrity

**Dataset shape:**
- Multi-project corpora with ground-truth "this memory belongs to project X, should NOT appear in project Y query"
- Deliberate probing queries designed to flush out leaks
- Cross-tenant attacks (session hijacking simulation, org-scope bypass attempts)

**Scoring:** isolation rate (no false positives) + transfer correctness.

**Security-review gate:** the results of this bench are a prerequisite for any enterprise pitch. Phase 2C-5 (security audit) must see these results.

### Phase 2A exit gate

All 5 custom benches published with honest results docs. Framework (dataset generators, scoring helpers, forge-bench subcommand pattern) extracted into reusable modules. MEMORY.md updated with pointers to each result. Every bench passes the full 7-gate quality checklist.

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
Phase 2A ──────────────────────────┐
                                   │
  Forge-Persist ─→ Forge-Tool ─→ Forge-Identity ─→ Forge-Multi ─→ Forge-Transfer
  (framework)     (narrative)    (moat pivot)      (big moat)     (enterprise)
       │                                                                │
       ├─── 2C-1 observability (parallel from here, emits KPI natively)
       │                                                                │
       ├─── 2C-3 bug fixing (continuous from here)                      │
       │                                                                │
       │                          ├─── Wave 3 (after Forge-Identity)    │
       │                          │                                     │
       │                          │                                     ├─── 2C-5 security audit
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
