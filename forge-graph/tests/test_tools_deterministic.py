"""Tests for deterministic tools — forge_patterns, forge_forget, forge_usage, forge_decisions, forge_timeline."""
from __future__ import annotations

import asyncio
import json

import pytest

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema
from forge_graph.memory.tools import (
    _remember_impl,
    _recall_impl,
    _link_impl,
    _patterns_impl,
    _forget_impl,
    _usage_impl,
    _decisions_impl,
    _timeline_impl,
)


@pytest.fixture
def db_with_schema(tmp_path):
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def _run(coro):
    return asyncio.get_event_loop().run_until_complete(coro)


# ---------------------------------------------------------------------------
# forge_patterns
# ---------------------------------------------------------------------------

class TestForgePatterns:
    def test_forge_patterns_filters_by_domain(self, db_with_schema):
        """Create 2 patterns in 'resilience' domain + 1 in 'security', filter by domain."""
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "Retry with backoff", "description": "Exponential backoff", "domain": "resilience"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "Circuit breaker", "description": "Fail fast", "domain": "resilience"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "Input validation", "description": "Sanitize inputs", "domain": "security"},
        ))

        result_json = _run(_patterns_impl(db_with_schema, domain="resilience"))
        result = json.loads(result_json)

        assert len(result["results"]) == 2
        for r in result["results"]:
            assert r["domain"] == "resilience"
        assert result["_meta"]["path"] == "deterministic"
        assert result["_meta"]["llm_calls"] == 0

    def test_forge_patterns_filters_by_confidence(self, db_with_schema):
        """Create patterns with different confidence, filter by min_confidence."""
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "Low conf", "description": "Not sure", "domain": "test", "confidence": 0.3},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "High conf", "description": "Very sure", "domain": "test", "confidence": 0.9},
        ))

        result_json = _run(_patterns_impl(db_with_schema, min_confidence=0.7))
        result = json.loads(result_json)

        assert len(result["results"]) == 1
        assert result["results"][0]["name"] == "High conf"


# ---------------------------------------------------------------------------
# forge_forget
# ---------------------------------------------------------------------------

class TestForgeForget:
    def test_forge_forget_soft_deletes(self, db_with_schema):
        """Create decision, forget it, recall should return empty, historical should find it."""
        r = json.loads(_run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Temp decision", "rationale": "Will be forgotten"},
        )))
        node_id = r["node_id"]

        # Recall before forget — should find it
        recall_before = json.loads(_run(_recall_impl(
            db_with_schema, query="Temp decision", type="decision",
        )))
        assert len(recall_before["results"]) == 1

        # Forget
        forget_json = _run(_forget_impl(
            db_with_schema, node_id=node_id, node_label="Decision", reason="Testing soft delete",
        ))
        forget_result = json.loads(forget_json)
        assert forget_result["status"] == "forgotten"
        assert forget_result["node_id"] == node_id
        assert forget_result["reason"] == "Testing soft delete"
        assert forget_result["_meta"]["path"] == "deterministic"

        # Recall after forget — should be empty (current view)
        recall_after = json.loads(_run(_recall_impl(
            db_with_schema, query="Temp decision", type="decision",
        )))
        assert len(recall_after["results"]) == 0

        # Recall with include_historical — should find it
        recall_hist = json.loads(_run(_recall_impl(
            db_with_schema, query="Temp decision", type="decision",
            include_historical=True,
        )))
        assert len(recall_hist["results"]) == 1

    def test_forge_forget_rejects_invalid_label(self, db_with_schema):
        """ValueError for invalid node_label."""
        with pytest.raises(ValueError, match="Invalid node_label"):
            _run(_forget_impl(
                db_with_schema,
                node_id="x",
                node_label="Session",
                reason="invalid",
            ))

    def test_forge_forget_respects_acl(self, db_with_schema):
        """PermissionError for read-only agent."""
        with pytest.raises(PermissionError, match="does not have access"):
            _run(_forget_impl(
                db_with_schema,
                node_id="x",
                node_label="Decision",
                reason="unauthorized",
                agent_id="forge-generator",
            ))


# ---------------------------------------------------------------------------
# forge_usage
# ---------------------------------------------------------------------------

class TestForgeUsage:
    def test_forge_usage_returns_session_stats(self, db_with_schema):
        """Create a Session node with token counts, query usage, verify numbers."""
        db_with_schema.conn.execute(
            "CREATE (s:Session {"
            "id: 'sess-001', "
            "total_tokens_input: 1000, "
            "total_tokens_output: 500, "
            "total_llm_calls: 5, "
            "total_tool_calls: 10, "
            "deterministic_ratio: 0.8, "
            "started_at: current_timestamp()"
            "})"
        )

        # Query specific session
        result_json = _run(_usage_impl(db_with_schema, session_id="sess-001"))
        result = json.loads(result_json)

        assert result["session_id"] == "sess-001"
        assert result["total_tokens_input"] == 1000
        assert result["total_tokens_output"] == 500
        assert result["total_llm_calls"] == 5
        assert result["total_tool_calls"] == 10
        assert result["deterministic_ratio"] == 0.8
        assert result["_meta"]["path"] == "deterministic"

    def test_forge_usage_aggregate_all_sessions(self, db_with_schema):
        """Create 2 sessions, aggregate usage stats."""
        db_with_schema.conn.execute(
            "CREATE (s:Session {"
            "id: 'sess-a', "
            "total_tokens_input: 100, "
            "total_tokens_output: 50, "
            "total_llm_calls: 2, "
            "total_tool_calls: 4, "
            "deterministic_ratio: 0.6, "
            "started_at: current_timestamp()"
            "})"
        )
        db_with_schema.conn.execute(
            "CREATE (s:Session {"
            "id: 'sess-b', "
            "total_tokens_input: 200, "
            "total_tokens_output: 100, "
            "total_llm_calls: 3, "
            "total_tool_calls: 6, "
            "deterministic_ratio: 1.0, "
            "started_at: current_timestamp()"
            "})"
        )

        result_json = _run(_usage_impl(db_with_schema))
        result = json.loads(result_json)

        assert result["total_tokens_input"] == 300
        assert result["total_tokens_output"] == 150
        assert result["total_llm_calls"] == 5
        assert result["total_tool_calls"] == 10
        assert result["avg_deterministic_ratio"] == pytest.approx(0.8)
        assert result["_meta"]["llm_calls"] == 0

    def test_forge_usage_session_not_found(self, db_with_schema):
        """Non-existent session returns error."""
        result_json = _run(_usage_impl(db_with_schema, session_id="nonexistent"))
        result = json.loads(result_json)
        assert result["error"] == "session not found"


# ---------------------------------------------------------------------------
# forge_decisions
# ---------------------------------------------------------------------------

class TestForgeDecisions:
    def test_forge_decisions_searches_by_path(self, db_with_schema):
        """Create decisions mentioning 'auth', query by code_path, verify match."""
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Auth middleware", "rationale": "Protect auth/login.py endpoint"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "DB connection pool", "rationale": "Performance optimization"},
        ))

        result_json = _run(_decisions_impl(db_with_schema, code_path="auth"))
        result = json.loads(result_json)

        assert len(result["results"]) == 1
        assert "auth" in result["results"][0]["title"].lower() or "auth" in result["results"][0]["rationale"].lower()
        assert result["_meta"]["path"] == "deterministic"
        assert result["_meta"]["llm_calls"] == 0

    def test_forge_decisions_returns_all_when_no_filter(self, db_with_schema):
        """No filter returns all active decisions."""
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Dec A", "rationale": "Reason A"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Dec B", "rationale": "Reason B"},
        ))

        result_json = _run(_decisions_impl(db_with_schema))
        result = json.loads(result_json)
        assert len(result["results"]) == 2


# ---------------------------------------------------------------------------
# forge_timeline
# ---------------------------------------------------------------------------

class TestForgeTimeline:
    def test_forge_timeline_follows_chain(self, db_with_schema):
        """Create 2 decisions, link with SUPERSEDES, query timeline from newer, verify older appears."""
        r_old = json.loads(_run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Original approach", "rationale": "First attempt"},
        )))
        r_new = json.loads(_run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Revised approach", "rationale": "Better design"},
        )))

        # Link newer -> older via SUPERSEDES
        _run(_link_impl(
            db_with_schema,
            from_id=r_new["node_id"],
            to_id=r_old["node_id"],
            edge_type="SUPERSEDES",
            from_label="Decision",
            to_label="Decision",
        ))

        # Query timeline from newer decision
        result_json = _run(_timeline_impl(
            db_with_schema,
            node_id=r_new["node_id"],
            node_label="Decision",
        ))
        result = json.loads(result_json)

        assert len(result["chain"]) >= 1
        chain_ids = [c["id"] for c in result["chain"]]
        assert r_old["node_id"] in chain_ids
        assert result["_meta"]["path"] == "deterministic"
        assert result["_meta"]["llm_calls"] == 0

    def test_forge_timeline_rejects_invalid_label(self, db_with_schema):
        """ValueError for invalid node_label."""
        with pytest.raises(ValueError, match="Invalid node_label"):
            _run(_timeline_impl(
                db_with_schema,
                node_id="x",
                node_label="Pattern",
            ))
