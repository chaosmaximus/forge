"""Temporal query helpers — current vs historical views."""


def CURRENT_VIEW(alias: str) -> str:
    """Cypher WHERE clause fragment for current (non-invalidated) nodes."""
    return f"{alias}.invalid_at IS NULL"


def current_filter(include_historical: bool = False) -> str:
    """Return a WHERE clause fragment. Prefix with AND in existing queries."""
    if include_historical:
        return ""
    return "AND n.invalid_at IS NULL"
