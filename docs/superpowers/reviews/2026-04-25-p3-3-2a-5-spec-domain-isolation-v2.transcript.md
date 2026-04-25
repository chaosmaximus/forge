# Adversarial review v2 — P3-3 2A-5 spec (domain-transfer isolation bench)

**Target:** `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md` v2
**Reviewer:** Claude Opus 4.7 (general-purpose), 2026-04-25
**Scope:** spec-only second pass — verify v1 finding closure + surface NEW issues
**Commit range:** base `aa14763` (v1 spec) → head `c1389bd` (v2 spec)

---

## v1 finding closure verification

### B1 — `deterministic_embedding` fixture (RESOLVED)

v2 §2 fact 13 + T2.1 lift the function. Verified at code:
- `forge_consolidation.rs:1684` `const EMBEDDING_DIM: usize = 768;`
- `forge_consolidation.rs:1687` `pub fn generate_base_embedding(seed_key: &str) -> Vec<f32>`
- Call sites (L1752, 1763, 1773, 3558, 3566, 3574, 3582, 3583, 3609, 3618, 3631) all use the bare name — re-export from forge_consolidation preserves them under T2.1's plan.

CLOSED.

### B2 — `composite_score` lift (RESOLVED)

v2 §2 fact 12 + T2.2 lift the function. Verified at code:
- `forge_identity.rs:82` `const DIM_WEIGHTS: [f64; 6] = [0.15, 0.15, 0.15, 0.15, 0.15, 0.25];` — sum = 1.00. NON-uniform.
- `forge_identity.rs:1632` `fn composite_score(dimensions: &[DimensionScore; 6]) -> f64` — currently private, hardcoded 6-tuple, indexes into DIM_WEIGHTS.
- Tests at L1848 (`composite_score(&zeroed) == 0.0`) and L1857 (`composite_score(&ones) == sum(DIM_WEIGHTS)`) will catch any byte-non-identical regression.

T2.2's proposed N-dim signature `composite_score(dims: &[DimensionScore], weights: &[f64]) -> f64` with `debug_assert weights.len() == dims.len() && (sum(weights) − 1.0).abs() < 1e-9` is sound. Forge-identity's call site updates trivially: pass `&DIM_WEIGHTS[..]`. Byte-identical composite preserved.

CLOSED.

### B3 — D1 query fix (RESOLVED)

v2 changed D1 query from `""` to `"isolation_bench"`. Verified at code:
- `db/ops.rs:506-528` `sanitize_fts5_query` filters non-alphanumeric; `"isolation_bench"` (alphanumeric + `_`) survives intact.
- `db/ops.rs:569-571` empty-after-sanitize → `Ok(Vec::new())` short-circuit. `"isolation_bench"` does NOT trigger short-circuit.
- §3.3 foreign-token denominator math (excludes globals; includes alphabet sentinel for D5 probe 5) is correct: globals legitimately appear from any project so excluding them avoids false-positive leakage; alphabet sentinel IS a foreign-token source for `alpha` queries.

CLOSED.

### H1 — compile_context coverage via D6 (PARTIALLY RESOLVED — see N1, N2, N3)

v2 added D6 driving `compile_context`. Verified at code:
- `recall.rs:2047` `compile_context(conn, agent, project)` calls `compile_static_prefix` + `compile_dynamic_suffix`.
- `recall.rs:1003-1008` decisions SQL: `WHERE memory_type = 'decision' AND status = 'active' AND (project = ?1 OR project IS NULL OR project = '')`.
- `recall.rs:1100` lessons SQL: same WHERE shape.

So D6 DOES exercise the memory-layer of compile_context. But compile_dynamic_suffix also calls (none of which D6 seeds):
- `list_domain_dna(conn, Some(proj))` at L938
- `list_entities(conn, project, entities_limit)` at L1291
- `list_unconsumed_perceptions(conn, None, project)` at L1417

D6 covers ~30% of compile_context's project-scoped helpers. v2 §5 and §4 D10 disclaim this. Disclaimer is adequate but documentation is fragmented (see N2). And the current denominator math (N1) effectively neutralizes the D6 signal.

PARTIALLY RESOLVED.

### H2 — D5(a) empty-string semantics (RESOLVED)

v2 redefined D5(a) to `empty_string_targets_global`. Verified at code:
- `db/ops.rs:705` `m.project = ?2 OR m.project IS NULL OR m.project = ''`
- With `?2 = ""`, returns NULL OR empty project — i.e., the global pool.
- v2 probe 1 asserts "no `{x}_secret_` tokens from any non-global project in result" — semantics now match implementation.

CLOSED.

### H3 — D5 expansion (RESOLVED)

v2 ships 7 probes. Each is meaningful and non-redundant.

**Probe 6 (case sensitivity):** `grep -in COLLATE schema.rs` returns NOTHING. Memory.project at L324 `project TEXT,` has NO collation override → SQLite default BINARY for `=` operator → `'ALPHA' != 'alpha'`. Probe 6 assumption is correct.

**Probe 4 (SQL injection):** bind params used at db/ops.rs:726, 729 (`params![safe_query, proj, limit, org]`) → injection structurally impossible. Row-count assertion is correct for what it tests but weak vs mutation injection (see N4).

CLOSED.

### M1, M2, M3, M4, L1, L2, L3 — RESOLVED

- **M1:** §3.7 single-shared-DaemonState mandate sound for an isolation bench.
- **M2:** §4 D9 + §5 disclaimer adequate.
- **M3:** §5 tag-leakage disclaimer + §8.2 v3 trigger adequate.
- **M4:** deterministic confidence formula — math walked through:
  - idx=0: 0.7 + clamp(0.0, 0.0, 0.29) = 0.70 ✓
  - idx=29: 0.7 + clamp(0.29, 0.0, 0.29) = 0.99 ✓
  - idx=30+: 0.7 + clamp(0.30, 0.0, 0.29) = 0.99 (clamp engages)
  - All in [0.70, 0.99]. CORRECT.
- **L1:** rationale added.
- **L2:** T1 grep added.
- **L3:** cosmetic; `grep -c "^fn check_" forge_identity.rs` = 14 strict count. v1 reviewer's 37 was loose grep including indented `fn check_` matches inside other functions. v2 wording reflects both.

ALL RESOLVED.

---

## NEW issues introduced or surfaced by v2

### N1 [HIGH] — D6 max_possible math is wrong

§3.3:
```
D6 score per project P:
   xml = compile_context(&conn, "isolation_bench_agent", Some(P))
   foreign_tokens = sum(occurrences(xml, "{Q}_secret_") for Q in projects if Q != P)
   max_possible = (N-1) × 30 (other-projects' memory count)
   score_P = 1 − (foreign_tokens / max_possible)
```

Two errors:

**(a) Rendered cap.** `compile_context` calls `compile_dynamic_suffix` which uses `decisions_limit = 10` + `lessons_limit = 5` (config.rs:500-501) per query. SQL is `LIMIT {decisions_limit}` (recall.rs:1008) and `LIMIT {lessons_limit}` (recall.rs:1101). Maximum rendered rows per project = **15**, not 30. `(N-1) × 30 = 120` is double the structural ceiling.

**(b) WHERE-clause filter.** SQL filter is `(project = ?1 OR project IS NULL OR project = '')` — foreign-project rows are filtered AT THE SQL LAYER. Under correct behavior, `foreign_tokens = 0` always; D6 trivially scores 1.0 on every clean run. The denominator only matters when a regression breaks the WHERE clause and leaks N foreign rows into the rendered XML.

If the worst observable leak is "compile_context renders all 15 decision-slot rows from a foreign project" = 15 foreign tokens, and `max_possible = 120`, then a 1-row leak scores `1 − 1/120 = 0.9917` (PASS), a 5-row leak `1 − 5/120 = 0.958` (PASS — still above 0.95 min), 14-row leak `0.883` (FAIL). The denominator hides leaks ≤ 5 rows.

Better denominator: `max_possible = decisions_limit + lessons_limit = 15` per project. Then 1-row leak → 0.933 (BELOW 0.95 min, CAUGHT). 0-row leak → 1.0 (PASS).

Or: override bench's ContextConfig to push decisions_limit + lessons_limit up to 30 each, then `max_possible = 60` per project.

Either fix unblocks D6's signal; current spec produces a denominator that hides single-digit leak regressions.

### N2 [MEDIUM] — D6 partial-coverage disclaimer fragmented

Disclaimer about D6 NOT covering skills/declared/domain_dna/perception/entity is in 3 places:
- §3.1 D6 row note "v1 covers only the memory-layer portion"
- §4 D10
- §5 third bullet

A future reviewer might miss that a leakage bug in `list_entities` (recall.rs:1291) or `list_unconsumed_perceptions` (recall.rs:1417) is invisible to v1. Recommend single consolidated coverage table in §5 enumerating each compile_context-invoked helper and v1 coverage status.

### N3 [MEDIUM] — D6 invocation under-specifies config dependency

`compile_context` at recall.rs:2048 calls `crate::config::load_config()` to get `ContextInjectionConfig`. In a `:memory:` bench with no FORGE_DIR, `load_config()` returns `Default::default()` — currently `inj.session_context = true`. If a future default flip turns it off, D6 emits empty `<decisions/>` + `<lessons/>` (the section_disabled branch at recall.rs:999-1000) → `foreign_tokens = 0` → D6 = 1.0 trivially.

Fix: bench should call `compile_dynamic_suffix_with_inj` (recall.rs:876) directly with a pinned `ContextInjectionConfig { session_context: true, .. }`. Or add infra assertion #9 verifying compile_context output contains non-empty `<decisions>` + `<lessons>` blocks before D6 runs.

### N4 [MEDIUM] — D5 probe 4 SQL-injection assertion is too weak

§3.1a probe 4: "Pass = `memory` table row count post-call equals pre-call count."

Catches `DROP TABLE` and `DELETE FROM`. Misses mutation-style injection like `UPDATE memory SET project = 'attacker'` (preserves row count, corrupts scoping).

Bind-param usage verified (db/ops.rs:726, 729) → injection is structurally impossible TODAY. But the bench is a regression detector for FUTURE code that might switch to string-interp. Add a sentinel-row hash check: hash `(title, content, project, tags)` of one canary row pre-call, assert equal post-call.

### N5 [LOW] — Composite weighting calibration documentation

Walked the calibration scenarios:
- D2=0.85 (exact min), others perfect: composite = 0.25·1 + 0.15·0.85 + 0.10·1 + 0.10·1 + 0.15·1 + 0.25·1 = **0.9775** (passes both gates ✓).
- D1=D6=0.95 (exact mins), others perfect: composite = 0.25·0.95 + 0.65·1 + 0.25·0.95 = **0.975** ✓.
- D1=D6=0.85 (BELOW min), others perfect: composite = 0.50·1 + 0.50·0.85 = **0.925** ← fails composite gate AND per-dim min (both catch).

Math sound; dual gate load-bearing for D1+D6. D2/D3/D4 at 0.15/0.10/0.10 means a D2 collapse to 0.0 still allows composite = 0.85 (others perfect), which is below 0.95 but only just. Not a flaw, but spec §4 D4 should ship a calibration table walking these cases for reviewer-aid.

### N6 [LOW] — Confidence formula `f32` vs schema `REAL`

`0.7 + (idx as f32 * 0.01).clamp(0.0, 0.29)` widens to f64 lossily — 0.70 in f32 is `0.69999998807907104...`. Determinism preserved per-run, but ORDER BY tie-breaking on close BM25 scores could shift across compiler versions. Use `0.7_f64 + (idx as f64 * 0.01).clamp(0.0, 0.29)` for cleanness.

---

## Verdict

`lockable-with-fixes`

- All 13 v1 findings genuinely closed (3 BLOCKER + 3 HIGH + 4 MED + 3 LOW).
- **NEW: 0 BLOCKER, 1 HIGH, 3 MEDIUM, 2 LOW.**
- N1 (HIGH) is addressable in a fix-wave commit — pin denominator to `decisions_limit + lessons_limit` (15) or override ContextConfig.
- N3 + N4 are cheap pre-implementation tightening fixes.
- No BLOCKER → spec can lock once N1 closed.

The v2 substantively addresses every v1 finding with verified code-grounded changes. The remaining gap is N1's denominator math, which produces a D6 score that rounds to PASS even with a 5-row foreign-leak. Fix is a one-line spec edit + matching impl; implementer can address inside T5 (D6 implementation).

CLOSED COUNT: 13 v1 findings (3B + 3H + 4M + 3L) all resolved.
NEW COUNT: 0 BLOCKER, 1 HIGH, 3 MEDIUM, 2 LOW.
</content>
</invoke>