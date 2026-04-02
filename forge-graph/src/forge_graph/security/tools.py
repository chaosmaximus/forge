"""MCP tool: forge_scan — scan directory for exposed secrets."""
import json
import os
from pathlib import Path
from typing import Optional

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
            # P2: Skip symlinks to prevent reading files outside workspace
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


@mcp.tool()
async def forge_scan(
    path: Optional[str] = None, depth: str = "shallow", agent_id: Optional[str] = None
) -> str:
    """Scan directory for exposed secrets via forge-core Rust scanner."""
    if not check_access(agent_id, "forge_scan"):
        raise PermissionError(f"Agent '{agent_id}' does not have access to forge_scan")

    meta = ToolMeta()
    scan_path = (Path(path) if path else Path.cwd()).resolve()
    cwd = Path.cwd().resolve()
    try:
        scan_path.relative_to(cwd)
    except ValueError:
        raise PermissionError(f"Scan path '{scan_path}' is outside workspace '{cwd}'")

    # Try forge-core CLI first (Rust, fast)
    from forge_graph.cli_bridge import run_forge_core
    result = run_forge_core(["scan", str(scan_path)])

    if result.returncode == 0 and result.stdout.strip():
        # Parse NDJSON from Rust scanner
        findings_data = []
        for line in result.stdout.strip().split("\n"):
            if line.strip():
                try:
                    findings_data.append(json.loads(line))
                except json.JSONDecodeError:
                    continue

        # Store in graph
        db = get_db()
        for f in findings_data:
            import hashlib
            sid = "secret-" + hashlib.sha256(
                f"{f['file_path']}:{f['line_number']}:{f['fingerprint']}".encode()
            ).hexdigest()[:12]
            db.conn.execute(
                "MERGE (s:Secret {id: $sid}) "
                "SET s.file_path = $fp, s.line_number = $ln, "
                "s.type = $type, s.provider = $prov, s.discovered_at = current_timestamp(), "
                "s.risk_level = $risk, s.status = 'active', s.fingerprint = $fp2",
                parameters={"sid": sid, "fp": f["file_path"], "ln": f["line_number"],
                            "type": f.get("type", "unknown"), "prov": f.get("provider", "generic"),
                            "risk": f.get("risk_level", "medium"), "fp2": f.get("fingerprint", "")})

        return json.dumps({
            "total": len(findings_data),
            "by_risk": {r: sum(1 for f in findings_data if f.get("risk_level") == r)
                        for r in ("critical", "high", "medium", "low")},
            "findings": [{"rule": f.get("rule_id", ""), "provider": f.get("provider", ""),
                          "type": f.get("type", ""), "file": f.get("file_path", ""),
                          "line": f.get("line_number", 0), "risk": f.get("risk_level", ""),
                          "description": f.get("description", "")} for f in findings_data],
            "_meta": meta.finish()
        })

    # Fallback: Python scanner (if forge-core not available)
    findings = _scan_directory(scan_path, depth)
    db = get_db()
    for f in findings:
        import hashlib
        sid = "secret-" + hashlib.sha256(
            f"{f.file_path}:{f.line_number}:{f.fingerprint}".encode()
        ).hexdigest()[:12]
        db.conn.execute(
            "MERGE (s:Secret {id: $sid}) "
            "SET s.file_path = $fp, s.line_number = $ln, "
            "s.type = $type, s.provider = $prov, s.discovered_at = current_timestamp(), "
            "s.risk_level = $risk, s.status = 'active', s.fingerprint = $fp2",
            parameters={"sid": sid, "fp": f.file_path, "ln": f.line_number,
                        "type": f.type, "prov": f.provider,
                        "risk": f.risk_level, "fp2": f.fingerprint})
    return json.dumps({
        "total": len(findings),
        "by_risk": {r: sum(1 for f in findings if f.risk_level == r)
                    for r in ("critical", "high", "medium", "low")},
        "findings": [{"rule": f.rule_id, "provider": f.provider, "type": f.type,
                       "file": f.file_path, "line": f.line_number,
                       "risk": f.risk_level, "description": f.description} for f in findings],
        "_meta": meta.finish()
    })
