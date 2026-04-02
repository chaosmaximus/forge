"""Build _meta dict for every MCP tool response."""
import time
from dataclasses import dataclass, field


@dataclass
class ToolMeta:
    start_time: float = field(default_factory=time.monotonic)
    tokens_input: int = 0
    tokens_output: int = 0
    llm_calls: int = 0
    path: str = "deterministic"

    def finish(self) -> dict:
        return {
            "tokens_input": self.tokens_input,
            "tokens_output": self.tokens_output,
            "llm_calls": self.llm_calls,
            "duration_ms": round((time.monotonic() - self.start_time) * 1000),
            "path": self.path,
        }
