"""Test CLI bridge -- subprocess wrapper for forge-core."""
import json
import pytest


def test_find_forge_core_returns_path():
    from forge_graph.cli_bridge import find_forge_core
    path = find_forge_core()
    # Should find it in target/release/ since we built it
    assert path is not None
    assert "forge-core" in path


def test_run_forge_core_index():
    from forge_graph.cli_bridge import run_forge_core
    result = run_forge_core(["index", "src/forge_graph/hooks/"])
    assert result.returncode == 0
    lines = [l for l in result.stdout.strip().split("\n") if l.strip()]
    assert len(lines) > 0
    for line in lines:
        data = json.loads(line)
        assert "kind" in data


def test_run_forge_core_scan():
    from forge_graph.cli_bridge import run_forge_core
    result = run_forge_core(["scan", "tests/"])
    assert result.returncode == 0


def test_run_forge_core_bad_subcommand():
    from forge_graph.cli_bridge import run_forge_core
    result = run_forge_core(["nonexistent-subcommand-xyz"])
    assert result.returncode != 0
