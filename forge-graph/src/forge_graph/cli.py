"""forge-graph CLI — stateless graph operations.

Opens DB per invocation, operates, closes, exits. No persistent process.
Replaces the MCP server entirely.

Usage: python3 -m forge_graph.cli --db <path> <command> [args]
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema


def get_db(db_path: str) -> GraphDB:
    """Open DB (creates if needed), ensure schema, return handle."""
    Path(db_path).parent.mkdir(parents=True, exist_ok=True)
    db = GraphDB(db_path)
    create_schema(db.conn)
    return db


def cmd_remember(db: GraphDB, args) -> None:
    from forge_graph.memory.ops import remember
    data = json.loads(args.data)
    result = remember(db, args.type, data)
    print(json.dumps(result, default=str))


def cmd_recall(db: GraphDB, args) -> None:
    from forge_graph.memory.ops import recall
    result = recall(db, args.query, args.type, args.include_historical)
    print(json.dumps(result, default=str))


def cmd_forget(db: GraphDB, args) -> None:
    from forge_graph.memory.ops import forget
    result = forget(db, args.node_id, args.label, args.reason)
    print(json.dumps(result, default=str))


def cmd_health(db: GraphDB, _args) -> None:
    from forge_graph.memory.ops import health
    result = health(db)
    print(json.dumps(result, default=str))


def cmd_query(db: GraphDB, args) -> None:
    from forge_graph.axon_proxy import validate_cypher_query
    if not validate_cypher_query(args.cypher):
        print(json.dumps({"error": "Query rejected by Cypher sandbox"}))
        return
    try:
        result = db.conn.execute(args.cypher)
        rows = result.get_as_pl()
        if len(rows) == 0:
            print(json.dumps({"results": [], "count": 0}))
        else:
            print(json.dumps({"results": rows.to_dicts(), "count": len(rows)}, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e)}))


def cmd_sync(db: GraphDB, args) -> None:
    from forge_graph.memory.ops import sync_pending
    count = sync_pending(db, args.pending_path)
    print(json.dumps({"synced": count}))


def cmd_index(db: GraphDB, args) -> None:
    """Index codebase symbols into graph via forge-core index output."""
    import subprocess
    from forge_graph.code.ingest import ingest_symbols

    result = subprocess.run(
        ["forge-core", "index", args.path],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        # Try with full path
        import os
        plugin_root = os.environ.get("CLAUDE_PLUGIN_ROOT", "")
        forge_core = os.path.join(plugin_root, "servers", "forge-core") if plugin_root else "forge-core"
        result = subprocess.run(
            [forge_core, "index", args.path],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(json.dumps({"error": result.stderr.strip()}))
            return

    count = ingest_symbols(db, result.stdout)
    print(json.dumps({"indexed": count}))


def main() -> None:
    parser = argparse.ArgumentParser(description="forge-graph CLI — stateless graph operations")
    parser.add_argument("--db", required=True, help="Path to .lbdb database file")
    sub = parser.add_subparsers(dest="command")

    # remember
    p = sub.add_parser("remember", help="Store a memory node")
    p.add_argument("--type", required=True, choices=["decision", "pattern", "lesson", "preference"])
    p.add_argument("--data", required=True, help="JSON string with node fields")

    # recall
    p = sub.add_parser("recall", help="Search memory by keyword")
    p.add_argument("query", help="Search keyword")
    p.add_argument("--type", default=None, choices=["decision", "pattern", "lesson", "preference"])
    p.add_argument("--include-historical", action="store_true")

    # forget
    p = sub.add_parser("forget", help="Soft-delete a memory node")
    p.add_argument("node_id", help="Node ID to forget")
    p.add_argument("--label", required=True, choices=["Decision", "Pattern", "Lesson", "Preference"])
    p.add_argument("--reason", default="")

    # health
    sub.add_parser("health", help="Graph health check")

    # query
    p = sub.add_parser("query", help="Execute read-only Cypher query")
    p.add_argument("cypher", help="Cypher query string")

    # sync
    p = sub.add_parser("sync", help="Sync pending.jsonl entries to graph")
    p.add_argument("--pending-path", required=True, help="Path to pending.jsonl")

    # index
    p = sub.add_parser("index", help="Index codebase symbols into graph")
    p.add_argument("path", help="Path to index")

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(1)

    db = get_db(args.db)
    try:
        cmds = {
            "remember": cmd_remember, "recall": cmd_recall, "forget": cmd_forget,
            "health": cmd_health, "query": cmd_query, "sync": cmd_sync,
            "index": cmd_index,
        }
        cmds[args.command](db, args)
    finally:
        db.close()


if __name__ == "__main__":
    main()
