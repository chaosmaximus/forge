"""Integration tests — cross-tool workflows."""
from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest


def _run(coro):
    """Run an async coroutine synchronously."""
    return asyncio.get_event_loop().run_until_complete(coro)


@pytest.fixture
def db_with_schema(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def test_full_decision_lifecycle(db_with_schema):
    """remember → recall → link → decisions → forget → recall-historical"""
    from forge_graph.memory.tools import (
        _remember_impl, _recall_impl, _link_impl,
        _decisions_impl, _forget_impl,
    )
    db = db_with_schema

    # 1. Remember a decision
    r1 = json.loads(_run(_remember_impl(db, "decision", structured={
        "title": "Use JWT for auth",
        "rationale": "Stateless, scalable across microservices",
    })))
    decision_id = r1["node_id"]
    assert decision_id.startswith("decision-")

    # 2. Remember a lesson
    r2 = json.loads(_run(_remember_impl(db, "lesson", structured={
        "insight": "OAuth was too complex for our team size",
        "context": "Auth sprint week 3",
        "severity": "warning",
    })))
    lesson_id = r2["node_id"]

    # 3. Recall the decision
    recall = json.loads(_run(_recall_impl(db, query="JWT")))
    assert len(recall["results"]) >= 1
    assert any(r["title"] == "Use JWT for auth" for r in recall["results"])

    # 4. Link decision to lesson (MOTIVATED_BY)
    link = json.loads(_run(_link_impl(db, from_id=decision_id, to_id=lesson_id,
                                       edge_type="MOTIVATED_BY",
                                       from_label="Decision", to_label="Lesson")))
    assert link["status"] == "linked"

    # 5. Query decisions by path
    decisions = json.loads(_run(_decisions_impl(db, code_path="auth")))
    assert len(decisions["results"]) >= 1

    # 6. Forget the decision
    forget = json.loads(_run(_forget_impl(db, node_id=decision_id,
                                           node_label="Decision", reason="superseded")))
    assert forget["status"] == "forgotten"

    # 7. Recall should NOT find it (current view)
    recall2 = json.loads(_run(_recall_impl(db, query="JWT")))
    assert len(recall2["results"]) == 0

    # 8. Recall with historical should find it
    recall3 = json.loads(_run(_recall_impl(db, query="JWT", include_historical=True)))
    assert len(recall3["results"]) >= 1


def test_full_pattern_lifecycle(db_with_schema):
    """remember pattern → recall → patterns filter → forget"""
    from forge_graph.memory.tools import (
        _remember_impl, _recall_impl, _patterns_impl, _forget_impl,
    )
    db = db_with_schema

    # Create patterns in different domains
    r1 = json.loads(_run(_remember_impl(db, "pattern", structured={
        "name": "Circuit breaker", "description": "Fail fast after N retries",
        "domain": "resilience", "confidence": 0.9,
    })))
    r2 = json.loads(_run(_remember_impl(db, "pattern", structured={
        "name": "Input validation", "description": "Validate at system boundary",
        "domain": "security", "confidence": 0.95,
    })))

    # Filter by domain
    resilience = json.loads(_run(_patterns_impl(db, domain="resilience")))
    assert len(resilience["results"]) == 1
    assert resilience["results"][0]["name"] == "Circuit breaker"

    security = json.loads(_run(_patterns_impl(db, domain="security")))
    assert len(security["results"]) == 1

    # Filter by confidence
    high_conf = json.loads(_run(_patterns_impl(db, min_confidence=0.92)))
    assert len(high_conf["results"]) == 1
    assert high_conf["results"][0]["name"] == "Input validation"

    # Forget and verify
    _run(_forget_impl(db, node_id=r1["node_id"], node_label="Pattern", reason="outdated"))
    all_patterns = json.loads(_run(_patterns_impl(db)))
    assert len(all_patterns["results"]) == 1  # Only security pattern remains


def test_session_token_tracking(db_with_schema):
    """Create sessions → usage aggregation"""
    from forge_graph.memory.tools import _usage_impl
    db = db_with_schema

    # Create 3 sessions with varying token counts
    for i, (ti, to, llm, tools, dr) in enumerate([
        (1000, 500, 2, 20, 0.9),
        (2000, 1000, 5, 30, 0.8),
        (500, 200, 1, 10, 0.95),
    ]):
        db.conn.execute(
            "CREATE (s:Session {id: $id, started_at: current_timestamp(), mode: 'feature', "
            "project: 'test', total_tokens_input: $ti, total_tokens_output: $to, "
            "total_llm_calls: $llm, total_tool_calls: $tools, deterministic_ratio: $dr})",
            parameters={"id": f"sess-{i}", "ti": ti, "to": to, "llm": llm, "tools": tools, "dr": dr},
        )

    # Aggregate all
    usage = json.loads(_run(_usage_impl(db)))
    assert usage["total_tokens_input"] == 3500
    assert usage["total_tokens_output"] == 1700
    assert usage["total_llm_calls"] == 8

    # Single session
    usage1 = json.loads(_run(_usage_impl(db, session_id="sess-0")))
    assert usage1["total_tokens_input"] == 1000


def test_meta_always_present(db_with_schema):
    """Every tool response must include _meta with required fields."""
    from forge_graph.memory.tools import (
        _remember_impl, _recall_impl, _patterns_impl,
        _forget_impl, _usage_impl, _decisions_impl,
    )
    db = db_with_schema

    responses = [
        _run(_remember_impl(db, "decision", structured={"title": "test", "rationale": "test"})),
        _run(_recall_impl(db, query="test")),
        _run(_patterns_impl(db)),
        _run(_decisions_impl(db)),
        _run(_usage_impl(db)),
    ]
    for resp_str in responses:
        data = json.loads(resp_str)
        assert "_meta" in data, f"Missing _meta in: {resp_str[:100]}"
        meta = data["_meta"]
        assert "duration_ms" in meta
        assert "path" in meta
        assert meta["path"] in ("deterministic", "agent")
        assert "llm_calls" in meta
        assert "tokens_input" in meta


def test_acl_enforced_across_all_write_tools(db_with_schema):
    """All write tools reject unauthorized agents."""
    from forge_graph.memory.tools import _remember_impl, _link_impl, _forget_impl

    db = db_with_schema
    readonly_agents = ["forge-planner", "forge-generator", "forge-evaluator", "unknown-agent"]

    for agent in readonly_agents:
        with pytest.raises(PermissionError):
            _run(_remember_impl(db, "decision", structured={"title": "x", "rationale": "x"}, agent_id=agent))

        with pytest.raises(PermissionError):
            _run(_forget_impl(db, node_id="fake", node_label="Decision", reason="test", agent_id=agent))
