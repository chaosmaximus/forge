"""Tests for per-agent ACL enforcement."""
from forge_graph.auth import check_access, get_role, AgentRole


def test_lead_has_full_access():
    """None agent_id (lead) can call both write and read tools."""
    assert check_access(None, "forge_remember") is True
    assert check_access(None, "forge_scan") is True
    assert check_access(None, "axon_cypher") is True


def test_planner_is_read_only():
    """forge-planner can read but cannot write."""
    assert check_access("forge-planner", "forge_recall") is True
    assert check_access("forge-planner", "forge_decisions") is True
    assert check_access("forge-planner", "forge_remember") is False
    assert check_access("forge-planner", "forge_link") is False


def test_generator_is_read_only():
    """forge-generator can read but cannot write."""
    assert check_access("forge-generator", "forge_recall") is True
    assert check_access("forge-generator", "forge_remember") is False


def test_evaluator_is_read_only():
    """forge-evaluator can read but cannot write."""
    assert check_access("forge-evaluator", "forge_recall") is True
    assert check_access("forge-evaluator", "forge_remember") is False


def test_unknown_agent_defaults_read_only():
    """An unknown agent_id falls back to READONLY role."""
    assert get_role("unknown-agent") == AgentRole.READONLY
    assert check_access("unknown-agent", "forge_recall") is True
    assert check_access("unknown-agent", "forge_remember") is False
    assert check_access("unknown-agent", "forge_forget") is False


def test_no_agent_id_treated_as_lead():
    """None agent_id is treated as LEAD with full access."""
    assert get_role(None) == AgentRole.LEAD
    assert check_access(None, "forge_remember") is True
    assert check_access(None, "forge_link") is True
    assert check_access(None, "forge_forget") is True
    assert check_access(None, "forge_recall") is True
