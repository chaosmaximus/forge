"""forge-graph MCP server — unified code intelligence + memory."""
import json
import os
import sys
from pathlib import Path
from typing import Optional

# Import shared mcp instance and DB accessor from app.py (breaks circular imports)
from forge_graph.app import mcp, get_db, set_db

_hud = None  # HudStateWriter, initialized on startup


def update_hud() -> None:
    """Update HUD state file with current graph stats. Called after tool operations."""
    db = None
    try:
        db = get_db()
    except RuntimeError:
        return
    if _hud is None or db is None:
        return
    try:
        counts = {}
        for label in ("Decision", "Pattern", "Lesson"):
            r = db.conn.execute(f"MATCH (n:{label}) WHERE n.invalid_at IS NULL RETURN count(n) AS c")
            rows = r.get_as_pl()
            counts[label.lower()] = int(rows["c"][0]) if len(rows) > 0 else 0
        # Secret uses status
        r = db.conn.execute("MATCH (n:Secret) WHERE n.status = 'active' RETURN count(n) AS c")
        rows = r.get_as_pl()
        counts["secret"] = int(rows["c"][0]) if len(rows) > 0 else 0

        r = db.conn.execute("MATCH (n) RETURN count(n) AS c")
        nodes = int(r.get_as_pl()["c"][0])
        r = db.conn.execute("MATCH ()-[r]->() RETURN count(r) AS c")
        edges = int(r.get_as_pl()["c"][0])

        _hud.update(
            graph={"nodes": nodes, "edges": edges},
            memory={
                "decisions": counts.get("decision", 0),
                "patterns": counts.get("pattern", 0),
                "lessons": counts.get("lesson", 0),
                "secrets": counts.get("secret", 0),
            },
        )
        _hud.maybe_flush()
    except Exception:
        pass  # HUD update is best-effort, never block tool operations


def init_db(db_path: str | Path):
    from forge_graph.db import GraphDB
    db = GraphDB(db_path)
    set_db(db)
    return db


# ─── Tools defined in server.py (always available) ──────────────────────────

@mcp.tool()
async def forge_health() -> str:
    """Health check."""
    db = get_db()
    result = db.conn.execute("MATCH (n) RETURN count(n) AS nodes")
    rows = result.get_as_pl()
    node_count = rows["nodes"][0] if len(rows) > 0 else 0
    return json.dumps({"status": "ok", "nodes": int(node_count)})


@mcp.tool()
async def forge_index(path: Optional[str] = None) -> str:
    """Index a codebase with tree-sitter via forge-core and store symbols in the graph."""
    from forge_graph.cli_bridge import run_forge_core
    from forge_graph.code.ingest import ingest_symbols
    from forge_graph.meta import ToolMeta

    meta = ToolMeta()
    scan_path = path or os.getcwd()

    result = run_forge_core(["index", scan_path])
    if result.returncode != 0:
        return json.dumps({"error": result.stderr.strip(), "_meta": meta.finish()})

    count = ingest_symbols(get_db(), result.stdout)
    return json.dumps({"indexed": count, "_meta": meta.finish()})


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
        if len(rows) == 0:
            return json.dumps({"results": [], "_meta": meta.finish()})
        records = rows.to_dicts()
        return json.dumps({"results": records, "count": len(records), "_meta": meta.finish()}, default=str)
    except Exception as e:
        return json.dumps({"error": str(e), "_meta": meta.finish()})


# ─── Register ALL tool modules at import time ───────────────────────────────
# Now that mcp lives in app.py, there's no circular import.
# memory/tools.py and security/tools.py import from app.py, not server.py.

from forge_graph.memory import tools as _memory_tools  # noqa: F401, E402
from forge_graph.security import tools as _security_tools  # noqa: F401, E402


# ─── Server startup ─────────────────────────────────────────────────────────

def _init_on_startup(db_path: str) -> None:
    """Initialize DB schema, HUD writer, and write initial state."""
    global _hud
    db = init_db(db_path)

    # Ensure schema exists
    from forge_graph.memory.schema import create_schema
    create_schema(db.conn)

    # Initialize HUD writer
    data_dir = os.environ.get("CLAUDE_PLUGIN_DATA", "")
    if data_dir:
        from forge_graph.hud.state import HudStateWriter
        _hud = HudStateWriter(os.path.join(data_dir, "hud", "hud-state.json"))
        skills_dir = os.path.join(os.environ.get("CLAUDE_PLUGIN_ROOT", ""), "skills")
        skill_count = 0
        if os.path.isdir(skills_dir):
            skill_count = len([f for f in os.listdir(skills_dir) if f.endswith(".md")])
        _hud.update(skills={"active": skill_count, "fix_candidates": 0})
        update_hud()
        _hud.flush()


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser(description="forge-graph MCP server")
    parser.add_argument("command", choices=["serve"])
    parser.add_argument("--db", required=True, help="Path to .lbdb file")
    args = parser.parse_args()

    _init_on_startup(args.db)
    mcp.run()


if __name__ == "__main__":
    main()
