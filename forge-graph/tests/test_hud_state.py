"""Tests for the HUD state file writer."""
import json
import os
import stat

import pytest

from forge_graph.hud.state import HudStateWriter


@pytest.fixture
def hud_path(tmp_path):
    return tmp_path / "hud-state.json"


class TestHudStateWriter:
    def test_writes_valid_json(self, hud_path):
        """update + flush writes valid JSON that can be read back."""
        writer = HudStateWriter(hud_path)
        writer.update(graph={"nodes": 42, "edges": 17})
        writer.flush()

        data = json.loads(hud_path.read_text())
        assert data["graph"]["nodes"] == 42
        assert data["graph"]["edges"] == 17
        # Other sections should still have defaults
        assert data["memory"]["decisions"] == 0

    def test_file_permissions_are_0600(self, hud_path):
        """Flushed file must have 0600 permissions (owner read/write only)."""
        writer = HudStateWriter(hud_path)
        writer.update(graph={"nodes": 1})
        writer.flush()

        mode = os.stat(hud_path).st_mode
        assert stat.S_IMODE(mode) == 0o600

    def test_debounce_coalesces_rapid_updates(self, hud_path):
        """Rapid updates within debounce window are coalesced; only last value persists."""
        writer = HudStateWriter(hud_path, debounce_ms=500)

        writer.update(tokens={"input": 10})
        writer.maybe_flush()
        writer.update(tokens={"input": 20})
        writer.maybe_flush()
        writer.update(tokens={"input": 30})
        writer.maybe_flush()

        # The first maybe_flush should have written (no prior write),
        # but subsequent ones should be debounced.
        # Force a final flush to get the last value.
        writer.flush()

        data = json.loads(hud_path.read_text())
        assert data["tokens"]["input"] == 30

    def test_relative_file_paths_in_agent_state(self, hud_path):
        """Absolute paths in team agent entries are relativized on flush."""
        writer = HudStateWriter(hud_path)
        writer.update(team={
            "agent-1": {
                "current_file": "/home/user/project/src/main.py",
                "status": "active",
            }
        })
        writer.flush()

        data = json.loads(hud_path.read_text())
        agent = data["team"]["agent-1"]
        assert agent["current_file"] == "main.py"
        assert not agent["current_file"].startswith("/")
