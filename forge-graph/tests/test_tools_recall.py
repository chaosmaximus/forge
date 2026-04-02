"""Tests for forge_recall tool."""
from __future__ import annotations

import asyncio
import json

import pytest

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema
from forge_graph.memory.tools import _remember_impl, _recall_impl


@pytest.fixture
def db_with_schema(tmp_path):
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def _run(coro):
    return asyncio.get_event_loop().run_until_complete(coro)


class TestForgeRecall:
    def test_recall_returns_matching_decisions(self, db_with_schema):
        """Store 2 decisions, recall by keyword, verify match."""
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Use LadybugDB", "rationale": "Fast graph queries"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Use PostgreSQL", "rationale": "Relational data"},
        ))

        result_json = _run(_recall_impl(
            db_with_schema,
            query="LadybugDB",
        ))
        result = json.loads(result_json)

        assert len(result["results"]) == 1
        assert result["results"][0]["type"] == "decision"
        assert "LadybugDB" in result["results"][0]["title"]
        assert "_meta" in result

    def test_recall_filters_by_type(self, db_with_schema):
        """Store decision + pattern, recall with type='pattern', verify only pattern returns."""
        _run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Serialize writes", "rationale": "Thread safety"},
        ))
        _run(_remember_impl(
            db_with_schema,
            type="pattern",
            structured={"name": "Serialize writes", "description": "Use async lock"},
        ))

        result_json = _run(_recall_impl(
            db_with_schema,
            query="Serialize",
            type="pattern",
        ))
        result = json.loads(result_json)

        assert len(result["results"]) == 1
        assert result["results"][0]["type"] == "pattern"

    def test_recall_returns_empty_for_no_match(self, db_with_schema):
        """Query for nonexistent term returns empty results."""
        result_json = _run(_recall_impl(
            db_with_schema,
            query="xyznonexistent",
        ))
        result = json.loads(result_json)

        assert result["results"] == []
        assert "_meta" in result
