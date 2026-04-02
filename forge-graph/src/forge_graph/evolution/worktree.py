"""Git worktree lifecycle for safe skill evolution.

Creates an isolated worktree so LLM edits never touch the main tree
until the user explicitly approves the diff.
"""
from __future__ import annotations

import subprocess
import tempfile
import uuid
from pathlib import Path


class EvolutionWorktree:
    """Context manager for a temporary git worktree."""

    def __init__(self, repo_path: str | Path) -> None:
        self._repo = Path(repo_path).resolve()
        self._worktree_path: Path | None = None
        self._branch_name: str | None = None
        self._tmpdir: tempfile.TemporaryDirectory | None = None

    # -- lifecycle ------------------------------------------------------------

    def create(self) -> Path:
        """Create a temporary git worktree branched from HEAD."""
        self._tmpdir = tempfile.TemporaryDirectory(prefix="forge-evolve-")
        wt_dir = Path(self._tmpdir.name) / "wt"
        self._branch_name = f"forge-evolve-{uuid.uuid4().hex[:8]}"

        subprocess.run(
            [
                "git", "worktree", "add",
                "-b", self._branch_name,
                str(wt_dir),
                "HEAD",
            ],
            cwd=str(self._repo),
            capture_output=True,
            check=True,
        )
        self._worktree_path = wt_dir
        return wt_dir

    def get_diff(self) -> str:
        """Return the diff of changes in the worktree vs its HEAD."""
        if self._worktree_path is None:
            raise RuntimeError("Worktree not created yet")

        result = subprocess.run(
            ["git", "diff"],
            cwd=str(self._worktree_path),
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout

    def apply_to_main(self) -> bool:
        """Apply worktree diff to the main repo tree.

        Returns True on success, False if the patch cannot be applied.
        """
        diff = self.get_diff()
        if not diff.strip():
            return False

        # Dry-run check
        check = subprocess.run(
            ["git", "apply", "--check"],
            input=diff,
            cwd=str(self._repo),
            capture_output=True,
            text=True,
        )
        if check.returncode != 0:
            return False

        # Actually apply
        subprocess.run(
            ["git", "apply"],
            input=diff,
            cwd=str(self._repo),
            capture_output=True,
            text=True,
            check=True,
        )
        return True

    def cleanup(self) -> None:
        """Remove the worktree and temporary branch."""
        if self._worktree_path is not None:
            subprocess.run(
                ["git", "worktree", "remove", "--force", str(self._worktree_path)],
                cwd=str(self._repo),
                capture_output=True,
            )
        if self._branch_name is not None:
            subprocess.run(
                ["git", "branch", "-D", self._branch_name],
                cwd=str(self._repo),
                capture_output=True,
            )
        if self._tmpdir is not None:
            self._tmpdir.cleanup()
            self._tmpdir = None
        self._worktree_path = None
        self._branch_name = None

    # -- context manager ------------------------------------------------------

    def __enter__(self) -> "EvolutionWorktree":
        self.create()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        self.cleanup()
