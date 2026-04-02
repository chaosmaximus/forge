"""Tests for the secret scanner — regex + entropy detection."""
from forge_graph.security.scanner import scan_content, _fingerprint, _shannon_entropy


def test_detects_aws_access_key():
    content = 'aws_key = "AKIAIOSFODNN7EXAMPLE"'
    findings = scan_content(content, "config.py")
    assert len(findings) >= 1
    aws = [f for f in findings if f.rule_id == "aws-access-key"]
    assert len(aws) == 1
    assert aws[0].provider == "aws"
    assert aws[0].risk_level == "critical"


def test_detects_github_pat():
    content = 'token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkl"'
    findings = scan_content(content, "auth.py")
    assert len(findings) >= 1
    gh = [f for f in findings if f.rule_id == "github-pat"]
    assert len(gh) == 1
    assert gh[0].provider == "github"
    assert gh[0].type == "token"


def test_detects_private_key():
    content = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA..."
    findings = scan_content(content, "key.pem")
    assert len(findings) >= 1
    pk = [f for f in findings if f.rule_id == "private-key"]
    assert len(pk) == 1
    assert pk[0].type == "ssh_key"


def test_detects_high_entropy_string():
    # High-entropy value next to a secret-like key name
    content = 'api_secret = "aB3$kL9mXp2!qR7nYw4ZvC8jF6hT0sUe"'
    findings = scan_content(content, "settings.py")
    assert len(findings) >= 1
    entropy_findings = [f for f in findings if f.rule_id == "entropy-high"]
    assert len(entropy_findings) == 1
    assert entropy_findings[0].risk_level == "high"


def test_ignores_low_entropy_strings():
    content = 'name = "hello world this is normal text"'
    findings = scan_content(content, "readme.py")
    assert len(findings) == 0


def test_does_not_store_actual_secret():
    secret = "AKIAIOSFODNN7EXAMPLE"
    content = f'key = "{secret}"'
    findings = scan_content(content, "config.py")
    assert len(findings) >= 1
    for f in findings:
        # Fingerprint must NOT contain the actual secret
        assert secret not in f.fingerprint
        # Fingerprint is a 16-char hex string (SHA256 prefix)
        assert len(f.fingerprint) == 16
        assert all(c in "0123456789abcdef" for c in f.fingerprint)


def test_returns_line_numbers():
    content = "line1 = safe\nline2 = safe\ntoken = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkl\nline4 = safe"
    findings = scan_content(content, "multi.py")
    assert len(findings) >= 1
    gh = [f for f in findings if f.rule_id == "github-pat"]
    assert len(gh) == 1
    assert gh[0].line_number == 3
