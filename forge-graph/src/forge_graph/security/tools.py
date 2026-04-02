"""Security scanning — scan directory for exposed secrets.

Pure logic, no MCP. Called by forge-core scan (Rust) directly.
Python fallback available via forge_graph.cli.
"""
import json
import os
from pathlib import Path
from typing import Optional

from forge_graph.auth import check_access
from forge_graph.security.scanner import scan_content

_SCANNABLE = frozenset({
    ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".rs", ".java",
    ".yml", ".yaml", ".json", ".toml", ".ini", ".cfg", ".conf",
    ".env", ".sh", ".bash", ".tf", ".tfvars",
})
_SKIP_DIRS = frozenset({
    ".git", "node_modules", "__pycache__", ".venv", "venv",
    ".mypy_cache", ".ruff_cache", ".pytest_cache", "dist", "build", ".axon",
})


def scan_directory(path: Path, depth: str = "shallow") -> list:
    """Scan a directory for secrets. Returns list of SecretFinding."""
    from forge_graph.security.scanner import SecretFinding
    findings: list[SecretFinding] = []
    max_file_size = 1_000_000
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if d not in _SKIP_DIRS]
        for fname in files:
            fpath = Path(root) / fname
            if fpath.is_symlink():
                continue
            if fpath.suffix not in _SCANNABLE and fpath.name not in {".env", ".npmrc", ".pypirc"}:
                continue
            try:
                if fpath.stat().st_size > max_file_size:
                    continue
                content = fpath.read_text(errors="ignore")
                rel_path = str(fpath.relative_to(path))
                findings.extend(scan_content(content, rel_path))
            except (PermissionError, OSError):
                continue
    return findings
