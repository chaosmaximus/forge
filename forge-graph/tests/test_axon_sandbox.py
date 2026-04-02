"""Tests for the axon_cypher sandbox — blocks memory nodes + write operations."""
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
