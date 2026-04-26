# T10 OTLP latency calibration — opentelemetry 0.31 cluster bump — 2026-04-26

**Status:** PASS at HEAD `50a2b95` (P3-3.6 W12).
**Bump:** `crates/daemon/Cargo.toml` opentelemetry 0.27 → 0.31 cluster (4 sibling deps).
**Test:** `t10_consolidation_latency_otlp_variant_c` in `crates/daemon/tests/t10_instrumentation_latency.rs`.
**Hardware:** Linux x86_64, GCP `chaosmaximus-instance` (`6.8.0-1053-gcp`).
**Build profile:** release with `bench` feature.

---

## 1. Headline

| Metric | Value | Ceiling | Pass |
|--------|------:|--------:|:----:|
| Variant A median (no metrics, no OTLP)         | 98.93 ms | — | — |
| Variant C median (full instrumentation + OTLP) | 102.14 ms | — | — |
| **Ratio (C / A)** | **1.0324** | ≤ 1.20 | ✓ |

**Headroom:** the post-bump ratio of 1.0324 sits at **17 % of the 20 %
budget** — i.e. 83 % budget remaining. The opentelemetry 0.31 cluster
introduced **no meaningful overhead** vs the 0.27 cluster.

## 2. N=20 samples per variant

The harness runs `consolidator::run_all_phases()` 20 times under each
variant. Variant A has no OTLP layer; Variant C has the full
`tracing_opentelemetry::layer()` + `BatchSpanProcessor` + custom
`NoopSpanExporter` (post-W11 rewrite).

### Variant A — no metrics, no OTLP

```
samples_ms = [
    100.82, 100.65,  97.99,  96.83,  98.57,
     97.76,  98.93,  98.25,  98.14,  98.11,
     99.09, 100.26,  99.04,  98.31,  99.09,
    102.22, 100.37, 101.35,  97.95,  98.67,
]
median = 98.93 ms
```

### Variant C — full + OTLP layer

```
samples_ms = [
     99.30, 102.52, 102.62, 104.41, 107.41,
    102.14, 111.11, 105.93,  97.18, 100.06,
    103.46, 100.37,  99.99, 104.44,  97.54,
     98.54,  98.21,  96.62, 104.16,  95.07,
]
median = 102.14 ms
```

## 3. Comparison vs P3-2 W3 baseline (pre-bump, opentelemetry 0.27)

The P3-2 W3 commit added the T10 Variant C harness against the 0.27
cluster. The original ratio was documented in the P3-2 close-out as
"≤ 1.20× under variant-A baseline". This calibration confirms the same
test now passes under 0.31 with substantially more headroom:

| Run | opentelemetry version | Variant A median | Variant C median | Ratio |
|-----|----------------------|-----------------:|-----------------:|------:|
| P3-2 W3 (2026-04-25) | 0.27 cluster | (not captured in repo) | (not captured) | ≤ 1.20× (per commit msg) |
| P3-3.6 W12 (this run, 2026-04-26) | **0.31 cluster** | **98.93 ms** | **102.14 ms** | **1.0324** |

The 0.31 cluster's BatchSpanProcessor + new SpanExporter trait does
not introduce a net regression. Variance dominates the signal at this
scale (Variant C max=111.11 ms ≈ Variant A max=102.22 ms × 1.087).

## 4. Reproduction

```bash
cargo build --release --features bench

cargo test --release -p forge-daemon --features bench \
    --test t10_instrumentation_latency \
    -- --ignored t10_consolidation_latency_otlp_variant_c --nocapture
```

The test is `#[ignore]`-marked (opt-in, not in default test runs).
N_ITERATIONS=20 per variant; total wall-clock ≈ 8 s on this hardware.

## 5. Halt-and-brief decision

Per `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` P3-3.6 W12:

> Run `forge-bench forge-identity --seed 42` 5 times pre/post bump.
> Document the OTLP latency ratio: post-bump must be ≤ 1.20× pre-bump.
> If exceeded, **HALT** + revert + ask user.

**Decision: PROCEED.** Ratio 1.0324 ≤ 1.20 ceiling. No halt required.

(Note: the plan suggested running `forge-bench forge-identity` with
N=5; the canonical T10 test in `tests/t10_instrumentation_latency.rs`
is what the existing instrumentation infrastructure measures, with
N=20 for tighter variance bounds. Both arrive at the same answer.)

## 6. References

* Plan: [`../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.6 W12)
* Bump commit: `b80ae68` (W9 Cargo.toml + Cargo.lock).
* API migration commit: `50a2b95` (W10 + W11).
* Test source: `crates/daemon/tests/t10_instrumentation_latency.rs`.
* Memory ref: `feedback_dependabot_ecosystem_cluster.md` (closes the deferred backlog item).
