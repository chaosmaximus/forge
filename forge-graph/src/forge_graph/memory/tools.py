"""Memory tools — forge_remember, forge_recall, forge_link.

Registered as MCP tools on the shared ``mcp`` instance from server.py.
"""
from __future__ import annotations

import json
import uuid
from typing import Any

from forge_graph.auth import check_access
from forge_graph.db import GraphDB
from forge_graph.memory.temporal import CURRENT_VIEW
from forge_graph.meta import ToolMeta
from forge_graph.server import mcp, get_db

# ---------------------------------------------------------------------------
# Allowed types / edge labels
# ---------------------------------------------------------------------------
_ALLOWED_TYPES = frozenset({"decision", "pattern", "lesson", "preference"})

_ALLOWED_EDGE_TYPES = frozenset({
    "SUPERSEDES", "MOTIVATED_BY", "FOLLOWS", "CONTRADICTS",
    "LEARNED_IN", "DECIDED_IN", "EVOLVED_FROM", "APPLIED_IN",
})

# Mapping from type name to (label, searchable text fields)
_TYPE_INFO: dict[str, tuple[str, list[str]]] = {
    "decision": ("Decision", ["title", "rationale"]),
    "pattern":  ("Pattern",  ["name", "description"]),
    "lesson":   ("Lesson",   ["insight", "context"]),
    "preference": ("Preference", ["key", "value"]),
}

# ---------------------------------------------------------------------------
# Internal implementations (accept db explicitly for testability)
# ---------------------------------------------------------------------------


async def _remember_impl(
    db: GraphDB,
    type: str,
    structured: dict[str, Any],
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_remember"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_remember"
        )

    if type not in _ALLOWED_TYPES:
        raise ValueError(
            f"Unknown type '{type}'. Allowed: {sorted(_ALLOWED_TYPES)}"
        )

    label = _TYPE_INFO[type][0]
    node_id = f"{type}-{uuid.uuid4().hex[:12]}"

    # Build parameter dict: always include id + timestamps
    params: dict[str, Any] = {
        "id": node_id,
        **structured,
    }

    # Build the property list for the CREATE clause
    prop_parts = [f"{k}: ${k}" for k in params]
    prop_parts.append("created_at: current_timestamp()")
    prop_parts.append("updated_at: current_timestamp()")
    prop_parts.append("valid_at: current_timestamp()")
    props_str = ", ".join(prop_parts)

    query = f"CREATE (n:{label} {{{props_str}}})"
    await db.write(query, parameters=params)

    return json.dumps({"node_id": node_id, "_meta": meta.finish()})


async def _recall_impl(
    db: GraphDB,
    query: str,
    type: str | None = None,
    include_historical: bool = False,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_recall"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_recall"
        )

    results: list[dict[str, Any]] = []

    # Determine which types to search
    if type is not None:
        if type not in _ALLOWED_TYPES:
            raise ValueError(
                f"Unknown type '{type}'. Allowed: {sorted(_ALLOWED_TYPES)}"
            )
        types_to_search = [type]
    else:
        types_to_search = list(_ALLOWED_TYPES)

    for t in types_to_search:
        label, fields = _TYPE_INFO[t]

        # Build CONTAINS conditions for each searchable field
        contains_clauses = [f"n.{f} CONTAINS $query" for f in fields]
        text_filter = " OR ".join(contains_clauses)

        # Temporal filter
        if include_historical:
            temporal_clause = ""
        else:
            temporal_clause = f"AND {CURRENT_VIEW('n')}"

        cypher = (
            f"MATCH (n:{label}) "
            f"WHERE ({text_filter}) {temporal_clause} "
            f"RETURN n.id, n.{fields[0]}, n.{fields[1]}"
        )

        result = await db.execute(cypher, parameters={"query": query})
        while result.has_next():
            row = result.get_next()
            results.append({
                "id": row[0],
                "type": t,
                fields[0]: row[1],
                fields[1]: row[2],
            })

    return json.dumps({"results": results, "_meta": meta.finish()})


async def _link_impl(
    db: GraphDB,
    from_id: str,
    to_id: str,
    edge_type: str,
    from_label: str,
    to_label: str,
    properties: dict[str, Any] | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_link"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_link"
        )

    if edge_type not in _ALLOWED_EDGE_TYPES:
        raise ValueError(
            f"Invalid edge_type '{edge_type}'. "
            f"Allowed: {sorted(_ALLOWED_EDGE_TYPES)}"
        )

    # Build property string for edge
    params: dict[str, Any] = {"from_id": from_id, "to_id": to_id}

    if properties:
        prop_parts = [f"{k}: ${k}" for k in properties]
        props_str = " {" + ", ".join(prop_parts) + "}"
        params.update(properties)
    else:
        props_str = ""

    cypher = (
        f"MATCH (a:{from_label}), (b:{to_label}) "
        f"WHERE a.id = $from_id AND b.id = $to_id "
        f"CREATE (a)-[r:{edge_type}{props_str}]->(b)"
    )

    await db.write(cypher, parameters=params)

    return json.dumps({"status": "linked", "_meta": meta.finish()})


# ---------------------------------------------------------------------------
# MCP tool registrations
# ---------------------------------------------------------------------------


@mcp.tool()
async def forge_remember(
    type: str,
    structured: dict[str, Any],
    agent_id: str | None = None,
) -> str:
    """Store a memory node (decision, pattern, lesson, or preference)."""
    return await _remember_impl(get_db(), type, structured, agent_id)


@mcp.tool()
async def forge_recall(
    query: str,
    type: str | None = None,
    include_historical: bool = False,
    agent_id: str | None = None,
) -> str:
    """Search memory nodes by keyword."""
    return await _recall_impl(get_db(), query, type, include_historical, agent_id)


@mcp.tool()
async def forge_link(
    from_id: str,
    to_id: str,
    edge_type: str,
    from_label: str,
    to_label: str,
    properties: dict[str, Any] | None = None,
    agent_id: str | None = None,
) -> str:
    """Create an edge between two memory nodes."""
    return await _link_impl(
        get_db(), from_id, to_id, edge_type,
        from_label, to_label, properties, agent_id,
    )
