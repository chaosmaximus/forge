"""Deterministic test: agent .md files must not reference phantom MCP tools.

Since MCP is removed (v0.3.0), agent definitions should reference Bash CLI
commands, not mcp__forge_forge-graph__* tool names.
"""
import re
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent.parent
AGENTS_DIR = REPO_ROOT / "agents"
PLUGIN_JSON = REPO_ROOT / ".claude-plugin" / "plugin.json"


def _get_agent_mcp_refs():
    """Find any remaining mcp__forge_forge-graph__* refs in agent .md files."""
    refs = {}
    pattern = re.compile(r"mcp__forge_forge-graph__(\w+)")
    for md_file in AGENTS_DIR.glob("*.md"):
        content = md_file.read_text()
        tools_found = set(pattern.findall(content))
        if tools_found:
            refs[md_file.name] = tools_found
    return refs


def test_no_mcp_tool_refs_in_agents():
    """Agent .md files must NOT reference forge-graph MCP tools (MCP removed in v0.3.0)."""
    refs = _get_agent_mcp_refs()
    assert not refs, (
        f"Agent files still reference MCP tools (MCP removed):\n"
        + "\n".join(f"  {f}: {t}" for f, t in refs.items())
    )


def test_agents_have_bash_tool():
    """All agents must have Bash in their tools list (for forge-core CLI access)."""
    for md_file in AGENTS_DIR.glob("*.md"):
        content = md_file.read_text()
        # Find tools: line in frontmatter
        if "tools:" in content:
            tools_line = [l for l in content.split("\n") if l.startswith("tools:")][0]
            assert "Bash" in tools_line, f"{md_file.name} missing Bash in tools (needed for forge-core CLI)"


def test_agent_files_exist():
    """All three agent files must exist."""
    for name in ["forge-planner.md", "forge-generator.md", "forge-evaluator.md"]:
        assert (AGENTS_DIR / name).exists(), f"Missing agent file: {name}"


def test_plugin_json_no_mcp_servers():
    """plugin.json mcpServers must be empty (MCP removed)."""
    import json
    pj = json.loads(PLUGIN_JSON.read_text())
    servers = pj.get("mcpServers", {})
    assert servers == {}, f"mcpServers should be empty, got: {servers}"


def test_plugin_json_agents_match_files():
    """plugin.json agents list must match actual agent files."""
    import json
    pj = json.loads(PLUGIN_JSON.read_text())
    registered = {Path(p).name for p in pj.get("agents", [])}
    actual = {f.name for f in AGENTS_DIR.glob("*.md")}
    assert not (actual - registered), f"Agent files not in plugin.json: {actual - registered}"
    assert not (registered - actual), f"plugin.json references missing files: {registered - actual}"
