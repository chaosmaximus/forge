import json
from pathlib import Path
import subprocess
import pytest


@pytest.fixture
def db_with_skills(tmp_path: Path):
    from forge_graph.db import GraphDB
    from forge_graph.memory.schema import create_schema
    db = GraphDB(tmp_path / "test.lbdb")
    create_schema(db.conn)
    # Create a skill with high fallback rate (should trigger FIX)
    db.conn.execute("""
        CREATE (s:Skill {
            id: 'skill-fix', name: 'forge-feature', version: '1.0',
            generation: 0, is_active: true, content_hash: 'abc',
            total_selections: 10, total_applied: 8,
            total_completions: 4, total_fallbacks: 5,
            created_at: current_timestamp(), updated_at: current_timestamp()
        })
    """)
    # Create a healthy skill (should NOT trigger)
    db.conn.execute("""
        CREATE (s:Skill {
            id: 'skill-ok', name: 'forge-review', version: '1.0',
            generation: 0, is_active: true, content_hash: 'def',
            total_selections: 10, total_applied: 9,
            total_completions: 8, total_fallbacks: 1,
            created_at: current_timestamp(), updated_at: current_timestamp()
        })
    """)
    # Create a cold-start skill (too few selections)
    db.conn.execute("""
        CREATE (s:Skill {
            id: 'skill-new', name: 'forge-ship', version: '1.0',
            generation: 0, is_active: true, content_hash: 'ghi',
            total_selections: 3, total_applied: 2,
            total_completions: 0, total_fallbacks: 2,
            created_at: current_timestamp(), updated_at: current_timestamp()
        })
    """)
    yield db
    db.close()


def test_compute_skill_metrics_healthy(db_with_skills):
    from forge_graph.evolution.metrics import compute_skill_metrics
    m = compute_skill_metrics(db_with_skills, "skill-ok")
    assert m["cold_start"] is False
    assert m["applied_rate"] == pytest.approx(0.9)
    assert m["completion_rate"] == pytest.approx(8/9, abs=0.01)
    assert m["effective_rate"] == pytest.approx(0.8)
    assert m["fallback_rate"] == pytest.approx(1/9, abs=0.01)


def test_compute_skill_metrics_unhealthy(db_with_skills):
    from forge_graph.evolution.metrics import compute_skill_metrics
    m = compute_skill_metrics(db_with_skills, "skill-fix")
    assert m["fallback_rate"] == pytest.approx(5/8, abs=0.01)  # > 0.40


def test_cold_start_guard(db_with_skills):
    from forge_graph.evolution.metrics import compute_skill_metrics
    m = compute_skill_metrics(db_with_skills, "skill-new")
    assert m["cold_start"] is True


def test_check_evolution_candidates(db_with_skills):
    from forge_graph.evolution.metrics import check_evolution_candidates
    candidates = check_evolution_candidates(db_with_skills)
    # skill-fix should be a FIX candidate (fallback_rate > 0.40)
    # skill-ok should NOT be a candidate
    # skill-new should NOT be a candidate (cold start)
    assert len(candidates) == 1
    assert candidates[0]["skill_name"] == "forge-feature"
    assert candidates[0]["evolution_type"] == "fix"


def test_format_suggestions_empty():
    from forge_graph.evolution.engine import format_suggestions_for_user
    assert format_suggestions_for_user([]) == ""


def test_format_suggestions_with_candidates():
    from forge_graph.evolution.engine import format_suggestions_for_user
    suggestions = [{"skill": "forge-feature", "type": "fix", "reason": "fallback_rate=0.63 > 0.40", "metrics": {}}]
    output = format_suggestions_for_user(suggestions)
    assert "forge-feature" in output
    assert "AUTO-FIX" in output
    assert "(a)" in output


def test_worktree_lifecycle(tmp_path: Path):
    """Test worktree create/modify/diff/cleanup."""
    # Create a temp git repo
    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init", str(repo)], capture_output=True, check=True)
    subprocess.run(["git", "-C", str(repo), "config", "user.email", "test@test.com"], capture_output=True)
    subprocess.run(["git", "-C", str(repo), "config", "user.name", "Test"], capture_output=True)
    # Create initial commit
    (repo / "skills" / "test").mkdir(parents=True)
    (repo / "skills" / "test" / "SKILL.md").write_text("original")
    subprocess.run(["git", "-C", str(repo), "add", "."], capture_output=True)
    subprocess.run(["git", "-C", str(repo), "commit", "-m", "init"], capture_output=True)

    from forge_graph.evolution.worktree import EvolutionWorktree
    with EvolutionWorktree(repo) as wt:
        wt_path = wt._worktree_path
        assert wt_path.exists()
        # Modify a file in worktree
        (wt_path / "skills" / "test" / "SKILL.md").write_text("modified")
        diff = wt.get_diff()
        assert "modified" in diff
    # After context manager, worktree should be cleaned up
    assert not wt_path.exists()
