"""Evolution engine — deterministic orchestration, zero LLM.

Called at SessionStart to surface evolution suggestions to the user.
"""
from __future__ import annotations

from forge_graph.db import GraphDB
from forge_graph.evolution.metrics import check_evolution_candidates

_TYPE_LABELS = {
    "fix": "AUTO-FIX",
    "improve": "AUTO-IMPROVE",
}


def check_for_suggestions(db: GraphDB) -> list[dict]:
    """Return evolution suggestions for the user.

    Pure deterministic — no LLM calls. Scans active skills for
    metric-based evolution triggers.
    """
    candidates = check_evolution_candidates(db)

    suggestions: list[dict] = []
    for c in candidates:
        suggestions.append({
            "skill": c["skill_name"],
            "type": c["evolution_type"],
            "reason": c["reason"],
            "metrics": c["metrics"],
        })

    return suggestions


def format_suggestions_for_user(suggestions: list[dict]) -> str:
    """Format evolution suggestions as lettered options.

    Returns empty string if no suggestions.
    """
    if not suggestions:
        return ""

    lines: list[str] = ["Skill evolution suggestions:"]
    for i, s in enumerate(suggestions):
        letter = chr(ord("a") + i)
        label = _TYPE_LABELS.get(s["type"], s["type"].upper())
        lines.append(f"  ({letter}) {label}: {s['skill']} — {s['reason']}")

    return "\n".join(lines)
