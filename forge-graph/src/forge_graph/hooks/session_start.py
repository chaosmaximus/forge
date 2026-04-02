"""SessionStart hook — create Session node, load context from graph."""
import json
import os
import uuid
from datetime import datetime, timezone

from forge_graph.db import GraphDB


def run_session_start(db: GraphDB) -> str:
    """Create Session node, query context, return hookSpecificOutput JSON."""
    session_id = f"session-{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:8]}"

    db.conn.execute(
        "CREATE (s:Session {id: $sid, started_at: current_timestamp(), mode: 'unknown', "
        "project: $proj, total_tokens_input: 0, total_tokens_output: 0, "
        "total_llm_calls: 0, total_tool_calls: 0, deterministic_ratio: 1.0})",
        parameters={"sid": session_id, "proj": os.getcwd()},
    )

    # Active decisions (max 10, user trust only)
    r = db.conn.execute(
        "MATCH (d:Decision) WHERE d.status = 'active' AND d.invalid_at IS NULL "
        "RETURN d.title AS title, d.rationale AS rationale LIMIT 10"
    )
    decisions = []
    rows = r.get_as_pl()
    for i in range(len(rows)):
        decisions.append(f"- {rows['title'][i]}: {rows['rationale'][i]}")

    # Recent lessons (max 5)
    r2 = db.conn.execute(
        "MATCH (l:Lesson) WHERE l.invalid_at IS NULL "
        "RETURN l.insight AS insight LIMIT 5"
    )
    lessons = []
    rows2 = r2.get_as_pl()
    for i in range(len(rows2)):
        lessons.append(f"- {rows2['insight'][i]}")

    parts = [f"[Forge v0.2.0] Session {session_id}."]
    if decisions:
        parts.append(f"Active decisions ({len(decisions)}):")
        parts.extend(decisions)
    if lessons:
        parts.append(f"Recent lessons ({len(lessons)}):")
        parts.extend(lessons)
    parts.append(
        "Tools: forge_remember, forge_recall, forge_link, forge_decisions, "
        "forge_patterns, forge_timeline, forge_forget, forge_usage, forge_scan."
    )

    return json.dumps({"hookSpecificOutput": {"additionalContext": "\n".join(parts)}})


def main() -> None:
    """CLI entry point: python3 -m forge_graph.hooks.session_start"""
    import sys
    plugin_data = os.environ.get("CLAUDE_PLUGIN_DATA", "")
    if not plugin_data:
        print('{"hookSpecificOutput":{"additionalContext":"[Forge v0.2.0] No plugin data dir."}}')
        sys.exit(0)

    db_path = os.path.join(plugin_data, "graph", "forge.lbdb")
    if not os.path.exists(os.path.dirname(db_path)):
        os.makedirs(os.path.dirname(db_path), exist_ok=True)

    db = GraphDB(db_path)
    from forge_graph.memory.schema import create_schema
    create_schema(db.conn)

    print(run_session_start(db))
    db.close()


if __name__ == "__main__":
    main()
