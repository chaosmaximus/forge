"""Bridge to forge-core Rust CLI. Finds binary and calls subcommands."""
import os
import shutil
import subprocess
from pathlib import Path
from typing import Optional


def find_forge_core() -> Optional[str]:
    """Find forge-core binary. Checks: PATH, workspace target, plugin servers."""
    found = shutil.which("forge-core")
    if found:
        return found

    workspace = Path(__file__).resolve().parents[3]
    candidates = [
        workspace / "target" / "release" / "forge-core",
        workspace / "target" / "debug" / "forge-core",
        workspace / "forge-core" / "target" / "release" / "forge-core",
    ]
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return str(c)

    plugin_root = os.environ.get("CLAUDE_PLUGIN_ROOT", "")
    if plugin_root:
        srv = Path(plugin_root) / "servers" / "forge-core"
        if srv.is_file() and os.access(srv, os.X_OK):
            return str(srv)

    return None


def run_forge_core(
    args: list,
    timeout: int = 120,
    cwd: Optional[str] = None,
) -> subprocess.CompletedProcess:
    """Run forge-core with given args. Returns CompletedProcess."""
    binary = find_forge_core()
    if not binary:
        return subprocess.CompletedProcess(
            args=["forge-core"] + args,
            returncode=127,
            stdout="",
            stderr="forge-core binary not found. Build with: cargo build --release -p forge-core",
        )
    try:
        return subprocess.run(
            [binary] + args,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd or os.getcwd(),
        )
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(
            args=[binary] + args,
            returncode=124,
            stdout="",
            stderr=f"forge-core timed out after {timeout}s",
        )
    except FileNotFoundError:
        return subprocess.CompletedProcess(
            args=[binary] + args,
            returncode=127,
            stdout="",
            stderr=f"forge-core binary at {binary} not executable",
        )
