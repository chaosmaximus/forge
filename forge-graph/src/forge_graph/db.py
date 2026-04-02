"""LadybugDB connection manager with write serialization.

Provides both synchronous and async access to the database:

- ``db.conn`` — the raw synchronous ``lb.Connection`` escape hatch.
  Use this when calling from synchronous code or when the MCP tool
  handler is already managing the async boundary.

- ``await db.execute(...)`` / ``await db.write(...)`` — async wrappers
  that run the synchronous DB calls via ``asyncio.to_thread()``,
  preventing event-loop blocking.  Prefer these in new async code.
"""
import asyncio
from pathlib import Path
import real_ladybug as lb


class GraphDB:
    def __init__(self, db_path: str | Path) -> None:
        self._path = Path(db_path)
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self._db = lb.Database(str(self._path))
        self._conn = lb.Connection(self._db)
        self._write_lock = asyncio.Lock()

    @property
    def conn(self) -> lb.Connection:
        """Synchronous connection — use for direct DB access outside async paths."""
        return self._conn

    async def execute(self, query: str, parameters: dict | None = None) -> lb.QueryResult:
        """Run a read query off the event loop via ``asyncio.to_thread``."""
        return await asyncio.to_thread(self._conn.execute, query, parameters or {})

    async def write(self, query: str, parameters: dict | None = None) -> lb.QueryResult:
        """Run a write query under the write lock, off the event loop."""
        try:
            acquired = await asyncio.wait_for(self._write_lock.acquire(), timeout=5)
        except asyncio.TimeoutError:
            raise TimeoutError("Write lock timeout (5s)")
        try:
            return await asyncio.to_thread(
                self._conn.execute, query, parameters or {}
            )
        finally:
            self._write_lock.release()

    def close(self) -> None:
        self._db.close()
