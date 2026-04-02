"""Evolution safety — output sanitization, path validation, diff limits."""
import re

MAX_DIFF_LINES = 500
ALLOWED_PREFIXES = ("skills/",)
BLOCKED_PATHS = frozenset({"hooks/", "scripts/", "agents/", ".claude-plugin/", "plugin.json", "hooks.json"})


def validate_evolution_path(path: str) -> bool:
    """Check if a file path is allowed for evolution writes."""
    if ".." in path:
        return False
    if not any(path.startswith(p) for p in ALLOWED_PREFIXES):
        return False
    if any(path.startswith(b) for b in BLOCKED_PATHS):
        return False
    return True


def validate_diff_size(diff: str) -> bool:
    """Check if a diff is within size limits (500 lines max)."""
    return diff.count("\n") <= MAX_DIFF_LINES


def sanitize_llm_output(output: str) -> str:
    """Strip <think> blocks and LLM artifacts. Handles nested tags."""
    result = output
    # Iteratively remove <think>...</think> (handles nesting)
    while True:
        cleaned = re.sub(r"<think>.*?</think>", "", result, flags=re.DOTALL)
        if cleaned == result:
            break
        result = cleaned
    # Remove unclosed <think> tags
    result = re.sub(r"<think>[^<]*$", "", result, flags=re.DOTALL)
    return result.strip()
