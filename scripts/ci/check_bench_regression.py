#!/usr/bin/env python3
"""Check bench composite regression vs prior runs.

Reads summary.json files from `current` and `prior` directories
(populated by the calling workflow via `gh run download`), computes
per-bench composite drift, and prints a markdown-formatted regression
report on stdout. Exits 0 if no regression, 1 if a regression ≥ 5%.

Usage:
    check_bench_regression.py --current DIR --prior DIR \
        --threshold 0.05 --output regression-report.md

The `current` and `prior` directories must contain per-bench
subdirectories (one per matrix entry), each with a `summary.json`
matching the schema emitted by `crates/daemon/src/bin/forge-bench.rs`.
The script tolerates missing benches in either dir (prior may have
fewer benches if the matrix expanded between runs).
"""

import argparse
import json
import sys
from pathlib import Path


def load_composites(dir_path: Path) -> dict[str, float | None]:
    """Walk `dir_path` for summary.json files; return {bench_name: composite}."""
    out: dict[str, float | None] = {}
    if not dir_path.exists():
        return out
    for sub in sorted(dir_path.iterdir()):
        if not sub.is_dir():
            continue
        # Artifact name format: bench-<name>-<sha>; strip prefix + sha to get name.
        # Tolerate either format: 'forge-isolation' or 'bench-forge-isolation-abcdef'.
        bench_name = sub.name
        if bench_name.startswith("bench-"):
            # bench-<name>-<sha>
            parts = bench_name.split("-")
            if len(parts) >= 3:
                bench_name = "-".join(parts[1:-1])
        summary_path = sub / "summary.json"
        if not summary_path.exists():
            out[bench_name] = None
            continue
        try:
            data = json.loads(summary_path.read_text())
            # forge-persist uses recovery_rate; others use composite.
            composite = data.get("composite")
            if composite is None:
                composite = data.get("recovery_rate")
            out[bench_name] = (
                float(composite) if composite is not None else None
            )
        except (json.JSONDecodeError, OSError, KeyError, ValueError):
            out[bench_name] = None
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--current", required=True, type=Path, help="dir with current run's bench artifacts")
    ap.add_argument("--prior", required=True, type=Path, help="dir with prior run's bench artifacts")
    ap.add_argument(
        "--threshold",
        type=float,
        default=0.05,
        help="composite drop threshold to flag regression (default 0.05 = 5%%)",
    )
    ap.add_argument("--output", type=Path, default=None, help="write report to file (also prints to stdout)")
    args = ap.parse_args()

    cur = load_composites(args.current)
    pri = load_composites(args.prior)

    all_benches = sorted(set(cur) | set(pri))
    rows = []
    regressions = []

    for bench in all_benches:
        cur_v = cur.get(bench)
        pri_v = pri.get(bench)
        if cur_v is None or pri_v is None:
            rows.append((bench, cur_v, pri_v, None, "missing"))
            continue
        delta = cur_v - pri_v
        # Regression = current is LOWER than prior by ≥ threshold.
        # (Improvements are positive deltas; we don't flag those.)
        is_regression = delta <= -args.threshold
        status = "REGRESSION" if is_regression else ("ok" if abs(delta) < 1e-9 else f"{delta:+.4f}")
        rows.append((bench, cur_v, pri_v, delta, status))
        if is_regression:
            regressions.append((bench, cur_v, pri_v, delta))

    # Build markdown report.
    lines = [
        "# Bench composite regression check",
        "",
        f"- Threshold: drop ≥ {args.threshold:.2%}",
        f"- Current run dir: `{args.current}`",
        f"- Prior run dir: `{args.prior}`",
        f"- Verdict: **{'REGRESSION' if regressions else 'OK'}**",
        "",
        "| bench | current | prior | delta | status |",
        "|-------|---------|-------|-------|--------|",
    ]
    for bench, cur_v, pri_v, delta, status in rows:
        cur_s = f"{cur_v:.4f}" if cur_v is not None else "—"
        pri_s = f"{pri_v:.4f}" if pri_v is not None else "—"
        delta_s = f"{delta:+.4f}" if delta is not None else "—"
        lines.append(f"| `{bench}` | {cur_s} | {pri_s} | {delta_s} | {status} |")

    if regressions:
        lines.append("")
        lines.append("## Regressions detail")
        for bench, cur_v, pri_v, delta in regressions:
            lines.append(
                f"- **{bench}**: composite dropped from {pri_v:.4f} to "
                f"{cur_v:.4f} (Δ {delta:+.4f}; threshold ≥ {args.threshold:.2%})"
            )

    report = "\n".join(lines) + "\n"
    sys.stdout.write(report)
    if args.output:
        args.output.write_text(report)

    return 1 if regressions else 0


if __name__ == "__main__":
    sys.exit(main())
