"""Shared FastMCP instance and DB accessor — breaks circular imports.

All tool modules import `mcp` and `get_db` from here, not from server.py.
server.py imports from here too. No circular dependency.
"""
from mcp.server.fastmcp import FastMCP
from forge_graph.db import GraphDB

mcp = FastMCP("forge-graph")
_db: GraphDB | None = None


def get_db() -> GraphDB:
    if _db is None:
        raise RuntimeError("Database not initialized")
    return _db


def set_db(db: GraphDB) -> None:
    global _db
    _db = db
