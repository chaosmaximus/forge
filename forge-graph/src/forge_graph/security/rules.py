"""Secret detection rules — regex patterns for known secret formats."""
import re
from dataclasses import dataclass


@dataclass(frozen=True)
class SecretRule:
    id: str
    provider: str
    type: str
    pattern: re.Pattern
    description: str


RULES: list[SecretRule] = [
    SecretRule("aws-access-key", "aws", "api_key",
               re.compile(r"(?:A3T[A-Z0-9]|AKIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ASIA)[A-Z0-9]{16}"),
               "AWS Access Key ID"),
    SecretRule("aws-secret-key", "aws", "api_key",
               re.compile(r'(?i)(?:aws)?_?(?:secret)?_?(?:access)?_?key[\'"]?\s*[:=]\s*[\'"]([A-Za-z0-9/+=]{40})[\'"]'),
               "AWS Secret Access Key"),
    SecretRule("github-pat", "github", "token",
               re.compile(r"ghp_[A-Za-z0-9]{36,}"), "GitHub PAT"),
    SecretRule("github-oauth", "github", "token",
               re.compile(r"gho_[A-Za-z0-9]{36,}"), "GitHub OAuth Token"),
    SecretRule("github-app", "github", "token",
               re.compile(r"ghs_[A-Za-z0-9]{36,}"), "GitHub App Token"),
    SecretRule("gcp-api-key", "gcp", "api_key",
               re.compile(r"AIza[A-Za-z0-9_-]{35}"), "GCP API Key"),
    SecretRule("stripe-secret", "stripe", "api_key",
               re.compile(r"sk_live_[A-Za-z0-9]{24,}"), "Stripe Secret Key"),
    SecretRule("private-key", "generic", "ssh_key",
               re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----"), "Private Key"),
    SecretRule("jwt-token", "generic", "token",
               re.compile(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"), "JWT"),
    SecretRule("generic-password", "generic", "password",
               re.compile(r'(?i)(?:password|passwd|pwd)\s*[:=]\s*[\'"][^\'"]{8,}[\'"]'), "Hardcoded Password"),
]
