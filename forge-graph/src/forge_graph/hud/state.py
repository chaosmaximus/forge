"""Atomic, debounced HUD state file writer."""
import json
import os
import tempfile
import time
from pathlib import Path


class HudStateWriter:
    def __init__(self, path: str | Path, debounce_ms: int = 100) -> None:
        self._path = Path(path)
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self._debounce_s = debounce_ms / 1000.0
        self._last_write: float = 0.0
        self._state: dict = {
            "version": self._read_plugin_version(),
            "graph": {"nodes": 0, "edges": 0},
            "memory": {"decisions": 0, "patterns": 0, "lessons": 0, "secrets": 0},
            "session": {"mode": None, "phase": None, "wave": None},
            "tokens": {"input": 0, "output": 0, "llm_calls": 0, "deterministic_ratio": 1.0},
            "skills": {"active": 0, "fix_candidates": 0},
            "team": {},
            "security": {"total": 0, "stale": 0, "exposed": 0},
        }
        self._dirty = False

    @staticmethod
    def _read_plugin_version() -> str:
        """Read version from plugin.json, falling back to '0.2.0'."""
        for candidate in [
            os.environ.get("CLAUDE_PLUGIN_ROOT", ""),
            os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."),
        ]:
            pj = os.path.join(candidate, ".claude-plugin", "plugin.json") if candidate else ""
            if pj and os.path.isfile(pj):
                try:
                    with open(pj) as f:
                        return json.load(f).get("version", "0.2.0")
                except Exception:
                    pass
        return "0.2.0"

    def update(self, **sections: dict) -> None:
        for key, value in sections.items():
            if key in self._state and isinstance(self._state[key], dict) and isinstance(value, dict):
                self._state[key].update(value)
            elif key in self._state:
                self._state[key] = value
        self._dirty = True

    def maybe_flush(self) -> None:
        if not self._dirty:
            return
        now = time.monotonic()
        if now - self._last_write < self._debounce_s:
            return
        self.flush()

    def flush(self) -> None:
        if not self._dirty and self._path.exists():
            return
        # Ensure agent file paths are relative (security)
        for agent_info in self._state.get("team", {}).values():
            if isinstance(agent_info, dict) and "current_file" in agent_info:
                cf = agent_info["current_file"]
                if cf and cf.startswith("/"):
                    agent_info["current_file"] = os.path.basename(cf)
        # Atomic write: temp file + rename
        parent = self._path.parent
        fd, tmp_path = tempfile.mkstemp(dir=parent, suffix=".tmp")
        try:
            with os.fdopen(fd, "w") as f:
                json.dump(self._state, f, separators=(",", ":"))
            os.chmod(tmp_path, 0o600)
            os.rename(tmp_path, self._path)
        except Exception:
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
            raise
        self._last_write = time.monotonic()
        self._dirty = False
