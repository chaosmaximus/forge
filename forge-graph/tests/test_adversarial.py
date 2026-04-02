"""Adversarial tests — release blockers per spec §10.7."""
from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
import pytest

from forge_graph.db import GraphDB
from forge_graph.memory.schema import create_schema


def _run(coro):
    return asyncio.get_event_loop().run_until_complete(coro)


@pytest.fixture
def db_with_schema(tmp_path: Path):
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def test_cypher_injection_via_recall(db_with_schema):
    """Attempt Cypher injection via forge_recall query param."""
    from forge_graph.memory.tools import _recall_impl
    malicious = [
        "'; DROP TABLE Decision; --",
        "' OR 1=1 --",
        "}) RETURN d; MATCH (x:Secret",
        "UNION MATCH (s:Secret) RETURN s.fingerprint AS title",
    ]
    for q in malicious:
        result = _run(_recall_impl(db_with_schema, query=q))
        data = json.loads(result)
        assert "results" in data  # Should return empty, not crash or leak


def test_axon_cypher_memory_bypass(db_with_schema):
    """Attempt to read memory nodes via axon_cypher sandbox."""
    from forge_graph.axon_proxy import validate_cypher_query
    bypasses = [
        "MATCH (d:Decision) RETURN d",
        "MATCH (s:Secret) RETURN s.fingerprint",
        "MATCH (p:Preference) RETURN p.value",
        "MATCH (s:Session) RETURN s.summary",
        "MATCH (n:_forge_meta) RETURN n",
    ]
    for q in bypasses:
        assert validate_cypher_query(q) is False, f"Should block: {q}"


def test_hud_symlink_handling(tmp_path: Path):
    """HUD writer should handle symlinks safely via atomic rename."""
    from forge_graph.hud.state import HudStateWriter
    # Create a symlink
    target = tmp_path / "evil-target.json"
    target.write_text("{}")
    link = tmp_path / "hud-state.json"
    link.symlink_to(target)

    writer = HudStateWriter(link)
    writer.update(graph={"nodes": 1, "edges": 1})
    writer.flush()
    # Atomic rename replaces the symlink with a real file
    assert not link.is_symlink(), "Symlink should be replaced by atomic rename"


def test_evolution_path_traversal():
    """Evolution must not write outside skills/ directory."""
    from forge_graph.evolution.safety import validate_evolution_path
    # Allowed
    assert validate_evolution_path("skills/forge-feature/SKILL.md") is True
    assert validate_evolution_path("skills/forge-new/templates/prd.md") is True
    # Blocked
    assert validate_evolution_path("hooks/hooks.json") is False
    assert validate_evolution_path("../../../etc/passwd") is False
    assert validate_evolution_path(".claude-plugin/plugin.json") is False
    assert validate_evolution_path("scripts/session-start.sh") is False
    assert validate_evolution_path("agents/forge-planner.md") is False


def test_oversized_evolution_diff():
    """Reject diffs larger than 500 lines."""
    from forge_graph.evolution.safety import validate_diff_size
    small = "- old\n+ new\n" * 10
    assert validate_diff_size(small) is True
    large = "- old\n+ new\n" * 600
    assert validate_diff_size(large) is False


def test_think_block_sanitization():
    """Strip <think> blocks from LLM output."""
    from forge_graph.evolution.safety import sanitize_llm_output
    output = "<think>reasoning...</think>\n---\nname: test\n---\nContent"
    clean = sanitize_llm_output(output)
    assert "<think>" not in clean
    assert "Content" in clean
    assert "name: test" in clean


def test_nested_think_blocks():
    """Handle nested <think> blocks."""
    from forge_graph.evolution.safety import sanitize_llm_output
    output = "<think>outer<think>inner</think>still</think>\nReal content"
    clean = sanitize_llm_output(output)
    assert "<think>" not in clean
    assert "Real content" in clean


def test_remember_acl_blocks_generator(db_with_schema):
    """Generator agent cannot write memory."""
    from forge_graph.memory.tools import _remember_impl
    with pytest.raises(PermissionError):
        _run(_remember_impl(db=db_with_schema, type="decision",
                            structured={"title": "evil", "rationale": "injection"},
                            agent_id="forge-generator"))
