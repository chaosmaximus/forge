# Adversarial review — 2A-5 forge-isolation IMPLEMENTATION (728cebb..db2e0fb)

**Verdict:** `lockable-with-fixes` — 0 BLOCKER, 2 HIGH, 5 MEDIUM, 4 LOW.
All HIGH findings are addressable in a single fix-wave commit (no
architectural rework). 13 spec/design properties verified resolved.

**Reviewer:** claude-general-purpose (Opus 4.7, 1M ctx)
**Date:** 2026-04-25
**Spec:** `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md` v2.1 LOCKED.

---

## 1. What I checked

Walked the diff `728cebb..db2e0fb` (8 implementation commits) against:

- Spec §3.1 D1-D6 dim definitions, score formulas, mins, weights
- Spec §3.1a D5 7-probe definitions, including N4 sentinel-row-hash fix
- Spec §3.3 composite + dual gate
- Spec §3.4 8 infrastructure assertions
- Spec §3.7 single-shared-DaemonState mandate
- Spec §3.2 corpus-generator 165-row layout, deterministic confidence
- Spec §6 lift prerequisites (T2.1 + T2.2)
- Spec §7 task breakdown T1..T13 traceability
- Test coverage in 17 module tests
- Harness-sync (`scripts/check-harness-sync.sh`)
- CI matrix entry shape + flag plumbing
- Events-namespace registry row
- Results doc (`2026-04-25-forge-isolation-stage1.md`) accuracy

Ran:
- `cargo test --features bench -p forge-daemon --lib bench::forge_isolation` → 17/17 pass
- `cargo test --features bench -p forge-daemon --lib bench::scoring` → 15/15 pass
- `cargo test --features bench -p forge-daemon --lib bench::common` → 8/8 pass
- `bash scripts/check-harness-sync.sh` → OK, no drift

---

## 2. Walked-through findings

### HIGH-1 — D6 over-counts foreign tokens (alphabet sentinel)

**Code:** `forge_isolation.rs:621-633`

```rust
for other in MAIN_PROJECTS { if other == project { continue; } ... }
let alphabet_needle = format!("{PREFIX_COLLISION_PROJECT}_secret_");
foreign_tokens += xml.matches(&alphabet_needle).count();
```

Spec §3.3 D6 formula reads:

```
foreign_tokens = sum(occurrences(xml, "{Q}_secret_") for Q in projects if Q != P)
```

Where `projects` in §3.2 explicitly carves alphabet out as a `D5-only construct`:

> Prefix-collision sentinel ("alphabet"): 5 memories ... Exists only to drive D5 probe 5

So the `alphabet_needle` line over-counts. Today it scores 1.0 only because
the SQL filter `(project = ?1 OR project IS NULL OR project = '')` blocks
alphabet rows from rendering. If a future regression admits alphabet (e.g.
an alias rule, or a JOIN that bypasses the project predicate), D6 would
penalize it — *correct in spirit*, *wrong by spec*.

Fix paths (pick one):
- (a) drop `foreign_tokens += xml.matches(&alphabet_needle).count();` and let
      D5 probe 5 handle the prefix-collision audit (spec-aligned)
- (b) update spec §3.3 to enumerate alphabet in the projects list, document
      this as part of the prefix-collision audit at the D6 layer

Severity HIGH because a contributor reading D6 as documented will be
surprised by the extra term and may revert it as a "bug fix", inadvertently
loosening the bench. Cosmetic at observed scores; structural under
regression.

---

### HIGH-2 — Infra check 8 weakens spec assertion

**Code:** `forge_isolation.rs:808`

```rust
let xml_ok = !xml.is_empty();
```

Spec §3.4 check 8:

> `compile_context_returns_xml` — `compile_context(&conn, "test_agent", Some("test_proj"))` returns non-empty `String` containing `<context>`

The string should *contain* `<context>` — the implementation only asserts
non-empty. Worse: `compile_dynamic_suffix_with_inj` returns content wrapped
in `<forge-dynamic>`, NOT `<context>`. So if the spec-literal check were
applied, it would FAIL on every clean run.

The implementation appears to have silently dropped the substring assertion
to make the check pass. This is the kind of `// FIXME: spec said X, code
says Y` divergence that compounds into "what does the bench actually check"
ambiguity later.

Fix: assert `xml.contains("<forge-dynamic>")` AND patch spec §3.4 to match
the actual root tag. Two-line fix; closes the gap.

---

### MED-1 — Determinism test misses the cross-seed property

**Code:** `forge_isolation.rs:1039-1051` (test `corpus_confidence_is_deterministic_and_in_range`)

The test pins `seed=42` twice and asserts identical output. The Stage 1
results doc claims "composite is invariant across seeds" (lines 26-30).
But there's no test asserting `generate_corpus(seed=7)` ≡
`generate_corpus(seed=42)` for the parts that should be seed-agnostic
(titles, projects, content templates).

Today the property holds because `_rng` is unused. A future contributor
may add `rng.random_range(...)` for, e.g., decision/lesson interleaving,
silently breaking the seed-invariance the bench advertises.

Fix: add a single test:

```rust
#[test]
fn corpus_titles_invariant_across_seeds() {
    let titles_42: Vec<_> = generate_corpus(&mut seeded_rng(42))
        .memories.iter().map(|m| m.title.clone()).collect();
    let titles_7: Vec<_> = generate_corpus(&mut seeded_rng(7))
        .memories.iter().map(|m| m.title.clone()).collect();
    assert_eq!(titles_42, titles_7, "title list must be seed-invariant");
}
```

---

### MED-2 — D6 max_possible reads runtime config, not spec-locked 15

**Code:** `forge_isolation.rs:602`

```rust
let max_possible = (ctx_config.decisions_limit + ctx_config.lessons_limit) as f64;
```

`ContextConfig::default()` happens to set 10+5=15 in `config.rs:500-501`,
but if any future configuration override changes the defaults (env var,
config file in scope, or a `Default` regression), `max_possible` silently
drifts. The N1 fix in spec §3.1/§3.3 is explicit about 15.

Fix options:
- (a) `let max_possible = 15.0_f64;` and add a debug_assert at top of fn
      that ctx_config.decisions_limit + ctx_config.lessons_limit == 15
- (b) keep the current code but document that the value is config-derived
      and add a regression test that asserts the default sums to 15

(a) is preferable — the spec §3.3 N1 rationale ("with denominator 120, a
5-row leak would score 0.958 and PASS") makes the literal value
load-bearing for the bench's blast radius.

---

### MED-3 — Probe 4 (SQL injection inert) discards the recall result

**Code:** `forge_isolation.rs:495`

```rust
let _ = crate::db::ops::recall_bm25_project(
    &state.conn,
    SHARED_TAG,
    Some("alpha'; DROP TABLE memory;--"),
    50,
);
```

The `let _ =` discards the Result. Today, the function returns
`Ok(empty Vec)` because (a) the project string is parameter-bound (?2),
not concatenated, and (b) `sanitize_fts5_query("isolation_bench")`
sanitizes fine. The N4 sentinel-row-hash check correctly catches
mutation, so the *property* (no DROP, no UPDATE) is asserted.

But the probe gives no signal whether the dangerous string actually
reached the binding layer or got short-circuited. A future regression
that, e.g., panics on the unusual project string would still pass this
probe (because the panic would be caught — wait, no, panic would
abort). The probe at least asserts no-mutation.

Fix: tighten to `assert!(recall_bm25_project(...).is_ok())` AND the hash
invariant. Or document the tolerance.

---

### MED-4 — Infra-failure path leaves dim scores populated alongside composite=0

**Code:** `forge_isolation.rs:847-855`

```rust
let dimensions: [DimensionScore; 6] = std::array::from_fn(|i| mark_pass(dims_raw[i].clone()));
let composite = if infra_pass { composite_score(&dimensions) } else { 0.0 };
```

Notice: the bench runs ALL 6 dim functions even when infra checks fail.
The `IsolationScore` JSON written to disk then has `composite: 0.0` but
populated dimension scores (potentially all 1.0). A reader downstream
would see an inconsistent state.

Forge-identity precedent (`forge_identity.rs:1672-1684`) returns early
with `zeroed_dimensions()`. Forge-isolation diverges.

Fix: replicate forge-identity's early-return pattern — if infra fails,
zero all dim scores AND set composite=0.0, write summary, return.

---

### MED-5 — `--expected-composite ±0.05` tolerance can mask 1-row regressions

**Code:** `bin/forge-bench.rs:885-892`

```rust
if (score.composite - expected).abs() > 0.05 {
    return Err(format!(...));
}
```

Composite is 1.0 deterministically. A 1-row regression in D6 drops a
project's D6 score from 1.0 to 0.933 (15-1)/15. Composite math:

```
0.25*1.0 + 0.15*1.0 + 0.10*1.0 + 0.10*1.0 + 0.15*1.0 + 0.25*(0.933+1.0+1.0+1.0+1.0)/5
= 0.75 + 0.25 * 0.9866
= 0.7466 + 0.2466
= 0.9933
```

Drift from expected 1.0 = 0.0067. Well under 0.05.

The dual gate (per-dim min) catches this via `score.pass = false` (D6 = 0.987 ≥ 0.95 — wait, recompute):

D6 per-project regression: one project drops to 0.933, others 1.0
D6 mean = (0.933 + 4 * 1.0) / 5 = 0.9866 ≥ 0.95 — passes per-dim min!

So a 1-row regression in ONE project is invisible to both the composite
threshold AND the per-dim min if the other 4 projects compensate. The
N1 fix promised "1-row regression scores 0.933 < 0.95 min and is
CAUGHT" — that's true *per-project*, but the bench averages across
projects. So the regression must hit MORE than one project to be caught,
OR D6 averaging is hiding signal.

This is borderline — a single 1-row regression in D6 in one project is
**not** caught. A multi-row regression IS caught. Spec §3.3 N1 wording
("1-row regression scores 0.933") is technically correct per-project but
misleading post-averaging.

Two paths:
- (a) tighten `--expected-composite` to ±0.005 (matches deterministic-1.0 claim)
- (b) change D6 to `min` instead of `mean` across projects — strict
      per-project gate

Severity MEDIUM because the per-dim min still catches *some* regressions
and the dual gate isn't completely defeated. But the documented blast
radius is overstated.

---

### LOW-1 — Test writes summary.json to /tmp

**Code:** `forge_isolation.rs:938-944`

```rust
output_dir: std::path::PathBuf::from("/tmp"),
```

Two parallel `cargo test` runs would race on `/tmp/summary.json`. Most
test code uses `tempfile::TempDir::new()`. Cosmetic; cargo test
serializes by default.

---

### LOW-2 — `pub const` on bench-internal constants

**Code:** `forge_isolation.rs:51-71`

`MAIN_PROJECTS`, `PREFIX_COLLISION_PROJECT`, etc. are `pub const`.
Forge-identity uses private/`pub(crate)`. Reduce API surface.

---

### LOW-3 — `as u64` cast on wall_duration_ms

`u128 → u64` truncates beyond ~584M years. Cosmetic; pattern exists
elsewhere in bench code.

---

### LOW-4 — Results doc `<1000` placeholder

The `wall_duration_ms` column reads `<1000` for every seed. Spec §9
requires "<1.5s on ubuntu-latest". Filling in actual measured values
(from a single-run measurement) would make the doc audit-ready instead
of placeholder-quality.

---

## 3. Properties verified resolved

(captured in YAML as RESOLVED-1 through RESOLVED-13)

- B3 fix: D1 query is `"isolation_bench"`, not empty
- N1 fix: D6 max_possible computes to 15 (not 120)
- N3 fix: D6 uses pinned `ContextInjectionConfig { session_context: true, .. }`
- N4 fix: D5 probe 4 includes sentinel-row hash
- §3.7: single shared DaemonState across all 6 dims
- T2.1 lift: `bench/common::deterministic_embedding` is canonical; forge_consolidation re-exports
- T2.2 lift: `bench/scoring::composite_score` is canonical N-dim; forge_identity wraps
- Composite + dual gate: `pass = infra && per_dim && composite ≥ 0.95`
- Determinism: `_rng` unused in generate_corpus (cross-seed invariance)
- events-namespace registry: 6 dim names match emit code
- CI matrix: continue-on-error true, --seed 42, output path templated
- harness-sync: clean
- 40+ tests across 3 modules pass under `--features bench`

---

## 4. Verdict rationale

**0 BLOCKER:** No spec-mandated invariant is violated to a degree that
prevents the bench from functioning as designed. All 6 dimensions
implement the locked formulas; the dual gate fires correctly; telemetry
emits; CI matrix runs.

**2 HIGH:** D6 alphabet over-count and infra-check-8 weakening — both
fixable in a 5-line patch. Neither blocks the bench from shipping today;
both should land before 2A-6 builds atop this primitive.

**5 MED + 4 LOW:** typical fix-wave material; none load-bearing for the
isolation property the bench is asserting.

→ `lockable-with-fixes`: address HIGH-1 and HIGH-2 in a fix-wave commit;
defer MED + LOW or fold them into the same commit.

---

## 5. Suggested fix-wave commit message

```
fix(P3-3 2A-5 review): address HIGH-1 + HIGH-2 from impl review

HIGH-1: drop alphabet_needle from D6 foreign-token sum (D5 owns
        the prefix-collision audit per spec §3.2)
HIGH-2: tighten infra check 8 to assert xml.contains("<forge-dynamic>")
        instead of just !xml.is_empty()

Optional bundled MED:
- MED-1: add corpus_titles_invariant_across_seeds test
- MED-2: hardcode max_possible = 15.0 + debug_assert
- MED-4: zero dim scores on infra-fail (forge-identity parity)
```
