#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check_license_manifest.py — validate the SPDX sidecar manifest.

Phase 2P-1b §3. The manifest at .claude-plugin/LICENSES.yaml declares
SPDX licenses for every shipped JSON file. This validator asserts:

  1. Schema v1 fields are present + well-formed.
  2. Every entry in `files[]` exists on disk and has a valid SPDX-style
     license token.
  3. Coverage: every *.json under each `coverage_paths[]` directory
     (recursively) is listed in `files[]`. New JSON files that haven't
     been added to the manifest fail this gate.
  4. Path containment: paths are relative and don't escape the repo via
     `..` or absolute prefixes (mirrors the W2 review-artifact guard).

Usage:
    check_license_manifest.py [--root <repo-root>]
                              [--manifest <path-from-root>]
"""
from __future__ import annotations

import argparse
import os
import re
import sys

try:
    import yaml
except ImportError:
    sys.stderr.write(
        "ERROR: PyYAML not installed (apt install python3-yaml or "
        "pip install pyyaml)\n"
    )
    sys.exit(2)


SCHEMA_VERSION = 1
# Per-token SPDX identifier: starts with a letter, then alphanumeric/dot/plus/
# hyphen. Compound expressions are validated by tokenising the input and
# checking each token alternates with an operator.
SPDX_TOKEN_RE = re.compile(r"^[A-Za-z][A-Za-z0-9.+\-]*$")
SPDX_OPERATORS = {"AND", "OR", "WITH"}


def err(msg: str) -> None:
    sys.stderr.write(f"FAIL: {msg}\n")


def warn(msg: str) -> None:
    sys.stderr.write(f"WARN: {msg}\n")


def _is_valid_spdx(expr: object) -> bool:
    """Validate an SPDX license expression.

    Accepts a single license id (`Apache-2.0`), conjunction/disjunction
    (`Apache-2.0 OR MIT`, `Apache-2.0 AND MIT`), the WITH operator
    (`MIT WITH Classpath-exception-2.0`), and parenthesised groups
    (`(Apache-2.0 OR MIT)`). Rejects whitespace-only strings and any
    free-form prose.
    """
    if not isinstance(expr, str):
        return False
    s = expr.strip()
    if not s:
        return False
    # Drop parens (treat them as separators); the resulting token stream
    # must alternate license-id, operator, license-id, ...
    cleaned = re.sub(r"[()]", " ", s)
    tokens = cleaned.split()
    if not tokens:
        return False
    # First token must be a license id.
    if tokens[0] in SPDX_OPERATORS or not SPDX_TOKEN_RE.match(tokens[0]):
        return False
    expecting_op = True
    for t in tokens[1:]:
        if expecting_op:
            if t not in SPDX_OPERATORS:
                return False
        else:
            if t in SPDX_OPERATORS or not SPDX_TOKEN_RE.match(t):
                return False
        expecting_op = not expecting_op
    # After the loop expecting_op must be True (we just consumed a
    # license-id); False means the expression ended on a dangling
    # operator (e.g. "Apache-2.0 OR").
    return expecting_op


def _path_within_repo(repo_root: str, p: str) -> bool:
    if os.path.isabs(p):
        return False
    full = os.path.realpath(os.path.join(repo_root, p))
    root = os.path.realpath(repo_root)
    return full == root or full.startswith(root + os.sep)


def validate(manifest: dict, repo_root: str) -> int:
    errors = 0

    sv = manifest.get("schema_version")
    if sv != SCHEMA_VERSION:
        err(
            f"schema_version must be {SCHEMA_VERSION} "
            f"(got {sv!r}, type {type(sv).__name__}) — "
            f"YAML scalar `1` parses as int; "
            f"`'1'` (quoted) parses as str and is rejected"
        )
        errors += 1

    pkg = manifest.get("package")
    if not isinstance(pkg, str) or not pkg.strip():
        err("`package` must be a non-empty string")
        errors += 1

    top_license = manifest.get("license")
    if not _is_valid_spdx(top_license):
        err(
            f"top-level `license` must be a valid SPDX expression "
            f"(got {top_license!r})"
        )
        errors += 1

    files = manifest.get("files") or []
    if not isinstance(files, list) or not files:
        err("`files` must be a non-empty list")
        errors += 1
        files = []

    listed_paths: set[str] = set()
    for fi, entry in enumerate(files):
        if not isinstance(entry, dict):
            err(f"files[{fi}]: must be a mapping")
            errors += 1
            continue
        p = entry.get("path")
        lic = entry.get("license")
        if not isinstance(p, str) or not p.strip():
            err(f"files[{fi}].path must be a non-empty string")
            errors += 1
            continue
        if not _path_within_repo(repo_root, p):
            err(f"files[{fi}].path escapes repo root or is absolute: {p}")
            errors += 1
            continue
        if not os.path.exists(os.path.join(repo_root, p)):
            err(f"files[{fi}].path not found in repo: {p}")
            errors += 1
        if not _is_valid_spdx(lic):
            err(
                f"files[{fi}].license must be a valid SPDX expression "
                f"(got {lic!r})"
            )
            errors += 1
        # Normalise the listed path for coverage diff.
        listed_paths.add(os.path.normpath(p))

    coverage_paths = manifest.get("coverage_paths") or []
    if not isinstance(coverage_paths, list):
        err("`coverage_paths` must be a list (or absent)")
        errors += 1
        coverage_paths = []
    if not coverage_paths:
        warn(
            "no `coverage_paths` defined; the coverage gate is effectively "
            "skipped — only files[] existence is checked"
        )

    discovered_jsons: set[str] = set()
    for ci, cp in enumerate(coverage_paths):
        if not isinstance(cp, str) or not cp.strip():
            err(f"coverage_paths[{ci}] must be a non-empty string")
            errors += 1
            continue
        if not _path_within_repo(repo_root, cp):
            err(
                f"coverage_paths[{ci}] escapes repo root or is absolute: {cp}"
            )
            errors += 1
            continue
        full = os.path.join(repo_root, cp)
        if not os.path.isdir(full):
            err(f"coverage_paths[{ci}] not a directory: {cp}")
            errors += 1
            continue
        for dirpath, _, filenames in os.walk(full):
            for fn in filenames:
                if not fn.endswith(".json"):
                    continue
                abs_p = os.path.join(dirpath, fn)
                rel_p = os.path.relpath(abs_p, repo_root)
                discovered_jsons.add(os.path.normpath(rel_p))

    missing = sorted(discovered_jsons - listed_paths)
    if missing:
        for m in missing:
            err(
                f"coverage gap: {m} found under coverage_paths but not "
                f"listed in files[]"
            )
            errors += 1

    refs = manifest.get("references") or []
    if refs and not isinstance(refs, list):
        err("`references` must be a list (or absent)")
        errors += 1
    else:
        for ri, r in enumerate(refs):
            if not isinstance(r, str):
                err(f"references[{ri}] must be a string (got {r!r})")
                errors += 1
                continue
            if not _path_within_repo(repo_root, r):
                err(
                    f"references[{ri}] escapes repo root or is absolute: {r}"
                )
                errors += 1
                continue
            if not os.path.exists(os.path.join(repo_root, r)):
                err(f"references[{ri}] not found in repo: {r}")
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
        "--manifest",
        default=".claude-plugin/LICENSES.yaml",
        help="manifest path relative to --root (default: %(default)s)",
    )
    args = parser.parse_args()

    repo_root = os.path.abspath(args.root)
    manifest_path = os.path.join(repo_root, args.manifest)
    if not os.path.exists(manifest_path):
        err(f"missing manifest: {manifest_path}")
        return 2

    try:
        with open(manifest_path) as f:
            manifest = yaml.safe_load(f)
    except (yaml.YAMLError, ValueError, TypeError) as e:
        err(f"invalid YAML in {args.manifest}: {e}")
        return 1

    if not isinstance(manifest, dict):
        err(f"top-level of {args.manifest} must be a mapping")
        return 1

    errors = validate(manifest, repo_root)
    if errors:
        sys.stderr.write(f"\nlicense-manifest: {errors} validation error(s)\n")
        return 1

    listed = len(manifest.get("files") or [])
    print(
        f"license-manifest: OK — {listed} file(s) declared, coverage clean"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # pragma: no cover (defensive top-level)
        # Any uncaught exception (PermissionError, OSError mid-walk, etc.)
        # surfaces as a single FAIL line plus exit 2 instead of a stack
        # trace that obscures the user-facing diagnostic.
        err(f"unhandled exception: {type(e).__name__}: {e}")
        sys.exit(2)
