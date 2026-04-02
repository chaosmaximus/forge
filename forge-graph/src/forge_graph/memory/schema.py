"""LadybugDB schema definition — 11 node tables + 11 edge tables.

Memory tables: Decision, Pattern, Lesson, Preference, Session, Skill, Secret
Code tables:   File, Function, Class, Method

All CREATE statements use IF NOT EXISTS for idempotency.
"""
from __future__ import annotations

import real_ladybug as lb

# ---------------------------------------------------------------------------
# Schema version written to _forge_meta on first creation
# ---------------------------------------------------------------------------
SCHEMA_VERSION = 1

# ---------------------------------------------------------------------------
# Node table DDL
# ---------------------------------------------------------------------------
_NODE_TABLES: list[str] = [
    # 1. Decision
    """
    CREATE NODE TABLE IF NOT EXISTS Decision (
        id STRING,
        title STRING,
        rationale STRING,
        status STRING DEFAULT 'active',
        created_at TIMESTAMP,
        updated_at TIMESTAMP,
        valid_at TIMESTAMP,
        invalid_at TIMESTAMP,
        confidence DOUBLE DEFAULT 1.0,
        trust_level STRING DEFAULT 'user',
        PRIMARY KEY (id)
    )
    """,
    # 2. Pattern
    """
    CREATE NODE TABLE IF NOT EXISTS Pattern (
        id STRING,
        name STRING,
        description STRING,
        domain STRING,
        frequency INT64 DEFAULT 0,
        confidence DOUBLE DEFAULT 0.5,
        generation INT64 DEFAULT 0,
        created_at TIMESTAMP,
        updated_at TIMESTAMP,
        valid_at TIMESTAMP,
        invalid_at TIMESTAMP,
        PRIMARY KEY (id)
    )
    """,
    # 3. Lesson
    """
    CREATE NODE TABLE IF NOT EXISTS Lesson (
        id STRING,
        insight STRING,
        context STRING,
        severity STRING DEFAULT 'info',
        created_at TIMESTAMP,
        updated_at TIMESTAMP,
        valid_at TIMESTAMP,
        invalid_at TIMESTAMP,
        PRIMARY KEY (id)
    )
    """,
    # 4. Preference
    """
    CREATE NODE TABLE IF NOT EXISTS Preference (
        id STRING,
        key STRING,
        value STRING,
        scope STRING DEFAULT 'project',
        confidence DOUBLE DEFAULT 1.0,
        created_at TIMESTAMP,
        updated_at TIMESTAMP,
        valid_at TIMESTAMP,
        invalid_at TIMESTAMP,
        PRIMARY KEY (id)
    )
    """,
    # 5. Session
    """
    CREATE NODE TABLE IF NOT EXISTS Session (
        id STRING,
        started_at TIMESTAMP,
        ended_at TIMESTAMP,
        mode STRING,
        project STRING,
        outcome STRING,
        summary STRING,
        total_tokens_input INT64 DEFAULT 0,
        total_tokens_output INT64 DEFAULT 0,
        total_llm_calls INT64 DEFAULT 0,
        total_tool_calls INT64 DEFAULT 0,
        deterministic_ratio DOUBLE DEFAULT 1.0,
        PRIMARY KEY (id)
    )
    """,
    # 6. Skill
    """
    CREATE NODE TABLE IF NOT EXISTS Skill (
        id STRING,
        name STRING,
        version STRING,
        generation INT64 DEFAULT 0,
        is_active BOOLEAN DEFAULT true,
        content_hash STRING,
        total_selections INT64 DEFAULT 0,
        total_applied INT64 DEFAULT 0,
        total_completions INT64 DEFAULT 0,
        total_fallbacks INT64 DEFAULT 0,
        created_at TIMESTAMP,
        updated_at TIMESTAMP,
        PRIMARY KEY (id)
    )
    """,
    # 7. Secret
    """
    CREATE NODE TABLE IF NOT EXISTS Secret (
        id STRING,
        type STRING,
        provider STRING DEFAULT 'generic',
        file_path STRING,
        line_number INT64,
        discovered_at TIMESTAMP,
        last_rotated TIMESTAMP,
        age_days INT64 DEFAULT 0,
        risk_level STRING DEFAULT 'medium',
        status STRING DEFAULT 'active',
        fingerprint STRING,
        PRIMARY KEY (id)
    )
    """,
    # -----------------------------------------------------------------------
    # Code intelligence node tables
    # -----------------------------------------------------------------------
    # 8. File
    """
    CREATE NODE TABLE IF NOT EXISTS File (
        id STRING,
        file_path STRING,
        name STRING,
        language STRING DEFAULT 'unknown',
        size_bytes INT64 DEFAULT 0,
        indexed_at STRING,
        PRIMARY KEY (id)
    )
    """,
    # 9. Function
    """
    CREATE NODE TABLE IF NOT EXISTS Function (
        id STRING,
        name STRING,
        file_path STRING,
        line_start INT64,
        line_end INT64,
        signature STRING DEFAULT '',
        PRIMARY KEY (id)
    )
    """,
    # 10. Class
    """
    CREATE NODE TABLE IF NOT EXISTS Class (
        id STRING,
        name STRING,
        file_path STRING,
        line_start INT64,
        line_end INT64,
        PRIMARY KEY (id)
    )
    """,
    # 11. Method
    """
    CREATE NODE TABLE IF NOT EXISTS Method (
        id STRING,
        name STRING,
        class_id STRING DEFAULT '',
        file_path STRING,
        line_start INT64,
        line_end INT64,
        signature STRING DEFAULT '',
        PRIMARY KEY (id)
    )
    """,
]

# ---------------------------------------------------------------------------
# Edge (REL) table DDL
# ---------------------------------------------------------------------------
_EDGE_TABLES: list[str] = [
    # Decision -> Decision
    """
    CREATE REL TABLE IF NOT EXISTS SUPERSEDES (
        FROM Decision TO Decision,
        valid_at TIMESTAMP,
        invalid_at TIMESTAMP,
        reason STRING
    )
    """,
    # Decision -> Lesson
    """
    CREATE REL TABLE IF NOT EXISTS MOTIVATED_BY (
        FROM Decision TO Lesson,
        strength DOUBLE DEFAULT 1.0
    )
    """,
    # Decision -> Pattern
    """
    CREATE REL TABLE IF NOT EXISTS FOLLOWS (
        FROM Decision TO Pattern,
        confidence DOUBLE DEFAULT 1.0
    )
    """,
    # Decision -> Decision
    """
    CREATE REL TABLE IF NOT EXISTS CONTRADICTS (
        FROM Decision TO Decision,
        detected_at TIMESTAMP,
        resolved BOOLEAN DEFAULT false
    )
    """,
    # Lesson -> Session
    """
    CREATE REL TABLE IF NOT EXISTS LEARNED_IN (
        FROM Lesson TO Session
    )
    """,
    # Decision -> Session
    """
    CREATE REL TABLE IF NOT EXISTS DECIDED_IN (
        FROM Decision TO Session
    )
    """,
    # Skill -> Skill
    """
    CREATE REL TABLE IF NOT EXISTS EVOLVED_FROM (
        FROM Skill TO Skill,
        evolution_type STRING,
        diff_ref STRING
    )
    """,
    # Skill -> Session
    """
    CREATE REL TABLE IF NOT EXISTS APPLIED_IN (
        FROM Skill TO Session,
        outcome STRING,
        tokens_input INT64 DEFAULT 0,
        tokens_output INT64 DEFAULT 0,
        llm_calls INT64 DEFAULT 0
    )
    """,
    # -----------------------------------------------------------------------
    # Code intelligence edge tables
    # -----------------------------------------------------------------------
    # File -> Function, File -> Class, File -> Method
    """
    CREATE REL TABLE IF NOT EXISTS CONTAINS (
        FROM File TO Function,
        FROM File TO Class,
        FROM File TO Method
    )
    """,
    # Function -> Function, Method -> Function, Method -> Method
    """
    CREATE REL TABLE IF NOT EXISTS CALLS (
        FROM Function TO Function,
        FROM Method TO Function,
        FROM Method TO Method,
        confidence FLOAT DEFAULT 0.5
    )
    """,
    # File -> File
    """
    CREATE REL TABLE IF NOT EXISTS IMPORTS (
        FROM File TO File
    )
    """,
    # Decision -> File/Function/Class (used by post_edit hook for decision-awareness)
    """
    CREATE REL TABLE IF NOT EXISTS AFFECTS (
        FROM Decision TO File,
        FROM Decision TO Function,
        FROM Decision TO Class,
        impact STRING DEFAULT 'medium'
    )
    """,
]

# ---------------------------------------------------------------------------
# Version meta table
# ---------------------------------------------------------------------------
_META_TABLE = """
CREATE NODE TABLE IF NOT EXISTS _forge_meta (
    key STRING,
    value STRING,
    PRIMARY KEY (key)
)
"""


def create_schema(conn: lb.Connection) -> None:
    """Create all Forge memory schema tables (idempotent).

    Safe to call multiple times — every statement uses IF NOT EXISTS.
    """
    # Node tables
    for ddl in _NODE_TABLES:
        conn.execute(ddl)

    # Edge tables (must come after node tables they reference)
    for ddl in _EDGE_TABLES:
        conn.execute(ddl)

    # Meta table
    conn.execute(_META_TABLE)

    # Set initial schema version if not already present
    result = conn.execute(
        "MATCH (m:_forge_meta) WHERE m.key = 'schema_version' RETURN m.value"
    )
    if not result.has_next():
        conn.execute(
            "CREATE (m:_forge_meta {key: 'schema_version', value: $ver})",
            parameters={"ver": str(SCHEMA_VERSION)},
        )
