"""forge-graph MCP server — unified code intelligence + memory."""
import sys
from pathlib import Path

from forge_graph import kuzu_compat  # noqa: F401 — inject kuzu shim
from mcp.server.fastmcp import FastMCP
from forge_graph.db import GraphDB

mcp = FastMCP("forge-graph")
_db: GraphDB | None = None


def get_db() -> GraphDB:
    if _db is None:
        raise RuntimeError("Database not initialized")
    return _db


def init_db(db_path: str | Path) -> GraphDB:
    global _db
    _db = GraphDB(db_path)
    return _db


@mcp.tool()
async def forge_health() -> str:
    """Health check."""
    import json
    db = get_db()
    result = db.conn.execute("MATCH (n) RETURN count(n) AS nodes")
    rows = result.get_as_pl()
    node_count = rows["nodes"][0] if len(rows) > 0 else 0
    return json.dumps({"status": "ok", "nodes": int(node_count)})


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser(description="forge-graph MCP server")
    parser.add_argument("command", choices=["serve"])
    parser.add_argument("--db", required=True, help="Path to .lbdb file")
    args = parser.parse_args()
    init_db(args.db)
    mcp.run()


if __name__ == "__main__":
    main()
