"""PostToolUse hook — check if edited file has AFFECTS edges to active Decisions."""
from typing import Optional
from forge_graph.db import GraphDB


def check_decision_awareness(db: GraphDB, file_path: str) -> Optional[str]:
    """Return warning string if file is governed by active Decisions, else None."""
    try:
        r = db.conn.execute(
            "MATCH (d:Decision)-[:AFFECTS]->(f:File) "
            "WHERE f.file_path = $fp AND d.status = 'active' AND d.invalid_at IS NULL "
            "RETURN d.title AS title",
            parameters={"fp": file_path},
        )
        rows = r.get_as_pl()
        if len(rows) == 0:
            return None
        titles = [rows["title"][i] for i in range(len(rows))]
        return f"This file is governed by: {', '.join(titles)}"
    except Exception:
        # If File or AFFECTS tables don't exist yet, no decisions to flag
        return None


def main() -> None:
    """CLI entry point: python3 -m forge_graph.hooks.post_edit <file_path>"""
    import json
    import os
    import sys

    if len(sys.argv) < 2:
        sys.exit(0)

    file_path = sys.argv[1]
    plugin_data = os.environ.get("CLAUDE_PLUGIN_DATA", "")
    if not plugin_data:
        sys.exit(0)

    db_path = os.path.join(plugin_data, "graph", "forge.lbdb")
    if not os.path.exists(db_path):
        sys.exit(0)

    db = GraphDB(db_path)
    warning = check_decision_awareness(db, file_path)
    db.close()

    if warning:
        print(json.dumps({
            "hookSpecificOutput": {"additionalContext": f"DECISION ALERT: {warning}"}
        }))


if __name__ == "__main__":
    main()
