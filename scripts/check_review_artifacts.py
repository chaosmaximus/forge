#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check_review_artifacts.py — validate adversarial review YAML artifacts.

Phase 2P-1b §2. Asserts every YAML in $REVIEWS_DIR matches schema v1, has
non-empty artifacts, no open BLOCKER/CRITICAL/HIGH findings, and lists
target_paths that exist in the working tree.

Usage:
    check_review_artifacts.py [--root <repo-root>]

Exits 0 if all reviews valid, 1 on validation errors, 2 on usage error.
"""
from __future__ import annotations

import argparse
import glob
import os
import sys
from typing import Any

try:
    import yaml
except ImportError:
    sys.stderr.write(
        "ERROR: PyYAML not installed (apt install python3-yaml or "
        "pip install pyyaml)\n"
    )
    sys.exit(2)


SCHEMA_VERSION = 1
ALLOWED_VERDICTS = {"lockable-as-is", "lockable-with-fixes", "not-lockable"}
ALLOWED_STATUSES = {"resolved", "deferred", "open"}
ALLOWED_SEVERITIES = {"BLOCKER", "CRITICAL", "HIGH", "MEDIUM", "LOW", "NIT"}
OPEN_BLOCKING_SEVERITIES = {"BLOCKER", "CRITICAL", "HIGH"}


def err(rel: str, msg: str) -> None:
    sys.stderr.write(f"FAIL [{rel}]: {msg}\n")


def validate_review(rel: str, data: Any, repo_root: str) -> int:
    """Return number of validation errors found in this review YAML."""
    errors = 0

    if not isinstance(data, dict):
        err(rel, "top-level must be a mapping")
        return 1

    sv = data.get("schema_version")
    if sv != SCHEMA_VERSION:
        err(rel, f"schema_version must be {SCHEMA_VERSION} (got {sv!r})")
        errors += 1

    target_paths = data.get("target_paths") or []
    if not isinstance(target_paths, list) or not target_paths:
        err(rel, "target_paths must be a non-empty list")
        errors += 1
    else:
        for p in target_paths:
            if not isinstance(p, str):
                err(rel, f"target_paths entry must be a string (got {p!r})")
                errors += 1
                continue
            if not os.path.exists(os.path.join(repo_root, p)):
                err(rel, f"target_paths entry not found in repo: {p}")
                errors += 1

    verdict = data.get("verdict")
    if verdict not in ALLOWED_VERDICTS:
        err(rel, f"verdict must be in {sorted(ALLOWED_VERDICTS)}, got {verdict!r}")
        errors += 1

    artifacts = data.get("artifacts") or []
    if not isinstance(artifacts, list) or not artifacts:
        err(rel, "artifacts must be a non-empty list")
        errors += 1
    else:
        for ai, a in enumerate(artifacts):
            if not isinstance(a, dict):
                err(rel, f"artifacts[{ai}]: must be a mapping")
                errors += 1
                continue
            if "kind" not in a or "path" not in a:
                err(rel, f"artifacts[{ai}]: must have 'kind' and 'path' keys")
                errors += 1
                continue
            ap = a["path"]
            if not isinstance(ap, str):
                err(rel, f"artifacts[{ai}].path must be a string (got {ap!r})")
                errors += 1
                continue
            if not os.path.exists(os.path.join(repo_root, ap)):
                err(rel, f"artifacts[{ai}].path not found in repo: {ap}")
                errors += 1

    findings = data.get("findings") or []
    if not isinstance(findings, list):
        err(rel, "findings must be a list (or absent)")
        errors += 1
        findings = []

    open_blockers: list[str] = []
    for fi, finding in enumerate(findings):
        if not isinstance(finding, dict):
            err(rel, f"findings[{fi}]: must be a mapping")
            errors += 1
            continue
        sev = finding.get("severity")
        status = finding.get("status")
        if sev not in ALLOWED_SEVERITIES:
            err(
                rel,
                f"findings[{fi}].severity must be in {sorted(ALLOWED_SEVERITIES)}, "
                f"got {sev!r}",
            )
            errors += 1
        if status not in ALLOWED_STATUSES:
            err(
                rel,
                f"findings[{fi}].status must be in {sorted(ALLOWED_STATUSES)}, "
                f"got {status!r}",
            )
            errors += 1
        if sev in OPEN_BLOCKING_SEVERITIES and status == "open":
            open_blockers.append(str(finding.get("id", f"#{fi}")))

    if open_blockers:
        err(
            rel,
            f"open BLOCKING-severity findings (BLOCKER/CRITICAL/HIGH must be "
            f"resolved or deferred): {open_blockers}",
        )
        errors += 1

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=os.environ.get("REPO_ROOT") or os.getcwd(),
        help="repository root (default: cwd or $REPO_ROOT)",
    )
    parser.add_argument(
        "--reviews-dir",
        default="docs/superpowers/reviews",
        help="reviews directory relative to --root (default: %(default)s)",
    )
    args = parser.parse_args()

    repo_root = os.path.abspath(args.root)
    reviews_dir = os.path.join(repo_root, args.reviews_dir)

    if not os.path.isdir(reviews_dir):
        sys.stderr.write(f"ERROR: missing reviews dir: {reviews_dir}\n")
        return 2

    yamls = sorted(glob.glob(os.path.join(reviews_dir, "*.yaml")))
    if not yamls:
        print(
            f"review-artifacts: no *.yaml found in {args.reviews_dir} — nothing to check"
        )
        return 0

    total_errors = 0
    for path in yamls:
        rel = os.path.relpath(path, repo_root)
        try:
            with open(path) as f:
                data = yaml.safe_load(f)
        except yaml.YAMLError as e:
            err(rel, f"invalid YAML: {e}")
            total_errors += 1
            continue
        total_errors += validate_review(rel, data, repo_root)

    if total_errors:
        sys.stderr.write(
            f"\nreview-artifacts: {total_errors} validation error(s) "
            f"across {len(yamls)} review(s)\n"
        )
        return 1

    print(
        f"review-artifacts: OK — {len(yamls)} review(s) valid, "
        f"no open blocking findings"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
