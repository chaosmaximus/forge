"""Proxy Axon's code intelligence tools + sandbox axon_cypher."""
import re

_MEMORY_LABELS = frozenset({
    "decision", "pattern", "lesson", "preference", "session", "skill", "secret",
    "_forge_meta",
})

_WRITE_KEYWORDS = re.compile(
    r"\b(CREATE|SET|DELETE|MERGE|REMOVE|DETACH)\b", re.IGNORECASE
)


def validate_cypher_query(query: str) -> bool:
    """Validate a Cypher query for the axon_cypher sandbox.

    Returns True if safe (read-only, code nodes only).
    Returns False if it accesses memory nodes or attempts writes.
    """
    if _WRITE_KEYWORDS.search(query):
        return False
    query_lower = query.lower()
    for label in _MEMORY_LABELS:
        if re.search(rf":{label}\b", query_lower):
            return False
    return True
