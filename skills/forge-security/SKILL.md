---
name: forge-security
description: "Always-on security monitoring — secret scanning, rotation alerts, vulnerability detection. Use when user says 'scan for secrets', 'security audit', 'check for exposed credentials', 'run security scan', or before committing/shipping code that may contain sensitive data."
---

# Security Monitor

Continuous security monitoring for the workspace. Detects exposed secrets, tracks rotation, alerts on new findings.

## When to Use

- Automatically at session start (via hooks)
- User asks about security posture
- User asks to "scan", "check secrets", "security audit"
- Before any commit or PR

## Commands

### Quick scan
```bash
forge scan .
```
NDJSON findings. Zero LLM calls. Pure regex + entropy.

### Watch mode (always-on)
```bash
forge scan . --watch --interval 30
```
Continuously monitors. Reports new findings as they appear.

### Full audit (with graph)
Run `forge scan .` via CLI, which:
1. Scans all files (Rust, fast)
2. Stores findings as Secret nodes in the graph
3. Links to File nodes via LOCATED_IN edges

## Security Rules (12 patterns)

| Rule | Provider | Risk |
|------|----------|------|
| AWS Access Key (AKIA...) | aws | critical |
| AWS Secret Key | aws | critical |
| GitHub PAT (ghp_) | github | critical |
| GitHub OAuth (gho_) | github | critical |
| GitHub App (ghs_) | github | high |
| Stripe Secret (sk_live_) | stripe | critical |
| Stripe Publishable (pk_live_) | stripe | medium |
| Private Key (BEGIN PRIVATE KEY) | generic | critical |
| GCP API Key (AIza...) | gcp | high |
| Slack Token (xox...) | slack | high |
| JWT | generic | high |
| High-entropy near keyword | generic | medium |

## Principles

- NEVER store actual secret values — fingerprint only
- Skip symlinks to prevent workspace escape
- Respect .gitignore
- Max file size: 1MB
