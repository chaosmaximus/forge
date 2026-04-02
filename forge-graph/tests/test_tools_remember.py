"""Tests for forge_remember tool."""
from __future__ import annotations

import asyncio
import json

import pytest

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema
from forge_graph.memory.tools import _remember_impl


@pytest.fixture
def db_with_schema(tmp_path):
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


class TestForgeRemember:
    def test_remember_decision_structured(self, db_with_schema):
        """Create a decision, verify node_id format and _meta.path."""
        result_json = asyncio.get_event_loop().run_until_complete(
            _remember_impl(
                db_with_schema,
                type="decision",
                structured={"title": "Use LadybugDB", "rationale": "Fast graph queries"},
            )
        )
        result = json.loads(result_json)

        assert result["node_id"].startswith("decision-")
        assert len(result["node_id"]) == len("decision-") + 12
        assert result["_meta"]["path"] == "deterministic"

        # Verify node exists in DB
        qr = db_with_schema.conn.execute(
            "MATCH (d:Decision) WHERE d.id = $id RETURN d.title",
            parameters={"id": result["node_id"]},
        )
        assert qr.has_next()
        assert qr.get_next()[0] == "Use LadybugDB"

    def test_remember_pattern_structured(self, db_with_schema):
        """Create a pattern, verify node_id format."""
        result_json = asyncio.get_event_loop().run_until_complete(
            _remember_impl(
                db_with_schema,
                type="pattern",
                structured={"name": "Write lock", "description": "Serialize DB writes"},
            )
        )
        result = json.loads(result_json)

        assert result["node_id"].startswith("pattern-")
        assert len(result["node_id"]) == len("pattern-") + 12

        # Verify node exists in DB
        qr = db_with_schema.conn.execute(
            "MATCH (p:Pattern) WHERE p.id = $id RETURN p.name",
            parameters={"id": result["node_id"]},
        )
        assert qr.has_next()
        assert qr.get_next()[0] == "Write lock"

    def test_remember_rejects_unknown_type(self, db_with_schema):
        """ValueError for type='unknown'."""
        with pytest.raises(ValueError, match="Unknown type"):
            asyncio.get_event_loop().run_until_complete(
                _remember_impl(
                    db_with_schema,
                    type="unknown",
                    structured={"title": "nope"},
                )
            )

    def test_remember_respects_acl(self, db_with_schema):
        """PermissionError for agent_id='forge-generator'."""
        with pytest.raises(PermissionError, match="does not have access"):
            asyncio.get_event_loop().run_until_complete(
                _remember_impl(
                    db_with_schema,
                    type="decision",
                    structured={"title": "Should fail"},
                    agent_id="forge-generator",
                )
            )
