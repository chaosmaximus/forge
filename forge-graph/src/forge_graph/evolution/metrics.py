"""Skill metric aggregation from the knowledge graph."""
from __future__ import annotations

from forge_graph.db import GraphDB

MIN_SUPPORT = 5  # Minimum selections before metrics trigger evolution


def _safe_div(numerator: float, denominator: float) -> float:
    """Division that returns 0.0 on zero denominator."""
    if denominator == 0:
        return 0.0
    return numerator / denominator


def compute_skill_metrics(db: GraphDB, skill_id: str) -> dict:
    """Compute evolution metrics for a single skill.

    Returns dict with applied_rate, completion_rate, effective_rate,
    fallback_rate, and cold_start flag.
    """
    result = db.conn.execute(
        "MATCH (s:Skill {id: $sid}) "
        "RETURN s.total_selections, s.total_applied, "
        "s.total_completions, s.total_fallbacks",
        parameters={"sid": skill_id},
    )

    if not result.has_next():
        return {
            "total_selections": 0,
            "total_applied": 0,
            "total_completions": 0,
            "total_fallbacks": 0,
            "applied_rate": 0.0,
            "completion_rate": 0.0,
            "effective_rate": 0.0,
            "fallback_rate": 0.0,
            "cold_start": True,
        }

    row = result.get_next()
    total_selections = row[0] or 0
    total_applied = row[1] or 0
    total_completions = row[2] or 0
    total_fallbacks = row[3] or 0

    cold_start = total_selections < MIN_SUPPORT

    return {
        "total_selections": total_selections,
        "total_applied": total_applied,
        "total_completions": total_completions,
        "total_fallbacks": total_fallbacks,
        "applied_rate": _safe_div(total_applied, total_selections),
        "completion_rate": _safe_div(total_completions, total_applied),
        "effective_rate": _safe_div(total_completions, total_selections),
        "fallback_rate": _safe_div(total_fallbacks, total_applied),
        "cold_start": cold_start,
    }


def check_evolution_candidates(db: GraphDB) -> list[dict]:
    """Find active skills that need evolution.

    Triggers:
    - AUTO-FIX: fallback_rate > 0.40
    - AUTO-IMPROVE: effective_rate < 0.55 AND applied_rate > 0.25
    """
    result = db.conn.execute(
        "MATCH (s:Skill) WHERE s.is_active = true RETURN s.id, s.name"
    )

    candidates: list[dict] = []
    skills: list[tuple[str, str]] = []
    while result.has_next():
        row = result.get_next()
        skills.append((row[0], row[1]))

    for skill_id, skill_name in skills:
        metrics = compute_skill_metrics(db, skill_id)

        if metrics["cold_start"]:
            continue

        # AUTO-FIX: high fallback rate
        if metrics["fallback_rate"] > 0.40:
            candidates.append({
                "skill_id": skill_id,
                "skill_name": skill_name,
                "evolution_type": "fix",
                "reason": f"fallback_rate={metrics['fallback_rate']:.2f} > 0.40",
                "metrics": metrics,
            })
        # AUTO-IMPROVE: low effectiveness but decent selection-to-apply
        elif metrics["effective_rate"] < 0.55 and metrics["applied_rate"] > 0.25:
            candidates.append({
                "skill_id": skill_id,
                "skill_name": skill_name,
                "evolution_type": "improve",
                "reason": (
                    f"effective_rate={metrics['effective_rate']:.2f} < 0.55 "
                    f"AND applied_rate={metrics['applied_rate']:.2f} > 0.25"
                ),
                "metrics": metrics,
            })

    return candidates
