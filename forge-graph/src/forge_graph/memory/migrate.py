"""Schema migration framework for forge-graph.

Provides version tracking, backup/restore, and a registry for future
migration functions.
"""
from __future__ import annotations

import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

import real_ladybug as lb

# ---------------------------------------------------------------------------
# Migration registry — add entries as schema evolves
# Key = target version, value = callable(conn) that performs the migration.
# ---------------------------------------------------------------------------
MIGRATIONS: dict[int, Callable[[lb.Connection], None]] = {}


# ---------------------------------------------------------------------------
# Version helpers
# ---------------------------------------------------------------------------

def get_schema_version(conn: lb.Connection) -> int:
    """Return the current schema version from _forge_meta, or 0 if unset."""
    try:
        result = conn.execute(
            "MATCH (m:_forge_meta) WHERE m.key = 'schema_version' RETURN m.value"
        )
        if not result.has_next():
            return 0
        row = result.get_next()
        return int(row[0])
    except RuntimeError:
        # _forge_meta table does not exist yet
        return 0


def set_schema_version(conn: lb.Connection, version: int) -> None:
    """Update the schema version in _forge_meta.

    Assumes _forge_meta table and the schema_version row already exist
    (created by create_schema).
    """
    conn.execute(
        "MATCH (m:_forge_meta) WHERE m.key = 'schema_version' SET m.value = $ver",
        parameters={"ver": str(version)},
    )


# ---------------------------------------------------------------------------
# Backup / restore
# ---------------------------------------------------------------------------

def backup_db(db_path: str | Path) -> Path:
    """Copy the LadybugDB file (or directory) to a timestamped backup location.

    Returns the Path to the backup.
    """
    db_path = Path(db_path)
    if not db_path.exists():
        raise FileNotFoundError(f"Database path does not exist: {db_path}")

    ts = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    backup_path = db_path.parent / f"{db_path.name}.backup-{ts}"

    if db_path.is_dir():
        shutil.copytree(db_path, backup_path)
    else:
        shutil.copy2(db_path, backup_path)
    return backup_path


def restore_from_backup(db_path: str | Path, backup_path: str | Path) -> None:
    """Restore a LadybugDB file (or directory) from a backup.

    Removes the current db_path (if it exists) and copies the backup in
    its place.
    """
    db_path = Path(db_path)
    backup_path = Path(backup_path)

    if not backup_path.exists():
        raise FileNotFoundError(f"Backup path does not exist: {backup_path}")

    if db_path.exists():
        if db_path.is_dir():
            shutil.rmtree(db_path)
        else:
            db_path.unlink()

    if backup_path.is_dir():
        shutil.copytree(backup_path, db_path)
    else:
        shutil.copy2(backup_path, db_path)


# ---------------------------------------------------------------------------
# Run migrations (future use)
# ---------------------------------------------------------------------------

def migrate(conn: lb.Connection, db_path: str | Path | None = None) -> int:
    """Apply all pending migrations in order.

    Returns the final schema version after applying migrations.
    If db_path is provided, a backup is created before each migration step.
    """
    current = get_schema_version(conn)
    target_versions = sorted(v for v in MIGRATIONS if v > current)

    for version in target_versions:
        if db_path is not None:
            backup_db(db_path)
        MIGRATIONS[version](conn)
        set_schema_version(conn, version)

    return get_schema_version(conn)
