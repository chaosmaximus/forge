"""SessionEnd hook — close Session node, update HUD state."""
import json
import os

from forge_graph.db import GraphDB
from forge_graph.hud.state import HudStateWriter


def run_session_end(db: GraphDB, session_id: str, plugin_data_dir: str) -> str:
    """Close Session, update HUD, return hookSpecificOutput JSON."""
    db.conn.execute(
        "MATCH (s:Session {id: $sid}) SET s.ended_at = current_timestamp()",
        parameters={"sid": session_id},
    )

    # Count active memory nodes — Secret uses status instead of invalid_at
    _COUNT_QUERIES = {
        "decision": "MATCH (n:Decision) WHERE n.invalid_at IS NULL RETURN count(n) AS c",
        "pattern": "MATCH (n:Pattern) WHERE n.invalid_at IS NULL RETURN count(n) AS c",
        "lesson": "MATCH (n:Lesson) WHERE n.invalid_at IS NULL RETURN count(n) AS c",
        "secret": "MATCH (n:Secret) WHERE n.status = 'active' RETURN count(n) AS c",
    }
    counts = {}
    for label, query in _COUNT_QUERIES.items():
        r = db.conn.execute(query)
        rows = r.get_as_pl()
        counts[label] = int(rows["c"][0]) if len(rows) > 0 else 0

    hud_path = os.path.join(plugin_data_dir, "hud", "hud-state.json")
    hud = HudStateWriter(hud_path)
    hud.update(
        session={"phase": "ended"},
        memory={
            "decisions": counts.get("decision", 0),
            "patterns": counts.get("pattern", 0),
            "lessons": counts.get("lesson", 0),
            "secrets": counts.get("secret", 0),
        },
    )
    hud.flush()

    return json.dumps({
        "hookSpecificOutput": {
            "additionalContext": "Session ended. Memory saved to graph."
        }
    })


def main() -> None:
    """CLI entry point: python3 -m forge_graph.hooks.session_end"""
    import sys
    plugin_data = os.environ.get("CLAUDE_PLUGIN_DATA", "")
    if not plugin_data:
        print('{"hookSpecificOutput":{"additionalContext":"Session ended."}}')
        sys.exit(0)

    db_path = os.path.join(plugin_data, "graph", "forge.lbdb")
    if not os.path.exists(db_path):
        print('{"hookSpecificOutput":{"additionalContext":"Session ended. No graph."}}')
        sys.exit(0)

    db = GraphDB(db_path)

    r = db.conn.execute(
        "MATCH (s:Session) WHERE s.ended_at IS NULL "
        "RETURN s.id AS id ORDER BY s.started_at DESC LIMIT 1"
    )
    rows = r.get_as_pl()
    session_id = rows["id"][0] if len(rows) > 0 else None

    if session_id:
        print(run_session_end(db, session_id, plugin_data))
    else:
        print('{"hookSpecificOutput":{"additionalContext":"Session ended. No open session found."}}')

    db.close()


if __name__ == "__main__":
    main()
