"""Tests for the axon_cypher sandbox — blocks memory nodes + write operations."""
import json
import pytest

from forge_graph.axon_proxy import validate_cypher_query


# ── Read-only code-node queries (should be allowed) ──────────────────────────

def test_allows_code_node_read():
    assert validate_cypher_query("MATCH (f:Function) RETURN f.name") is True


def test_allows_class_query():
    assert validate_cypher_query(
        "MATCH (c:Class)-[:EXTENDS]->(p:Class) RETURN c, p"
    ) is True


def test_allows_file_query():
    assert validate_cypher_query(
        "MATCH (f:File) WHERE f.file_path CONTAINS 'auth' RETURN f"
    ) is True


# ── Memory-node reads (should be blocked) ────────────────────────────────────

def test_blocks_decision_read():
    assert validate_cypher_query("MATCH (d:Decision) RETURN d") is False


def test_blocks_pattern_read():
    assert validate_cypher_query("MATCH (p:Pattern) RETURN p.name") is False


def test_blocks_session_read():
    assert validate_cypher_query("MATCH (s:Session) RETURN s") is False


def test_blocks_secret_read():
    assert validate_cypher_query("MATCH (s:Secret) RETURN s.fingerprint") is False


def test_blocks_skill_read():
    assert validate_cypher_query("MATCH (s:Skill) RETURN s") is False


def test_blocks_preference_read():
    assert validate_cypher_query("MATCH (p:Preference) RETURN p.value") is False


def test_blocks_lesson_read():
    assert validate_cypher_query("MATCH (l:Lesson) RETURN l") is False


def test_blocks_forge_meta():
    assert validate_cypher_query("MATCH (m:_forge_meta) RETURN m") is False


# ── Write operations (should be blocked) ─────────────────────────────────────

def test_blocks_create():
    assert validate_cypher_query("CREATE (f:Function {name: 'evil'})") is False


def test_blocks_set():
    assert validate_cypher_query("MATCH (f:Function) SET f.name = 'evil'") is False


def test_blocks_delete():
    assert validate_cypher_query("MATCH (f:Function) DELETE f") is False


def test_blocks_merge():
    assert validate_cypher_query("MERGE (f:Function {name: 'evil'})") is False


def test_blocks_remove():
    assert validate_cypher_query("MATCH (f:Function) REMOVE f.name") is False


# ── Case insensitivity ───────────────────────────────────────────────────────

def test_blocks_case_insensitive():
    assert validate_cypher_query("match (d:decision) return d") is False
    assert validate_cypher_query("MATCH (d:DECISION) RETURN d") is False


# ── Integration tests for Cypher query via CLI ────────────────────────────

@pytest.fixture
def graph_db_with_schema(tmp_path):
    """Initialize a GraphDB with full schema."""
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema

    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def test_cypher_returns_results(graph_db_with_schema):
    """Valid code queries return results."""
    graph_db_with_schema.conn.execute(
        "CREATE (f:File {id: 'f1', file_path: 'test.py', name: 'test.py'})"
    )
    result = graph_db_with_schema.conn.execute("MATCH (f:File) RETURN f.name AS name")
    rows = result.get_as_pl()
    assert len(rows) == 1
    assert rows["name"][0] == "test.py"


def test_cypher_sandbox_blocks_memory_query(graph_db_with_schema):
    """Cypher sandbox rejects queries accessing memory nodes."""
    assert not validate_cypher_query("MATCH (d:Decision) RETURN d")
    assert not validate_cypher_query("MATCH (a:AgentRun) RETURN a.summary")
