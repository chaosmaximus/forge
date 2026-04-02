"""Pure graph operations — no MCP, no decorators, just logic.

Each function takes a GraphDB connection and returns a dict (JSON-serializable).
Used by forge_graph.cli (the new CLI entry point).
"""
from __future__ import annotations

import json
import math
import uuid
from datetime import datetime, timezone
from typing import Any, Optional

from forge_graph.db import GraphDB


def remember(db: GraphDB, mem_type: str, data: dict) -> dict:
    """Store a memory node (decision, pattern, lesson, preference)."""
    valid_types = {"decision", "pattern", "lesson", "preference"}
    if mem_type not in valid_types:
        return {"error": f"Invalid type '{mem_type}'. Must be one of: {', '.join(sorted(valid_types))}"}

    node_id = data.get("id") or f"{mem_type}-{uuid.uuid4().hex[:12]}"
    now = "current_timestamp()"

    if mem_type == "decision":
        db.conn.execute(
            f"CREATE (n:Decision {{id: $id, title: $title, rationale: $rationale, "
            f"status: $status, confidence: $conf, trust_level: 'user', "
            f"created_at: {now}, updated_at: {now}, valid_at: {now}, accessed_at: {now}}})",
            parameters={
                "id": node_id,
                "title": data.get("title", ""),
                "rationale": data.get("rationale", data.get("content", "")),
                "status": data.get("status", "active"),
                "conf": data.get("confidence", 0.9),
            },
        )
    elif mem_type == "pattern":
        db.conn.execute(
            f"CREATE (n:Pattern {{id: $id, name: $name, description: $desc, "
            f"domain: $domain, confidence: $conf, "
            f"created_at: {now}, updated_at: {now}, valid_at: {now}, accessed_at: {now}}})",
            parameters={
                "id": node_id,
                "name": data.get("title", data.get("name", "")),
                "desc": data.get("description", data.get("content", "")),
                "domain": data.get("domain", "general"),
                "conf": data.get("confidence", 0.5),
            },
        )
    elif mem_type == "lesson":
        db.conn.execute(
            f"CREATE (n:Lesson {{id: $id, insight: $insight, context: $ctx, "
            f"severity: $sev, "
            f"created_at: {now}, updated_at: {now}, valid_at: {now}, accessed_at: {now}}})",
            parameters={
                "id": node_id,
                "insight": data.get("title", data.get("insight", "")),
                "ctx": data.get("content", data.get("context", "")),
                "sev": data.get("severity", "info"),
            },
        )
    elif mem_type == "preference":
        db.conn.execute(
            f"CREATE (n:Preference {{id: $id, key: $key, value: $val, "
            f"scope: 'project', confidence: $conf, "
            f"created_at: {now}, updated_at: {now}, valid_at: {now}, accessed_at: {now}}})",
            parameters={
                "id": node_id,
                "key": data.get("title", data.get("key", "")),
                "val": data.get("content", data.get("value", "")),
                "conf": data.get("confidence", 1.0),
            },
        )

    return {"status": "stored", "id": node_id, "type": mem_type}


def recall(db: GraphDB, query: str, mem_type: Optional[str] = None, include_historical: bool = False) -> dict:
    """Search memory nodes by keyword. Updates accessed_at for returned nodes."""
    labels = []
    if mem_type:
        label_map = {"decision": "Decision", "pattern": "Pattern", "lesson": "Lesson", "preference": "Preference"}
        label = label_map.get(mem_type)
        if label:
            labels = [label]
    if not labels:
        labels = ["Decision", "Pattern", "Lesson", "Preference"]

    results = []
    for label in labels:
        # Build search fields based on label
        if label == "Decision":
            search = "n.title CONTAINS $q OR n.rationale CONTAINS $q"
            fields = "n.id AS id, n.title AS title, n.rationale AS content, n.confidence AS confidence, n.status AS status"
        elif label == "Pattern":
            search = "n.name CONTAINS $q OR n.description CONTAINS $q"
            fields = "n.id AS id, n.name AS title, n.description AS content, n.confidence AS confidence"
        elif label == "Lesson":
            search = "n.insight CONTAINS $q OR n.context CONTAINS $q"
            fields = "n.id AS id, n.insight AS title, n.context AS content"
        else:
            search = "n.key CONTAINS $q OR n.value CONTAINS $q"
            fields = "n.id AS id, n.key AS title, n.value AS content, n.confidence AS confidence"

        where_clause = f"WHERE ({search})"
        if not include_historical:
            where_clause += " AND n.invalid_at IS NULL"

        cypher = f"MATCH (n:{label}) {where_clause} RETURN {fields}, '{label}' AS type LIMIT 20"
        try:
            r = db.conn.execute(cypher, parameters={"q": query})
            rows = r.get_as_pl()
            if len(rows) > 0:
                records = rows.to_dicts()
                results.extend(records)
                # Update accessed_at for returned nodes
                for rec in records:
                    nid = rec.get("id")
                    if nid:
                        try:
                            db.conn.execute(
                                f"MATCH (n:{label} {{id: $id}}) SET n.accessed_at = current_timestamp()",
                                parameters={"id": nid},
                            )
                        except Exception:
                            pass  # Best-effort
        except Exception:
            pass

    return {"results": results, "count": len(results), "query": query}


def forget(db: GraphDB, node_id: str, label: str, reason: str = "") -> dict:
    """Soft-delete a memory node by setting invalid_at."""
    valid_labels = {"Decision", "Pattern", "Lesson", "Preference"}
    if label not in valid_labels:
        return {"error": f"Invalid label '{label}'. Must be one of: {', '.join(sorted(valid_labels))}"}

    try:
        db.conn.execute(
            f"MATCH (n:{label} {{id: $id}}) SET n.invalid_at = current_timestamp()",
            parameters={"id": node_id},
        )
        return {"status": "forgotten", "id": node_id, "label": label}
    except Exception as e:
        return {"error": str(e)}


def health(db: GraphDB) -> dict:
    """Graph health check — node and edge counts by type."""
    counts = {}
    for label in ["Decision", "Pattern", "Lesson", "Preference", "Session", "Skill",
                   "Secret", "File", "Function", "Class", "Method", "AgentRun"]:
        try:
            r = db.conn.execute(f"MATCH (n:{label}) RETURN count(n) AS c")
            counts[label] = int(r.get_next()[0]) if r.has_next() else 0
        except Exception:
            counts[label] = 0

    r = db.conn.execute("MATCH ()-[r]->() RETURN count(r) AS c")
    total_edges = int(r.get_next()[0]) if r.has_next() else 0
    total_nodes = sum(counts.values())

    return {"status": "ok", "nodes": total_nodes, "edges": total_edges, "by_type": counts}


def sync_pending(db: GraphDB, pending_path: str) -> int:
    """Sync entries from pending.jsonl to the graph. Returns count synced."""
    import os
    if not os.path.exists(pending_path):
        return 0

    synced = 0
    remaining = []

    with open(pending_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
                if entry.get("synced"):
                    continue
                mem_type = entry.get("type", "")
                data = {
                    "id": entry.get("id"),
                    "title": entry.get("title", ""),
                    "content": entry.get("content", ""),
                    "confidence": entry.get("confidence", 0.9),
                    "status": entry.get("status", "active"),
                }
                result = remember(db, mem_type, data)
                if "error" not in result:
                    synced += 1
                else:
                    remaining.append(line)
            except (json.JSONDecodeError, Exception):
                remaining.append(line)

    # Rewrite pending with only failed entries
    with open(pending_path, "w") as f:
        for line in remaining:
            f.write(line + "\n")

    return synced
