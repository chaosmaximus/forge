# Forge-Isolation Bench (2A-5) — Stage 1 Results — 2026-04-25

**Status:** Calibration locked. All 5 seeds PASS at composite = 1.0000.
**Spec:** `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md` v2.1 LOCKED.
**Implementation HEAD at run time:** `44184fc`.
**Bench binary:** `target/release/forge-bench` (release profile, `bench` feature).
**Hardware profile:** linux x86_64, GCP `chaosmaximus-instance`.

---

## 1. Calibration sweep — 5/5 seeds PASS

| seed | composite | pass | infra checks | wall_duration_ms |
|------|-----------|------|--------------|-------------------|
|     7 | 1.0000 | ✓ | 8/8 | <1000 |
|    13 | 1.0000 | ✓ | 8/8 | <1000 |
|    42 | 1.0000 | ✓ | 8/8 | <1000 |
|   100 | 1.0000 | ✓ | 8/8 | <1000 |
|  1234 | 1.0000 | ✓ | 8/8 | <1000 |
| 99999 | 1.0000 | ✓ | 8/8 | <1000 |

**6/6 seeds (5 calibration + 1 dogfood) converged on first run** with no
iteration needed. Plan-doc allowed up to 5 calibration cycles per locked
decision; actual cycles required: 0.

**Why determinism is so tight:** corpus content is fully formula-derived
(no `random_range` consumption from rng — see spec §3.2 + v1 review M4
fix); embeddings come from `bench::common::deterministic_embedding(seed_key)`
which seeds its own RNG from the input string. Result: identical corpus
shape across every `--seed N` invocation; composite is invariant.

This is a **feature, not a bug**: an isolation bench's composite should
not vary with seed because the property under audit (zero leakage)
either holds structurally or doesn't. The seed parameter is preserved
for signature consistency with other bench harnesses + future extension.

## 2. Per-dimension breakdown (every seed, every dim)

```
[forge-isolation] === results ===
[forge-isolation] composite=1.0000
[forge-isolation] pass=true
[forge-isolation] cross_project_precision    = 1.0000 (min 0.95, pass=true)
[forge-isolation] self_recall_completeness   = 1.0000 (min 0.85, pass=true)
[forge-isolation] global_memory_visibility   = 1.0000 (min 0.90, pass=true)
[forge-isolation] unscoped_query_breadth     = 1.0000 (min 0.85, pass=true)
[forge-isolation] edge_case_resilience       = 1.0000 (min 0.85, pass=true)
[forge-isolation] compile_context_isolation  = 1.0000 (min 0.95, pass=true)
[forge-isolation] infrastructure_checks=8/8 passed
[forge-isolation] PASS
```

### Dimension scores explained

* **D1 cross_project_precision (1.0000)** — for each main project P, recall
  the shared `"isolation_bench"` tag scoped to P. Foreign-token denominator
  excludes globals (recallable from every project) and includes the
  alphabet-sentinel project (prefix-collision audit). Production project
  scoping prevents 100% of foreign tokens.
* **D2 self_recall_completeness (1.0000)** — for each project P, recall
  query=`"{P}_secret"` project=Some(P), top-10. Each project's 30 memories
  carry `_secret` in title + content, so own_hits_in_top_k = 10/10 on
  every project.
* **D3 global_memory_visibility (1.0000)** — for each project P, recall
  query=`"global_pattern"` project=Some(P). Production SQL filter
  `m.project = ?2 OR m.project IS NULL OR m.project = ''` returns globals
  to every project's recall — verified all 10 globals appear in every
  main project's bucket.
* **D4 unscoped_query_breadth (1.0000)** — recall query=`"isolation_bench"`
  project=None, limit=200. Returns memories from all 6 buckets (5 main
  projects + global pool). bucket_coverage = 6/6.
* **D5 edge_case_resilience (1.0000)** — all 7 sub-probes pass:
  * `empty_string_targets_global` — `Some("")` returns no project-scoped
    memories ✓
  * `special_chars_no_panic` — `Some("p@#$%")` Ok ✓
  * `overlong_project_no_panic` — 256-char project Ok ✓
  * `sql_injection_inert` — `Some("alpha'; DROP TABLE memory;--")` did
    not drop or mutate (sentinel-row hash unchanged pre/post) ✓
  * `prefix_collision_isolated` — `Some("alpha")` excludes alphabet
    memories ✓
  * `case_sensitivity_strict` — `Some("ALPHA")` excludes alpha corpus
    (BINARY collation) ✓
  * `trailing_whitespace_strict` — `Some(" alpha")` excludes alpha corpus ✓
* **D6 compile_context_isolation (1.0000)** — for each main project P,
  call `compile_dynamic_suffix_with_inj` with pinned
  `ContextInjectionConfig { session_context: true, .. }` (per N3 fix).
  Scan rendered XML for foreign-project secret tokens; foreign_tokens=0
  in every project's XML. max_possible=15 (decisions_limit + lessons_limit
  per N1 fix); a 1-row regression would score 0.933 < 0.95 min and CATCH it.

## 3. Infrastructure checks — 8/8 pass

| # | name | detail |
|---|------|--------|
| 1 | memory_project_index_exists | idx_memory_project present in sqlite_master |
| 2 | memory_project_column_exists | memory.project column accessible |
| 3 | recall_accepts_project_filter | recall_bm25_project returned Ok with Some(project) |
| 4 | seeded_rng_deterministic | seeded_rng(42) produces same u64 twice |
| 5 | corpus_size_matches_spec | corpus has 165 rows (expected 165) |
| 6 | project_distribution_correct | 5×30 + 5 + 10 = 165 confirmed |
| 7 | embedding_dim_matches_consolidation | first memory embedding.len() = 768, expected 768 |
| 8 | compile_context_returns_xml | compile_dynamic_suffix_with_inj returned 1500-3000 chars |

## 4. Reproduction

```bash
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"

cargo build --release --features bench --bin forge-bench

for seed in 7 13 42 100 1234 99999; do
    ./target/release/forge-bench forge-isolation \
        --seed $seed \
        --output bench_results_isolation_$seed \
        --expected-composite 1.0
done
```

`--expected-composite 1.0` enforces a ±0.05 drift gate; any future
regression that drops composite below 0.95 will fail the CLI exit code.

## 5. What this bench catches

A regression that breaks project scoping — for example:
- A typo in the WHERE clause: `m.project = ?2 OR m.project IS NULL OR
  m.project IS NULL` (duplicate IS NULL drops the empty-string match) →
  D3 globals stop appearing → composite drops below 0.95 → CI fails.
- A missing project filter on a new recall helper added in P3-3+ → D1
  returns foreign-project rows → D1 < 0.95 → CI fails.
- A future ContextInjection default flip turning session_context off
  globally → D6 still works (pinned config — N3 fix protects against
  this).
- An UPDATE-class SQL injection that preserves row count but corrupts
  scoping → D5 probe 4's sentinel-row hash check catches it (N4 fix).

## 6. Out-of-scope (deferred to v2 or future bench)

Per spec §5:

- Skills, declared, domain_dna, perception, entity layers — corpus only
  seeds the memory table.
- raw_documents-specific helpers — predicate is structurally identical
  to memory's, so by-construction covered, but `recall_raw_chunks_bm25`
  isn't exercised.
- Cross-organization isolation (organization_id filter) — all bench
  memories share `organization_id = None` (default org).
- Tag-substring leakage — corpus avoids putting foreign project names
  in tags as a sidestep; v2 bench should add a Dim 7 tag-sanitization
  probe.
- Concurrent recall stress — single-thread sequential.

## 7. CI integration

Stage 1 T12 adds `forge-isolation` to the `bench-fast` matrix in
`.github/workflows/ci.yml` with `continue-on-error: true` per the same
14-green-runs gate-promotion policy as forge-identity (T17, deferred).

## 8. References

- Spec: `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md`
- v1 review (verdict: not-lockable): `docs/superpowers/reviews/2026-04-25-p3-3-2a-5-spec-domain-isolation.yaml`
- v2 review (verdict: lockable-with-fixes): `docs/superpowers/reviews/2026-04-25-p3-3-2a-5-spec-domain-isolation-v2.yaml`
- Implementation: `crates/daemon/src/bench/forge_isolation.rs`
- CLI subcommand: `crates/daemon/src/bin/forge-bench.rs::run_forge_isolation`
- Events-namespace registry: `docs/architecture/events-namespace.md` `forge-isolation` row
