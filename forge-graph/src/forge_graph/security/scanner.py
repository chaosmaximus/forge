"""Secret scanner — regex + entropy detection, fingerprints only.

CRITICAL: We NEVER store the actual secret in the graph.
Only metadata + fingerprint (SHA256 of first4+last4 chars).
"""
import hashlib
import math
import re
from dataclasses import dataclass

from forge_graph.security.rules import RULES


@dataclass
class SecretFinding:
    rule_id: str
    provider: str
    type: str
    file_path: str
    line_number: int
    fingerprint: str  # SHA256 of first4+last4 — NEVER the full secret
    risk_level: str
    description: str


def _shannon_entropy(s: str) -> float:
    if not s:
        return 0.0
    freq: dict[str, int] = {}
    for c in s:
        freq[c] = freq.get(c, 0) + 1
    length = len(s)
    return -sum((count / length) * math.log2(count / length) for count in freq.values())


def _fingerprint(value: str) -> str:
    if len(value) < 8:
        fragment = value
    else:
        fragment = value[:4] + value[-4:]
    return hashlib.sha256(fragment.encode()).hexdigest()[:16]


def _risk_for_provider(provider: str) -> str:
    return {"aws": "critical", "gcp": "high", "azure": "critical",
            "github": "critical", "stripe": "critical"}.get(provider, "high")


def scan_content(content: str, file_path: str) -> list[SecretFinding]:
    findings: list[SecretFinding] = []
    lines = content.split("\n")
    for line_idx, line in enumerate(lines, start=1):
        # Layer 1: Regex rules
        for rule in RULES:
            if rule.pattern.search(line):
                match = rule.pattern.search(line)
                # P3-1: Prefer capture group 1 (the secret itself) over group 0
                # (which may include surrounding context like key names)
                if match and match.lastindex and match.lastindex >= 1:
                    value = match.group(1)
                else:
                    value = match.group(0) if match else ""
                findings.append(SecretFinding(
                    rule_id=rule.id, provider=rule.provider, type=rule.type,
                    file_path=file_path, line_number=line_idx,
                    fingerprint=_fingerprint(value),
                    risk_level=_risk_for_provider(rule.provider),
                    description=rule.description,
                ))
        # Layer 2: Entropy (only if no regex match on this line)
        if not any(f.line_number == line_idx for f in findings):
            pattern = re.compile(
                r'(?i)(?:key|secret|token|password|credential|auth)\s*[:=]\s*[\'"]([^\'"]{16,})[\'"]'
            )
            for match in pattern.finditer(line):
                value = match.group(1)
                if _shannon_entropy(value) > 4.5:
                    findings.append(SecretFinding(
                        rule_id="entropy-high", provider="generic", type="api_key",
                        file_path=file_path, line_number=line_idx,
                        fingerprint=_fingerprint(value), risk_level="high",
                        description=f"High-entropy string (entropy={_shannon_entropy(value):.1f})",
                    ))
    return findings
