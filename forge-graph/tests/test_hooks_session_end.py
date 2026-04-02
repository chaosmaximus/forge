"""Test session_end hook — closes Session node, computes metrics."""
import json
import pytest
from pathlib import Path


@pytest.fixture
def db_with_session(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema

    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)

    db.conn.execute(
        "CREATE (s:Session {id: 'sess-test', started_at: '2026-04-02T10:00:00Z', "
        "mode: 'feature', project: '/test', "
        "total_tokens_input: 1000, total_tokens_output: 500, "
        "total_llm_calls: 2, total_tool_calls: 15, deterministic_ratio: 0.87})"
    )
    yield db, tmp_path
    db.close()


def test_session_end_closes_session(db_with_session):
    from forge_graph.hooks.session_end import run_session_end

    db, tmp_path = db_with_session
    result = run_session_end(db, "sess-test", str(tmp_path))
    data = json.loads(result)

    assert "hookSpecificOutput" in data

    r = db.conn.execute(
        "MATCH (s:Session {id: 'sess-test'}) RETURN s.ended_at AS ended"
    )
    rows = r.get_as_pl()
    assert rows["ended"][0] is not None


def test_session_end_writes_hud_state(db_with_session):
    from forge_graph.hooks.session_end import run_session_end

    db, tmp_path = db_with_session
    hud_dir = tmp_path / "hud"
    hud_dir.mkdir()
    run_session_end(db, "sess-test", str(tmp_path))

    hud_path = hud_dir / "hud-state.json"
    assert hud_path.exists()
    state = json.loads(hud_path.read_text())
    assert state["session"]["phase"] == "ended"
