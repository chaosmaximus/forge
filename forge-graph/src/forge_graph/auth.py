"""Per-agent ACL enforcement for MCP tools."""
from enum import Enum


class AgentRole(Enum):
    LEAD = "lead"
    PLANNER = "planner"
    GENERATOR = "generator"
    EVALUATOR = "evaluator"
    EVOLUTION = "evolution"
    READONLY = "readonly"


_READ_TOOLS = frozenset({
    "forge_recall", "forge_decisions", "forge_patterns", "forge_timeline",
    "forge_usage", "forge_health", "forge_scan",
    "axon_query", "axon_context", "axon_impact", "axon_dead_code",
    "axon_detect_changes", "axon_cypher", "axon_communities", "axon_coupling",
    "axon_call_path", "axon_cycles", "axon_file_context", "axon_test_impact",
    "axon_review_risk", "axon_explain", "axon_list_repos",
})

_WRITE_TOOLS = frozenset({
    "forge_remember", "forge_link", "forge_forget",
})

_ALL_TOOLS = _READ_TOOLS | _WRITE_TOOLS

_ROLE_PERMISSIONS: dict[AgentRole, frozenset[str]] = {
    AgentRole.LEAD: _ALL_TOOLS,
    AgentRole.PLANNER: _READ_TOOLS,
    AgentRole.GENERATOR: _READ_TOOLS,
    AgentRole.EVALUATOR: _READ_TOOLS,
    AgentRole.EVOLUTION: _READ_TOOLS | frozenset({"forge_remember", "forge_link"}),
    AgentRole.READONLY: _READ_TOOLS,
}

_AGENT_ID_TO_ROLE: dict[str, AgentRole] = {
    "forge-planner": AgentRole.PLANNER,
    "forge-generator": AgentRole.GENERATOR,
    "forge-evaluator": AgentRole.EVALUATOR,
    "forge-evolution": AgentRole.EVOLUTION,
}


# Design note: agent_id=None -> LEAD is intentional. The MCP server runs as
# a subprocess of Claude Code in the user's session. The human lead calls tools
# directly (no agent_id). Subagents always set agent_id in their prompts.
# In a multi-user scenario, identity should be bound to MCP session metadata.
def get_role(agent_id: str | None) -> AgentRole:
    if agent_id is None:
        return AgentRole.LEAD
    return _AGENT_ID_TO_ROLE.get(agent_id, AgentRole.READONLY)


def check_access(agent_id: str | None, tool: str) -> bool:
    role = get_role(agent_id)
    allowed = _ROLE_PERMISSIONS.get(role, _READ_TOOLS)
    return tool in allowed
