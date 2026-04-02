"""Test code ingestion — forge-core JSON -> LadybugDB nodes."""
import json
import pytest
from pathlib import Path


SAMPLE_NDJSON = [
    {"kind": "file", "id": "file-abc123", "name": "auth.py", "file_path": "src/auth.py", "language": "python", "size_bytes": 500},
    {"kind": "class", "id": "class-def456", "name": "AuthMiddleware", "file_path": "src/auth.py", "line_start": 5, "line_end": 20},
    {"kind": "method", "id": "method-ghi789", "name": "verify_token", "file_path": "src/auth.py", "line_start": 10, "line_end": 15, "signature": "def verify_token(self, token: str) -> bool", "class_id": "class-def456"},
    {"kind": "function", "id": "func-jkl012", "name": "create_app", "file_path": "src/auth.py", "line_start": 22, "line_end": 25, "signature": "def create_app()"},
]


@pytest.fixture
def db_with_schema(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    yield db
    db.close()


def test_ingest_creates_nodes(db_with_schema):
    from forge_graph.code.ingest import ingest_symbols
    ndjson = "\n".join(json.dumps(s) for s in SAMPLE_NDJSON)
    count = ingest_symbols(db_with_schema, ndjson)
    assert count == 4

    r = db_with_schema.conn.execute("MATCH (f:File) RETURN f.name AS name")
    rows = r.get_as_pl()
    assert rows["name"][0] == "auth.py"


def test_ingest_creates_contains_edges(db_with_schema):
    from forge_graph.code.ingest import ingest_symbols
    ndjson = "\n".join(json.dumps(s) for s in SAMPLE_NDJSON)
    ingest_symbols(db_with_schema, ndjson)

    r = db_with_schema.conn.execute(
        "MATCH (f:File)-[:CONTAINS]->(fn:Function) RETURN fn.name AS name"
    )
    rows = r.get_as_pl()
    assert "create_app" in list(rows["name"])


def test_ingest_is_idempotent(db_with_schema):
    from forge_graph.code.ingest import ingest_symbols
    ndjson = "\n".join(json.dumps(s) for s in SAMPLE_NDJSON)
    ingest_symbols(db_with_schema, ndjson)
    ingest_symbols(db_with_schema, ndjson)  # Run again

    r = db_with_schema.conn.execute("MATCH (f:File) RETURN count(f) AS c")
    rows = r.get_as_pl()
    assert int(rows["c"][0]) == 1  # No duplicates
