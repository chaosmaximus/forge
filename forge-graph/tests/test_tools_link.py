"""Tests for forge_link tool."""
from __future__ import annotations

import asyncio
import json

import pytest

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema
from forge_graph.memory.tools import _remember_impl, _link_impl


@pytest.fixture
def db_with_schema(tmp_path):
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def _run(coro):
    return asyncio.get_event_loop().run_until_complete(coro)


class TestForgeLink:
    def test_link_creates_edge(self, db_with_schema):
        """Create 2 decisions, link them with SUPERSEDES, verify via Cypher."""
        r1 = json.loads(_run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "Old approach", "rationale": "Initial"},
        )))
        r2 = json.loads(_run(_remember_impl(
            db_with_schema,
            type="decision",
            structured={"title": "New approach", "rationale": "Better"},
        )))

        link_json = _run(_link_impl(
            db_with_schema,
            from_id=r2["node_id"],
            to_id=r1["node_id"],
            edge_type="SUPERSEDES",
            from_label="Decision",
            to_label="Decision",
        ))
        link_result = json.loads(link_json)
        assert link_result["status"] == "linked"
        assert "_meta" in link_result

        # Verify edge exists
        qr = db_with_schema.conn.execute(
            "MATCH (a:Decision)-[r:SUPERSEDES]->(b:Decision) "
            "WHERE a.id = $from_id AND b.id = $to_id "
            "RETURN count(r)",
            parameters={"from_id": r2["node_id"], "to_id": r1["node_id"]},
        )
        assert qr.has_next()
        assert qr.get_next()[0] == 1

    def test_link_rejects_invalid_edge_type(self, db_with_schema):
        """ValueError for edge_type='DESTROYS'."""
        with pytest.raises(ValueError, match="Invalid edge_type"):
            _run(_link_impl(
                db_with_schema,
                from_id="a",
                to_id="b",
                edge_type="DESTROYS",
                from_label="Decision",
                to_label="Decision",
            ))
