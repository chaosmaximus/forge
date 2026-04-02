"""LadybugDB connection manager with write serialization."""
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
        return self._conn

    async def execute(self, query: str, parameters: dict | None = None) -> lb.QueryResult:
        return self._conn.execute(query, parameters=parameters or {})

    async def write(self, query: str, parameters: dict | None = None) -> lb.QueryResult:
        try:
            async with asyncio.timeout(5):
                async with self._write_lock:
                    return self._conn.execute(query, parameters=parameters or {})
        except asyncio.TimeoutError:
            raise TimeoutError("Write lock timeout (5s)")

    def close(self) -> None:
        self._db.close()
