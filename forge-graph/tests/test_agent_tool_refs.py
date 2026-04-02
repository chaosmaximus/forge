"""Deterministic test: agent .md tool references must match actual MCP server tools.

This test prevents phantom tool references — agent definitions MUST only reference
tools that the forge-graph MCP server actually provides. If a tool is renamed or
removed, this test fails until the agent .md files are updated.
"""
import json
import os
import re
from pathlib import Path

import pytest

# Paths relative to repo root
REPO_ROOT = Path(__file__).parent.parent.parent
AGENTS_DIR = REPO_ROOT / "agents"
PLUGIN_JSON = REPO_ROOT / ".claude-plugin" / "plugin.json"


def _get_server_tools():
    """Get actual tool names from the MCP server (same as runtime)."""
    from forge_graph.server import mcp, init_db
    from forge_graph.memory.schema import create_schema
    import tempfile

    db = init_db(os.path.join(tempfile.mkdtemp(), "test_validate.lbdb"))
    create_schema(db.conn)

    # Import tool modules (this is how main() registers them)
    from forge_graph.memory import tools as _mt  # noqa: F401
    from forge_graph.security import tools as _st  # noqa: F401

    return set(mcp._tool_manager._tools.keys())


def _get_agent_tool_refs():
    """Extract all mcp__forge_forge-graph__* tool references from agent .md files."""
    refs = {}
    pattern = re.compile(r"mcp__forge_forge-graph__(\w+)")

    for md_file in AGENTS_DIR.glob("*.md"):
        content = md_file.read_text()
        tools_found = set(pattern.findall(content))
        if tools_found:
            refs[md_file.name] = tools_found

    return refs


@pytest.fixture(scope="module")
def server_tools():
    return _get_server_tools()


def test_all_agent_tool_refs_exist_in_server(server_tools):
    """Every forge-graph tool referenced in agent .md files must exist in the MCP server."""
    agent_refs = _get_agent_tool_refs()
    errors = []

    for agent_file, tool_names in agent_refs.items():
        for tool_name in tool_names:
            if tool_name not in server_tools:
                errors.append(
                    f"{agent_file} references 'mcp__forge_forge-graph__{tool_name}' "
                    f"but server only has: {sorted(server_tools)}"
                )

    assert not errors, "Phantom tool references found:\n" + "\n".join(errors)


def test_server_has_minimum_tool_set(server_tools):
    """Server must provide the core memory tools."""
    required = {"forge_remember", "forge_recall", "forge_forget", "forge_health"}
    missing = required - server_tools
    assert not missing, f"Server missing required tools: {missing}"


def test_agent_files_exist():
    """All three agent files must exist."""
    for name in ["forge-planner.md", "forge-generator.md", "forge-evaluator.md"]:
        assert (AGENTS_DIR / name).exists(), f"Missing agent file: {name}"


def test_agent_frontmatter_has_tools():
    """Each agent .md must have a tools: line in YAML frontmatter."""
    for md_file in AGENTS_DIR.glob("*.md"):
        content = md_file.read_text()
        # Check frontmatter exists
        assert content.startswith("---"), f"{md_file.name}: missing YAML frontmatter"
        # Find closing ---
        end = content.index("---", 3)
        frontmatter = content[3:end]
        assert "tools:" in frontmatter, f"{md_file.name}: missing 'tools:' in frontmatter"


def test_plugin_json_agents_match_files():
    """plugin.json agents list must match actual agent files."""
    pj = json.loads(PLUGIN_JSON.read_text())
    registered = set()
    for path in pj.get("agents", []):
        # path is like "./agents/forge-planner.md"
        registered.add(Path(path).name)

    actual = {f.name for f in AGENTS_DIR.glob("*.md")}
    missing_in_plugin = actual - registered
    extra_in_plugin = registered - actual

    assert not missing_in_plugin, f"Agent files not in plugin.json: {missing_in_plugin}"
    assert not extra_in_plugin, f"plugin.json references missing files: {extra_in_plugin}"


def test_no_phantom_tool_patterns(server_tools):
    """Ensure known phantom patterns are gone."""
    phantom_names = {
        "get_architecture", "search_graph", "trace_call_path",
        "detect_changes", "get_code_snippet",
    }
    agent_refs = _get_agent_tool_refs()
    for agent_file, tool_names in agent_refs.items():
        found_phantoms = tool_names & phantom_names
        assert not found_phantoms, (
            f"{agent_file} still references phantom tools: {found_phantoms}"
        )
