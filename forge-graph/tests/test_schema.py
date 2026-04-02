"""Tests for LadybugDB schema creation and migration framework."""
import shutil
from pathlib import Path

import pytest
import real_ladybug as lb


NODE_TABLES = ["Decision", "Pattern", "Lesson", "Preference", "Session", "Skill", "Secret"]

EDGE_TABLES = [
    ("SUPERSEDES", "Decision", "Decision"),
    ("MOTIVATED_BY", "Decision", "Lesson"),
    ("FOLLOWS", "Decision", "Pattern"),
    ("CONTRADICTS", "Decision", "Decision"),
    ("LEARNED_IN", "Lesson", "Session"),
    ("DECIDED_IN", "Decision", "Session"),
    ("EVOLVED_FROM", "Skill", "Skill"),
    ("APPLIED_IN", "Skill", "Session"),
]


def _table_exists(conn: lb.Connection, table_name: str) -> bool:
    """Check if a node table exists by attempting a MATCH query."""
    try:
        conn.execute(f"MATCH (n:{table_name}) RETURN count(n)")
        return True
    except RuntimeError:
        return False


def _edge_exists(conn: lb.Connection, edge_name: str, from_table: str, to_table: str) -> bool:
    """Check if an edge table exists by attempting a MATCH query."""
    try:
        conn.execute(
            f"MATCH (a:{from_table})-[r:{edge_name}]->(b:{to_table}) RETURN count(r)"
        )
        return True
    except RuntimeError:
        return False


class TestCreateSchema:
    """Tests for create_schema()."""

    def test_create_schema_creates_all_node_tables(self, tmp_db):
        """All 7 node tables should exist after create_schema."""
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)

        for table in NODE_TABLES:
            assert _table_exists(conn, table), f"Node table {table} should exist"

    def test_create_schema_creates_edge_tables(self, tmp_db):
        """All 8 edge tables should exist and allow traversal."""
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)

        for edge_name, from_table, to_table in EDGE_TABLES:
            assert _edge_exists(conn, edge_name, from_table, to_table), (
                f"Edge table {edge_name} ({from_table} -> {to_table}) should exist"
            )

    def test_schema_version_tracked(self, tmp_db):
        """Schema version should be 1 after initial create_schema."""
        from forge_graph.memory.migrate import get_schema_version
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)

        version = get_schema_version(conn)
        assert version == 1, f"Expected schema version 1, got {version}"

    def test_schema_is_idempotent(self, tmp_db):
        """Calling create_schema twice should not raise errors."""
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)
        # Second call must not fail
        create_schema(conn)

        # Tables should still exist
        for table in NODE_TABLES:
            assert _table_exists(conn, table), f"Node table {table} should still exist"

    def test_create_schema_creates_code_node_tables(self, tmp_db):
        """Code intelligence node tables (File, Function, Class, Method) should exist."""
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)
        for label in ["File", "Function", "Class", "Method"]:
            conn.execute(f"MATCH (n:{label}) RETURN count(n)")

    def test_create_schema_creates_code_edge_tables(self, tmp_db):
        """Code intelligence edge tables (CONTAINS, CALLS, IMPORTS) should exist."""
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)
        conn.execute(
            "CREATE (f:File {id: 'f1', file_path: 'test.py', name: 'test.py'})"
        )
        conn.execute(
            "CREATE (fn:Function {id: 'fn1', name: 'foo', file_path: 'test.py', "
            "line_start: 1, line_end: 5, signature: 'def foo()'})"
        )
        conn.execute(
            "MATCH (f:File {id: 'f1'}), (fn:Function {id: 'fn1'}) "
            "CREATE (f)-[:CONTAINS]->(fn)"
        )


class TestMigrationFramework:
    """Tests for the migration framework."""

    def test_get_set_schema_version(self, tmp_db):
        """get/set schema version round-trips."""
        from forge_graph.memory.migrate import get_schema_version, set_schema_version
        from forge_graph.memory.schema import create_schema

        conn, db_path = tmp_db
        create_schema(conn)

        assert get_schema_version(conn) == 1
        set_schema_version(conn, 2)
        assert get_schema_version(conn) == 2

    def test_backup_and_restore(self, tmp_path):
        """backup_db creates a copy; restore_from_backup restores it."""
        from forge_graph.memory.migrate import backup_db, restore_from_backup
        from forge_graph.memory.schema import create_schema

        db_path = tmp_path / "test.lbdb"
        db = lb.Database(str(db_path))
        conn = lb.Connection(db)
        create_schema(conn)
        db.close()

        backup_path = backup_db(db_path)
        assert backup_path.exists(), "Backup should exist"

        # Remove original (file or directory)
        if db_path.is_dir():
            shutil.rmtree(db_path)
        else:
            db_path.unlink()
        assert not db_path.exists()

        restore_from_backup(db_path, backup_path)
        assert db_path.exists(), "DB should be restored"

        # Verify restored DB works
        db2 = lb.Database(str(db_path))
        conn2 = lb.Connection(db2)
        result = conn2.execute("MATCH (n:Decision) RETURN count(n)")
        assert result.get_next() is not None
        db2.close()
