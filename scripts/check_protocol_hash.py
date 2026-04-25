#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check_protocol_hash.py — Phase 2P-1b §7 2A-4d interlock.

Asserts that `.claude-plugin/plugin.json`'s `protocol_hash` field equals
SHA-256 of `crates/core/src/protocol/request.rs`. Forces every PR that
mutates the daemon's authoritative protocol surface to also bump the
plugin manifest in lockstep — a missing bump fails CI loudly with a
copy-pasteable fix command.

Usage:
    check_protocol_hash.py [--root <repo-root>]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys


PROTOCOL_FILE = "crates/core/src/protocol/request.rs"
PLUGIN_FILE = ".claude-plugin/plugin.json"


def err(msg: str) -> None:
    sys.stderr.write(f"FAIL: {msg}\n")


def compute_sha256(path: str) -> str:
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=os.environ.get("REPO_ROOT") or os.getcwd(),
        help="repository root (default: cwd or $REPO_ROOT)",
    )
    parser.add_argument(
        "--protocol-file",
        default=PROTOCOL_FILE,
        help="protocol source path relative to --root (default: %(default)s)",
    )
    parser.add_argument(
        "--plugin-file",
        default=PLUGIN_FILE,
        help="plugin manifest path relative to --root (default: %(default)s)",
    )
    args = parser.parse_args()

    repo_root = os.path.abspath(args.root)
    proto = os.path.join(repo_root, args.protocol_file)
    plugin = os.path.join(repo_root, args.plugin_file)

    if not os.path.exists(proto):
        err(f"missing protocol source: {args.protocol_file}")
        return 2
    if not os.path.exists(plugin):
        err(f"missing plugin manifest: {args.plugin_file}")
        return 2

    try:
        expected = compute_sha256(proto)
    except OSError as e:
        err(f"cannot read {args.protocol_file}: {e}")
        return 2

    try:
        with open(plugin) as f:
            manifest = json.load(f)
    except (json.JSONDecodeError, OSError) as e:
        err(f"cannot parse {args.plugin_file}: {e}")
        return 1

    actual = manifest.get("protocol_hash")
    if actual is None:
        err(
            f"{args.plugin_file} is missing the 'protocol_hash' field. "
            f"Add it: \"protocol_hash\": \"{expected}\""
        )
        return 1

    if not isinstance(actual, str):
        err(f"'protocol_hash' must be a string (got {type(actual).__name__})")
        return 1

    if actual != expected:
        err(
            f"protocol_hash drift between {args.protocol_file} and "
            f"{args.plugin_file}:\n"
            f"  expected (SHA-256 of source): {expected}\n"
            f"  plugin.json currently:        {actual}\n"
            f"  fix: bash scripts/sync-protocol-hash.sh && "
            f"git add {args.plugin_file}"
        )
        return 1

    print(
        f"protocol-hash: OK — {args.protocol_file} ↔ {args.plugin_file} "
        f"in sync ({expected[:12]}…)"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # pragma: no cover
        err(f"unhandled exception: {type(e).__name__}: {e}")
        sys.exit(2)
