"""Test session_start hook — creates Session node, loads context."""
import json
import pytest
from pathlib import Path


@pytest.fixture
def db_with_context(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema

    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)

    # Seed a Decision and a Lesson
    db.conn.execute(
        "CREATE (d:Decision {id: 'dec-1', title: 'Use JWT', "
        "rationale: 'Stateless auth', status: 'active', confidence: 0.9, "
        "trust_level: 'user', created_at: current_timestamp(), updated_at: current_timestamp()})"
    )
    db.conn.execute(
        "CREATE (l:Lesson {id: 'les-1', insight: 'Always validate tokens', "
        "context: 'auth module', severity: 'warning', "
        "created_at: current_timestamp(), updated_at: current_timestamp()})"
    )
    yield db
    db.close()


def test_session_start_creates_session_node(db_with_context):
    from forge_graph.hooks.session_start import run_session_start

    result = run_session_start(db_with_context)
    data = json.loads(result)

    assert "hookSpecificOutput" in data
    ctx = data["hookSpecificOutput"]["additionalContext"]
    assert "Forge v0.2.0" in ctx

    r = db_with_context.conn.execute("MATCH (s:Session) RETURN s.id AS id")
    rows = r.get_as_pl()
    assert len(rows) == 1
    assert rows["id"][0].startswith("session-")


def test_session_start_includes_active_decisions(db_with_context):
    from forge_graph.hooks.session_start import run_session_start

    result = run_session_start(db_with_context)
    ctx = json.loads(result)["hookSpecificOutput"]["additionalContext"]
    assert "Use JWT" in ctx


def test_session_start_includes_recent_lessons(db_with_context):
    from forge_graph.hooks.session_start import run_session_start

    result = run_session_start(db_with_context)
    ctx = json.loads(result)["hookSpecificOutput"]["additionalContext"]
    assert "Always validate tokens" in ctx
