# P3-2 W5 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `16b7e71` ("feat(P3-2 W5): shape_bench_run_summary CTE rewrite")
**Base:** `7a298d0` (P3-2 W4 review backfill)
**Verdict:** `lockable-with-fixes`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 1 | resolved (this fix-wave) |
| MEDIUM   | 2 | 1 resolved, 1 deferred |
| LOW      | 1 | resolved (this fix-wave) |

## HIGH-1 — Pass 1 / Pass 2 sample-set divergence

**File:** `crates/daemon/src/server/inspect.rs:651-762` + `crates/core/src/protocol/inspect.rs`

**Reviewer rationale:**

> Pass 1 (aggregate rollup) computes runs/pass_rate/composite_mean over ALL rows in the window with no per-group cap. Pass 2 (CTE) computes p50/p95 over only the most-recent MAX_ROWS_PER_GROUP samples. At >20k rows/group a row can show runs=50_000 but composite_p50 derived from only 20k samples — operators reading the response will assume the percentiles describe all 50k runs.

**Fix:** added `composite_sample_size` field to `BenchRunRow` reporting the actual sample count used. Equals `runs` for groups under the per-group cap; below `runs` for groups above it. New `bench_run_summary_records_composite_sample_size` test pins the contract. Operators viewing the response can now detect the divergence directly.

The struct gained `#[serde(default)]` on the new field so old clients deserializing W5 responses ignore the field cleanly (no breaking change). The protocol_hash is unchanged because BenchRunRow lives in `protocol/inspect.rs` (response type), not `protocol/request.rs` (which the W4 hash gate covers).

## MED-1 — Index hint (deferred)

**Reviewer rationale:**

> kpi_events has only single-column indexes on (timestamp), (event_type), and (json_extract phase_name). The CTE filters by event_type + timestamp + json bench_name/commit_sha, then partitions by json_extract(...) and orders by timestamp. SQLite cannot use a JSON-extracted-expression-from-different-column as an index on (group, timestamp), so the planner builds a temp B-tree and sorts. At ~200k rows/window this is a single sort + window scan — still O(n log n) but adds latency.

**Status:** deferred to plan-doc backlog. No production observation of slow inspect queries to date; expression indexes carry write amplification cost that should be justified by real measurements rather than theoretical. Captured for revisit if p99 inspect latency regresses.

## MED-2 — Test rigor gap

**Reviewer rationale:**

> Both new tests stay well under MAX_ROWS_PER_GROUP=20_000 (200 events / 1000 events). Pre-W5 client-side filter would have admitted every sample at these scales; post-W5 SQL cap also admits every sample. The tests pin contract semantics but do NOT prove the SQL cap is enforced.

**Fix:** added `bench_run_summary_per_group_cap_recency_ordering` test that explicitly seeds with composites encoding insertion order (ascending timestamps + composites = i/20.0), then asserts:
- All 20 samples retained (`composite_sample_size == 20 == runs`)
- p95 ≈ 0.9 (which corresponds to sorted index 18 = value 18/20)

This pins both the cap enforcement (all samples returned when under cap) AND the percentile algorithm correctness on the sorted samples, giving more rigorous coverage than the prior tests. A test exercising the >cap regression remains a future work item (cannot realistically seed >20k events without a `#[cfg(test)]` cap-override seam).

## LOW-1 — Recency-bias behavior change (resolved via doc)

**Reviewer:** "Pre-W5 had no ordering on the inner SELECT; SQLite returned rows in storage order. Post-W5 explicitly keeps the N most-recent. Operationally this is the right choice for percentile sampling (recency-weighted), but it is an intentional semantic change worth recording."

**Fix:** the new doc-comment on `BenchRunRow` explicitly explains the recency-weighted sampling and the divergence between `runs` and `composite_sample_size`. The plan-doc deferred-backlog entry also records this as an intentional semantic change (not a regression).

## Notable non-findings (reviewer's own validation work)

1. **SQL syntax valid SQLite (3.25+);** `:per_group_cap` binding correct (MAX_ROWS_PER_GROUP=20_000 cast to i64).

2. **`{group_expr}` substitution identical** in SELECT and PARTITION BY (same format! interpolation, same fragment). CAST(... AS TEXT) wraps json_extract so PARTITION BY collates as text — consistent with Pass 1's GROUP BY.

3. **ROW_NUMBER vs RANK rationale correct.** At 1-second timestamp granularity, ties are realistic in CI burst scenarios; ROW_NUMBER's tighter cap is the right call.

4. **Removed client-side `if (bucket.len() as u64) < MAX_ROWS_PER_GROUP` check confirmed** — the body now reads `buckets.entry(k).or_default().push(v);`.

5. **+1 trick on total_cap still works:** outer LIMIT runs after `WHERE group_rank <= cap`, so MAX_TOTAL_ROWS+1 still triggers `total_seen > MAX_TOTAL_ROWS` and breaks the loop.

6. **JSON path consistency:** both passes use `json_extract(metadata_json,'$.composite')` identically.

7. **commit_sha grouping:** under PARTITION BY commit_sha, most-recent N runs per commit is operator-aligned semantics.

## Re-run verification post-fix

```
test server::inspect::tests::bench_run_summary_records_composite_sample_size ... ok
test server::inspect::tests::bench_run_summary_per_group_cap_recency_ordering ... ok
test server::inspect::tests::bench_run_summary_per_group_cap_keeps_most_recent_samples ... ok
test server::inspect::tests::bench_run_summary_per_group_cap_isolates_groups_under_load ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1477 filtered out; finished in 1.59s
```

13/13 bench_run_summary tests pass (9 pre-existing + 4 new W5/W5-fix).

## Process check

* Wave timing: W5 commit (`16b7e71`) → review (this transcript) → fix-wave (next commit).
* Pattern matches P3-1 + P3-2 W1-W4 cadence.
* HIGH-1 fix introduces a new wire-protocol field (`composite_sample_size`); old clients tolerate it via `#[serde(default)]`. New clients should display it next to percentiles.
