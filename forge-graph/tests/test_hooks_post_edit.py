"""Test post_edit hook — decision-awareness check."""
import json
import pytest
from pathlib import Path


@pytest.fixture
def db_with_decision_link(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema

    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)

    db.conn.execute(
        "CREATE (d:Decision {id: 'dec-jwt', title: 'Use JWT for auth', "
        "rationale: 'Stateless', status: 'active', confidence: 0.95, "
        "trust_level: 'user', created_at: current_timestamp(), updated_at: current_timestamp()})"
    )
    # Create a File node and AFFECTS edge
    # NOTE: The File table must exist in schema. It may or may not exist yet.
    # If it doesn't, you'll need to create it. Check schema.py first.
    # If File table doesn't exist, create it inline in this test fixture.
    try:
        db.conn.execute(
            "CREATE (f:File {id: 'file-auth', file_path: 'src/auth/middleware.py', name: 'middleware.py'})"
        )
    except Exception:
        # File table might not exist yet — create it
        db.conn.execute(
            "CREATE NODE TABLE IF NOT EXISTS File (id STRING, file_path STRING, name STRING, "
            "language STRING DEFAULT 'unknown', size_bytes INT64 DEFAULT 0, indexed_at STRING, PRIMARY KEY (id))"
        )
        db.conn.execute(
            "CREATE (f:File {id: 'file-auth', file_path: 'src/auth/middleware.py', name: 'middleware.py'})"
        )

    # AFFECTS edge: Decision → File
    try:
        db.conn.execute(
            "MATCH (d:Decision {id: 'dec-jwt'}), (f:File {id: 'file-auth'}) "
            "CREATE (d)-[:AFFECTS {impact: 'high'}]->(f)"
        )
    except Exception:
        # AFFECTS rel table might not exist — create it
        db.conn.execute(
            "CREATE REL TABLE IF NOT EXISTS AFFECTS (FROM Decision TO File, impact STRING DEFAULT 'medium')"
        )
        db.conn.execute(
            "MATCH (d:Decision {id: 'dec-jwt'}), (f:File {id: 'file-auth'}) "
            "CREATE (d)-[:AFFECTS {impact: 'high'}]->(f)"
        )

    yield db
    db.close()


def test_post_edit_flags_affected_file(db_with_decision_link):
    from forge_graph.hooks.post_edit import check_decision_awareness

    result = check_decision_awareness(db_with_decision_link, "src/auth/middleware.py")
    assert result is not None
    assert "Use JWT for auth" in result


def test_post_edit_returns_none_for_unaffected_file(db_with_decision_link):
    from forge_graph.hooks.post_edit import check_decision_awareness

    result = check_decision_awareness(db_with_decision_link, "src/utils/helpers.py")
    assert result is None
