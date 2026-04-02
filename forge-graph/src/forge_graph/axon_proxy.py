"""Proxy Axon's code intelligence tools + sandbox axon_cypher."""
import re

_CODE_LABELS = frozenset({
    "function", "class", "file", "folder", "method", "interface",
    "typealias", "enum", "community", "process",
    "coderelation",  # Axon's edge table
})

_WRITE_KEYWORDS = re.compile(
    r"\b(CREATE|SET|DELETE|MERGE|REMOVE|DETACH)\b", re.IGNORECASE
)


def validate_cypher_query(query: str) -> bool:
    """Validate a Cypher query for the axon_cypher sandbox.

    Returns True if safe (read-only, code nodes only).
    Returns False if it accesses memory nodes or attempts writes.

    Security model: allowlist of code labels. Any query that references
    a label NOT in _CODE_LABELS is rejected. Unlabeled node matches
    (e.g. ``MATCH (n)`` or ``MATCH ()``) are also rejected because they
    can access any table including memory nodes.
    """
    if _WRITE_KEYWORDS.search(query):
        return False

    # Block unlabeled node matches: MATCH (n) or MATCH ()
    # These can access any table including memory nodes
    node_patterns = re.findall(
        r'MATCH\s*\((\s*\w*\s*(?::\w+)?)\s*\)', query, re.IGNORECASE
    )
    for m in node_patterns:
        if ':' not in m:
            return False

    # Extract node labels from (var:Label) patterns — NOT relationship types
    # from [r:REL_TYPE] patterns. Only node labels determine table access.
    node_labels = re.findall(r'\(\s*\w*\s*:\s*(\w+)', query)
    if not node_labels:
        return False  # No node labels = suspicious

    # All node labels must be in code allowlist
    for label in node_labels:
        if label.lower() not in _CODE_LABELS:
            return False

    return True
