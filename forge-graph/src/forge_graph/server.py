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


def _init_on_startup(db_path: str) -> None:
    """Initialize DB schema and write initial HUD state on server start."""
    import os
    db = init_db(db_path)

    # Ensure schema exists
    from forge_graph.memory.schema import create_schema
    create_schema(db.conn)

    # Write initial HUD state
    data_dir = os.environ.get("CLAUDE_PLUGIN_DATA", "")
    if data_dir:
        from forge_graph.hud.state import HudStateWriter
        hud = HudStateWriter(os.path.join(data_dir, "hud", "hud-state.json"))
        try:
            r = db.conn.execute("MATCH (n) RETURN count(n) AS c")
            nodes = r.get_next()[0] if r.has_next() else 0
            r2 = db.conn.execute("MATCH ()-[r]->() RETURN count(r) AS c")
            edges = r2.get_next()[0] if r2.has_next() else 0
        except Exception:
            nodes, edges = 0, 0
        hud.update(graph={"nodes": int(nodes), "edges": int(edges)})
        hud.flush()


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser(description="forge-graph MCP server")
    parser.add_argument("command", choices=["serve"])
    parser.add_argument("--db", required=True, help="Path to .lbdb file")
    args = parser.parse_args()

    _init_on_startup(args.db)

    # Import tool modules to register them
    from forge_graph.memory import tools as _mt  # noqa: F401
    from forge_graph.security import tools as _st  # noqa: F401

    mcp.run()


if __name__ == "__main__":
    main()
