"""Memory tools — forge_remember, forge_recall, forge_link, and deterministic query tools.

Registered as MCP tools on the shared ``mcp`` instance from server.py.
"""
import json
import re
import uuid
from typing import Any, Dict, Optional

# P0: Strict regex for Cypher property keys — prevents injection via dict keys
_SAFE_KEY = re.compile(r'^[A-Za-z_][A-Za-z0-9_]{0,63}$')

from forge_graph.auth import check_access
from forge_graph.db import GraphDB
from forge_graph.memory.temporal import CURRENT_VIEW
from forge_graph.memory.trust import sanitize_for_context
from forge_graph.meta import ToolMeta
from forge_graph.server import mcp, get_db

# ---------------------------------------------------------------------------
# Query-limit helpers (P2-1: prevent unbounded result sets)
# ---------------------------------------------------------------------------
_DEFAULT_LIMIT = 20
_MAX_LIMIT = 100


def _clamp_limit(limit: int | None) -> int:
    """Clamp a user-supplied limit to [1, 100], defaulting to 20."""
    if limit is None:
        return _DEFAULT_LIMIT
    return min(max(limit, 1), _MAX_LIMIT)


def _clamp_depth(depth: int | None, default: int = 10) -> int:
    """Clamp a user-supplied depth to [1, 20]."""
    if depth is None:
        return default
    return min(max(depth, 1), 20)

# ---------------------------------------------------------------------------
# Allowed types / edge labels
# ---------------------------------------------------------------------------
_ALLOWED_TYPES = frozenset({"decision", "pattern", "lesson", "preference"})

_ALLOWED_EDGE_TYPES = frozenset({
    "SUPERSEDES", "MOTIVATED_BY", "FOLLOWS", "CONTRADICTS",
    "LEARNED_IN", "DECIDED_IN", "EVOLVED_FROM", "APPLIED_IN",
    "AFFECTS", "LOCATED_IN", "CONTAINS",
})

# Labels that support soft-delete via forge_forget
_FORGETTABLE_LABELS = frozenset({"Decision", "Pattern", "Lesson", "Preference", "Secret"})

# All valid node labels for link validation (memory + code)
ALLOWED_NODE_LABELS = frozenset({
    "Decision", "Pattern", "Lesson", "Preference", "Session", "Skill", "Secret",
    "Function", "Class", "File", "Folder", "Method", "Interface", "TypeAlias", "Enum",
})

# Labels that support timeline traversal
_TIMELINE_LABELS = frozenset({"Decision", "Skill"})

# Mapping from type name to (label, searchable text fields)
_TYPE_INFO: dict[str, tuple[str, list[str]]] = {
    "decision": ("Decision", ["title", "rationale"]),
    "pattern":  ("Pattern",  ["name", "description"]),
    "lesson":   ("Lesson",   ["insight", "context"]),
    "preference": ("Preference", ["key", "value"]),
}

# Allowed fields per node type — prevents field injection via structured dict
ALLOWED_FIELDS: dict[str, set[str]] = {
    "decision": {"title", "rationale", "confidence", "status"},
    "pattern": {"name", "description", "domain", "confidence"},
    "lesson": {"insight", "context", "severity"},
    "preference": {"key", "value", "scope", "confidence"},
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

    # Filter structured to only allowed fields — prevents field injection
    allowed = ALLOWED_FIELDS.get(type, set())
    extra_keys = set(structured.keys()) - allowed
    if extra_keys:
        import logging
        logging.getLogger("forge_graph.memory").warning(
            "Dropping disallowed fields from %s: %s", type, sorted(extra_keys)
        )
    filtered = {k: v for k, v in structured.items() if k in allowed}

    # Build parameter dict: always include id + timestamps
    params: dict[str, Any] = {
        "id": node_id,
        **filtered,
    }

    # P2-2: Ensure trust_level is set to 'user' for structured input (decisions)
    if type == "decision":
        params["trust_level"] = "user"

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
    limit: int | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()
    lim = _clamp_limit(limit)

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

        # P2-2: Include trust_level for Decision nodes
        extra_return = ", n.trust_level" if t == "decision" else ""

        cypher = (
            f"MATCH (n:{label}) "
            f"WHERE ({text_filter}) {temporal_clause} "
            f"RETURN n.id, n.{fields[0]}, n.{fields[1]}{extra_return} "
            f"LIMIT $lim"
        )

        result = await db.execute(cypher, parameters={"query": query, "lim": lim})
        while result.has_next():
            row = result.get_next()
            entry: dict[str, Any] = {
                "id": row[0],
                "type": t,
                fields[0]: row[1],
                fields[1]: row[2],
            }
            # P2-2: Expose trust_level for decisions
            if t == "decision":
                entry["trust_level"] = row[3]
            results.append(entry)

    # P2-2: Sanitize text fields before returning (defense against prompt injection)
    for entry in results:
        t = entry["type"]
        _, fields = _TYPE_INFO[t]
        for f in fields:
            if f in entry and isinstance(entry[f], str):
                entry[f] = sanitize_for_context(entry[f])

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

    # Validate labels against allowlist to prevent label injection
    if from_label not in ALLOWED_NODE_LABELS:
        raise ValueError(
            f"Invalid from_label '{from_label}'. "
            f"Allowed: {sorted(ALLOWED_NODE_LABELS)}"
        )
    if to_label not in ALLOWED_NODE_LABELS:
        raise ValueError(
            f"Invalid to_label '{to_label}'. "
            f"Allowed: {sorted(ALLOWED_NODE_LABELS)}"
        )

    # Build property string for edge
    params: dict[str, Any] = {"from_id": from_id, "to_id": to_id}

    if properties:
        # P0: Validate property keys against strict regex to prevent Cypher injection
        for k in properties.keys():
            if not _SAFE_KEY.match(k):
                raise ValueError(f"Invalid property key: {k!r}")
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


async def _patterns_impl(
    db: GraphDB,
    domain: str | None = None,
    min_confidence: float | None = None,
    limit: int | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()
    lim = _clamp_limit(limit)

    if not check_access(agent_id, "forge_patterns"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_patterns"
        )

    params: dict[str, Any] = {"lim": lim}
    filters = ["n.invalid_at IS NULL"]

    if domain is not None:
        filters.append("n.domain = $domain")
        params["domain"] = domain
    if min_confidence is not None:
        filters.append("n.confidence >= $min_conf")
        params["min_conf"] = min_confidence

    where_clause = " AND ".join(filters)
    cypher = (
        f"MATCH (n:Pattern) WHERE {where_clause} "
        f"RETURN n.id, n.name, n.description, n.domain, n.confidence "
        f"LIMIT $lim"
    )

    result = await db.execute(cypher, parameters=params)
    results: list[dict[str, Any]] = []
    while result.has_next():
        row = result.get_next()
        results.append({
            "id": row[0],
            "name": row[1],
            "description": row[2],
            "domain": row[3],
            "confidence": row[4],
        })

    return json.dumps({"results": results, "_meta": meta.finish()})


async def _forget_impl(
    db: GraphDB,
    node_id: str,
    node_label: str,
    reason: str,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_forget"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_forget"
        )

    if node_label not in _FORGETTABLE_LABELS:
        raise ValueError(
            f"Invalid node_label '{node_label}'. "
            f"Allowed: {sorted(_FORGETTABLE_LABELS)}"
        )

    # Secret uses status='revoked' instead of invalid_at (different schema)
    if node_label == "Secret":
        cypher = (
            "MATCH (n:Secret) WHERE n.id = $id "
            "SET n.status = 'revoked'"
        )
    else:
        cypher = (
            f"MATCH (n:{node_label}) WHERE n.id = $id "
            f"SET n.invalid_at = current_timestamp(), n.updated_at = current_timestamp()"
        )
    await db.write(cypher, parameters={"id": node_id})

    return json.dumps({
        "status": "forgotten",
        "node_id": node_id,
        "reason": reason,
        "_meta": meta.finish(),
    })


async def _usage_impl(
    db: GraphDB,
    session_id: str | None = None,
    last_n_sessions: int | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_usage"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_usage"
        )

    if session_id is not None:
        cypher = (
            "MATCH (s:Session) WHERE s.id = $sid "
            "RETURN s.total_tokens_input, s.total_tokens_output, "
            "s.total_llm_calls, s.total_tool_calls, s.deterministic_ratio"
        )
        result = await db.execute(cypher, parameters={"sid": session_id})
        if result.has_next():
            row = result.get_next()
            return json.dumps({
                "session_id": session_id,
                "total_tokens_input": row[0],
                "total_tokens_output": row[1],
                "total_llm_calls": row[2],
                "total_tool_calls": row[3],
                "deterministic_ratio": row[4],
                "_meta": meta.finish(),
            })
        else:
            return json.dumps({
                "session_id": session_id,
                "error": "session not found",
                "_meta": meta.finish(),
            })
    else:
        # Aggregate across sessions, optionally limited to most recent N
        lim = _clamp_limit(last_n_sessions) if last_n_sessions is not None else None

        if lim is not None:
            # Subquery: pick the N most-recent sessions by started_at, then aggregate
            cypher = (
                "MATCH (s:Session) "
                "WITH s ORDER BY s.started_at DESC LIMIT $lim "
                "RETURN SUM(s.total_tokens_input), SUM(s.total_tokens_output), "
                "SUM(s.total_llm_calls), SUM(s.total_tool_calls), "
                "AVG(s.deterministic_ratio)"
            )
            params: dict[str, Any] = {"lim": lim}
        else:
            cypher = (
                "MATCH (s:Session) "
                "RETURN SUM(s.total_tokens_input), SUM(s.total_tokens_output), "
                "SUM(s.total_llm_calls), SUM(s.total_tool_calls), "
                "AVG(s.deterministic_ratio)"
            )
            params = {}

        result = await db.execute(cypher, parameters=params)
        if result.has_next():
            row = result.get_next()
            # LadybugDB aggregates may return Decimal; coerce to native types
            return json.dumps({
                "total_tokens_input": int(row[0]) if row[0] is not None else 0,
                "total_tokens_output": int(row[1]) if row[1] is not None else 0,
                "total_llm_calls": int(row[2]) if row[2] is not None else 0,
                "total_tool_calls": int(row[3]) if row[3] is not None else 0,
                "avg_deterministic_ratio": float(row[4]) if row[4] is not None else None,
                "_meta": meta.finish(),
            })
        else:
            return json.dumps({
                "total_tokens_input": 0,
                "total_tokens_output": 0,
                "total_llm_calls": 0,
                "total_tool_calls": 0,
                "avg_deterministic_ratio": None,
                "_meta": meta.finish(),
            })


async def _decisions_impl(
    db: GraphDB,
    code_path: str | None = None,
    symbol: str | None = None,
    limit: int | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()
    lim = _clamp_limit(limit)

    if not check_access(agent_id, "forge_decisions"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_decisions"
        )

    results: list[dict[str, Any]] = []

    if code_path is not None:
        # Simple approximation: search Decision title/rationale for the path string
        cypher = (
            "MATCH (d:Decision) WHERE d.invalid_at IS NULL "
            "AND (d.title CONTAINS $path OR d.rationale CONTAINS $path) "
            "RETURN d.id, d.title, d.rationale, d.status "
            "LIMIT $lim"
        )
        result = await db.execute(cypher, parameters={"path": code_path, "lim": lim})
        while result.has_next():
            row = result.get_next()
            results.append({
                "id": row[0],
                "title": row[1],
                "rationale": row[2],
                "status": row[3],
            })
    elif symbol is not None:
        # Same approach for symbol: search title/rationale
        cypher = (
            "MATCH (d:Decision) WHERE d.invalid_at IS NULL "
            "AND (d.title CONTAINS $sym OR d.rationale CONTAINS $sym) "
            "RETURN d.id, d.title, d.rationale, d.status "
            "LIMIT $lim"
        )
        result = await db.execute(cypher, parameters={"sym": symbol, "lim": lim})
        while result.has_next():
            row = result.get_next()
            results.append({
                "id": row[0],
                "title": row[1],
                "rationale": row[2],
                "status": row[3],
            })
    else:
        # No filter: return all active decisions
        cypher = (
            "MATCH (d:Decision) WHERE d.invalid_at IS NULL "
            "RETURN d.id, d.title, d.rationale, d.status "
            "LIMIT $lim"
        )
        result = await db.execute(cypher, parameters={"lim": lim})
        while result.has_next():
            row = result.get_next()
            results.append({
                "id": row[0],
                "title": row[1],
                "rationale": row[2],
                "status": row[3],
            })

    return json.dumps({"results": results, "_meta": meta.finish()})


async def _timeline_impl(
    db: GraphDB,
    node_id: str,
    node_label: str,
    depth: int | None = None,
    agent_id: str | None = None,
) -> str:
    meta = ToolMeta()

    if not check_access(agent_id, "forge_timeline"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_timeline"
        )

    if node_label not in _TIMELINE_LABELS:
        raise ValueError(
            f"Invalid node_label '{node_label}'. "
            f"Allowed: {sorted(_TIMELINE_LABELS)}"
        )

    max_depth = _clamp_depth(depth)
    chain: list[dict[str, Any]] = []

    if node_label == "Decision":
        # Try variable-length path first; fall back to iterative single-hop
        try:
            cypher = (
                f"MATCH (start:Decision {{id: $nid}})"
                f"-[:SUPERSEDES*1..{max_depth}]->"
                f"(older:Decision) "
                f"RETURN older.id, older.title, older.status"
            )
            result = await db.execute(cypher, parameters={"nid": node_id})
            while result.has_next():
                row = result.get_next()
                chain.append({
                    "id": row[0],
                    "title": row[1],
                    "status": row[2],
                })
        except Exception:
            # Fallback: iterative single-hop traversal
            current_id = node_id
            for _ in range(max_depth):
                cypher = (
                    "MATCH (a:Decision {id: $cid})-[:SUPERSEDES]->(b:Decision) "
                    "RETURN b.id, b.title, b.status"
                )
                result = await db.execute(cypher, parameters={"cid": current_id})
                if result.has_next():
                    row = result.get_next()
                    chain.append({
                        "id": row[0],
                        "title": row[1],
                        "status": row[2],
                    })
                    current_id = row[0]
                else:
                    break

    elif node_label == "Skill":
        try:
            cypher = (
                f"MATCH (start:Skill {{id: $nid}})"
                f"-[:EVOLVED_FROM*1..{max_depth}]->"
                f"(older:Skill) "
                f"RETURN older.id, older.name, older.generation"
            )
            result = await db.execute(cypher, parameters={"nid": node_id})
            while result.has_next():
                row = result.get_next()
                chain.append({
                    "id": row[0],
                    "name": row[1],
                    "generation": row[2],
                })
        except Exception:
            # Fallback: iterative single-hop traversal
            current_id = node_id
            for _ in range(max_depth):
                cypher = (
                    "MATCH (a:Skill {id: $cid})-[:EVOLVED_FROM]->(b:Skill) "
                    "RETURN b.id, b.name, b.generation"
                )
                result = await db.execute(cypher, parameters={"cid": current_id})
                if result.has_next():
                    row = result.get_next()
                    chain.append({
                        "id": row[0],
                        "name": row[1],
                        "generation": row[2],
                    })
                    current_id = row[0]
                else:
                    break

    return json.dumps({"chain": chain, "_meta": meta.finish()})


# ---------------------------------------------------------------------------
# MCP tool registrations
# ---------------------------------------------------------------------------


@mcp.tool()
async def forge_remember(
    type: str,
    structured: Dict[str, Any],
    agent_id: Optional[str] = None,
) -> str:
    """Store a memory node (decision, pattern, lesson, or preference)."""
    return await _remember_impl(get_db(), type, structured, agent_id)


@mcp.tool()
async def forge_recall(
    query: str,
    type: Optional[str] = None,
    include_historical: bool = False,
    agent_id: Optional[str] = None,
) -> str:
    """Search memory nodes by keyword."""
    return await _recall_impl(
        get_db(), query, type, include_historical,
        agent_id=agent_id,
    )


@mcp.tool()
async def forge_link(
    from_id: str,
    to_id: str,
    edge_type: str,
    from_label: str,
    to_label: str,
    properties: Optional[Dict[str, Any]] = None,
    agent_id: Optional[str] = None,
) -> str:
    """Create an edge between two memory nodes."""
    return await _link_impl(
        get_db(), from_id, to_id, edge_type,
        from_label, to_label, properties, agent_id,
    )


@mcp.tool()
async def forge_patterns(
    domain: Optional[str] = None,
    min_confidence: Optional[float] = None,
    agent_id: Optional[str] = None,
) -> str:
    """Query Pattern nodes, optionally filtered by domain and confidence."""
    return await _patterns_impl(
        get_db(), domain, min_confidence, agent_id=agent_id,
    )


@mcp.tool()
async def forge_forget(
    node_id: str,
    node_label: str,
    reason: str,
    agent_id: Optional[str] = None,
) -> str:
    """Soft-delete a memory node by setting invalid_at timestamp."""
    return await _forget_impl(get_db(), node_id, node_label, reason, agent_id)


@mcp.tool()
async def forge_usage(
    session_id: Optional[str] = None,
    last_n_sessions: Optional[int] = None,
    agent_id: Optional[str] = None,
) -> str:
    """Query token usage statistics for sessions."""
    return await _usage_impl(get_db(), session_id, last_n_sessions, agent_id)


@mcp.tool()
async def forge_decisions(
    code_path: Optional[str] = None,
    symbol: Optional[str] = None,
    agent_id: Optional[str] = None,
) -> str:
    """Query Decision nodes, optionally filtered by code path or symbol."""
    return await _decisions_impl(
        get_db(), code_path, symbol, agent_id=agent_id,
    )


@mcp.tool()
async def forge_timeline(
    node_id: str,
    node_label: str,
    depth: Optional[int] = None,
    agent_id: Optional[str] = None,
) -> str:
    """Follow SUPERSEDES (Decision) or EVOLVED_FROM (Skill) chains."""
    return await _timeline_impl(get_db(), node_id, node_label, depth, agent_id)
