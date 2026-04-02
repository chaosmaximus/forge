"""forge-graph MCP server — unified code intelligence + memory."""
import json
import sys
from pathlib import Path
from typing import Optional

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


@mcp.tool()
async def forge_index(path: Optional[str] = None) -> str:
    """Index a codebase with tree-sitter and store symbols in the graph."""
    import subprocess
    import os
    scan_path = path or os.getcwd()

    # Try forge-core binary
    for candidate in ["forge-core", "./target/release/forge-core", "../forge-core/target/release/forge-core"]:
        try:
            result = subprocess.run(
                [candidate, "index", scan_path],
                capture_output=True, text=True, timeout=120,
            )
            if result.returncode == 0:
                from forge_graph.code.ingest import ingest_symbols
                count = ingest_symbols(get_db(), result.stdout)
                return json.dumps({"indexed": count, "_meta": {"path": "deterministic", "duration_ms": 0}})
        except (FileNotFoundError, subprocess.TimeoutExpired):
            continue

    return json.dumps({"error": "forge-core binary not found. Build with: cargo build --release -p forge-core", "_meta": {"path": "deterministic"}})


@mcp.tool()
async def forge_cypher(query: str, agent_id: Optional[str] = None) -> str:
    """Execute a read-only Cypher query against code nodes. Memory nodes are blocked for security."""
    from forge_graph.auth import check_access
    from forge_graph.axon_proxy import validate_cypher_query
    from forge_graph.meta import ToolMeta

    if not check_access(agent_id, "forge_cypher"):
        raise PermissionError(f"Agent '{agent_id}' does not have access to forge_cypher")

    meta = ToolMeta()

    if not validate_cypher_query(query):
        return json.dumps({
            "error": "Query rejected: only read-only queries against code nodes (File, Function, Class, Method) are allowed. Memory nodes (Decision, Pattern, Lesson, etc.) and write operations are blocked.",
            "_meta": meta.finish()
        })

    db = get_db()
    try:
        result = db.conn.execute(query)
        rows = result.get_as_pl()
        # Convert polars DataFrame to list of dicts
        if len(rows) == 0:
            return json.dumps({"results": [], "_meta": meta.finish()})
        records = rows.to_dicts()
        return json.dumps({"results": records, "count": len(records), "_meta": meta.finish()}, default=str)
    except Exception as e:
        return json.dumps({"error": str(e), "_meta": meta.finish()})


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
