"""MCP tool: forge_scan — scan directory for exposed secrets."""
import json
import os
from pathlib import Path

from forge_graph.auth import check_access
from forge_graph.meta import ToolMeta
from forge_graph.security.scanner import scan_content
from forge_graph.server import mcp, get_db

_SCANNABLE = frozenset({
    ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".rs", ".java",
    ".yml", ".yaml", ".json", ".toml", ".ini", ".cfg", ".conf",
    ".env", ".sh", ".bash", ".tf", ".tfvars",
})
_SKIP_DIRS = frozenset({
    ".git", "node_modules", "__pycache__", ".venv", "venv",
    ".mypy_cache", ".ruff_cache", ".pytest_cache", "dist", "build", ".axon",
})


def _scan_directory(path: Path, depth: str = "shallow") -> list:
    from forge_graph.security.scanner import SecretFinding
    findings: list[SecretFinding] = []
    max_file_size = 1_000_000
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if d not in _SKIP_DIRS]
        for fname in files:
            fpath = Path(root) / fname
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


@mcp.tool()
async def forge_scan(
    path: str | None = None, depth: str = "shallow", agent_id: str | None = None
) -> str:
    """Scan directory for exposed secrets. NEVER stores actual secret values."""
    if not check_access(agent_id, "forge_scan"):
        raise PermissionError(
            f"Agent '{agent_id}' does not have access to forge_scan"
        )
    meta = ToolMeta()
    scan_path = (Path(path) if path else Path.cwd()).resolve()
    # Restrict scan path to cwd or subdirectories — reject paths outside workspace
    cwd = Path.cwd().resolve()
    try:
        scan_path.relative_to(cwd)
    except ValueError:
        raise PermissionError(
            f"Scan path '{scan_path}' is outside the workspace root '{cwd}'"
        )
    findings = _scan_directory(scan_path, depth)
    # Store as Secret nodes (no actual secret values)
    db = get_db()
    for f in findings:
        db.conn.execute(
            "MERGE (s:Secret {file_path: $fp, line_number: $ln}) "
            "SET s.type = $type, s.provider = $prov, s.discovered_at = current_timestamp(), "
            "s.risk_level = $risk, s.status = 'active', s.fingerprint = $fp2",
            parameters={"fp": f.file_path, "ln": f.line_number,
                        "type": f.type, "prov": f.provider,
                        "risk": f.risk_level, "fp2": f.fingerprint})
    return json.dumps({
        "total": len(findings),
        "by_risk": {r: sum(1 for f in findings if f.risk_level == r)
                    for r in ("critical", "high", "medium", "low")},
        "findings": [{"rule": f.rule_id, "provider": f.provider, "type": f.type,
                       "file": f.file_path, "line": f.line_number,
                       "risk": f.risk_level, "description": f.description}
                      for f in findings],
        "_meta": meta.finish()})
