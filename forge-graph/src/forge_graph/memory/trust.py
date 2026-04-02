"""Trust level classification and content sanitization."""
import re

_DANGEROUS_PATTERNS = [
    re.compile(r"<tool_use>.*?</tool_use>", re.DOTALL),
    re.compile(r"<tool_result>.*?</tool_result>", re.DOTALL),
    re.compile(r"`[^`]*(?:rm\s+-rf|sudo|chmod|chown|curl\s+\||\beval\b)[^`]*`"),
    re.compile(r"https?://\S+"),
    re.compile(r"\$\(.*?\)"),
]


def sanitize_for_context(text: str) -> str:
    """Strip dangerous patterns from memory content before LLM context injection."""
    result = text
    for pattern in _DANGEROUS_PATTERNS:
        result = pattern.sub("[REDACTED]", result)
    return result.strip()


def trust_filter(trust: str = "high") -> str:
    """Return Cypher WHERE clause for trust level filtering."""
    if trust == "high":
        return "trust_level = 'user'"
    if trust == "agent":
        return "trust_level = 'agent'"
    return ""
