"""Shared test fixtures."""
from pathlib import Path
import pytest
import real_ladybug as lb


@pytest.fixture
def tmp_db(tmp_path: Path):
    db_path = tmp_path / "test.lbdb"
    db = lb.Database(str(db_path))
    conn = lb.Connection(db)
    yield conn, db_path
    db.close()


@pytest.fixture
def graph_db(tmp_path: Path):
    from forge_graph.db import GraphDB
    db = GraphDB(tmp_path / "test.lbdb")
    yield db
    db.close()
