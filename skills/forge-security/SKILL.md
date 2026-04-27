---
name: forge-security
description: "Manual secret-scanning playbook for the workspace — run grep-based detectors against the working tree and store findings as Forge memory. Use when user says 'scan for secrets', 'security audit', 'check for exposed credentials', or before committing/shipping code that may contain sensitive data."
---

# Forge Security — Workspace Secret Sweep

A manual checklist for spotting exposed credentials in the working tree
before they hit a commit. The Forge daemon does **not** ship a `forge
scan` subcommand; this skill is a structured grep-based workflow that
reuses Forge memory for tracking findings across sessions.

## When to Use

- User asks about security posture
- User asks to "scan", "check secrets", "security audit"
- Before any commit or PR that touches files outside docs/
- After importing a third-party library or copy-pasting code

## Step 1: Run the pattern grep

Run each pattern below against the working tree. Add `--include` filters
for the languages your project uses. Skip `target/`, `node_modules/`,
`dist/`, `.venv/`, `.git/`.

| Pattern | Risk |
|---------|------|
| `AKIA[0-9A-Z]{16}` (AWS Access Key) | critical |
| `aws_secret_access_key` (AWS Secret) | critical |
| `ghp_[A-Za-z0-9]{36}` (GitHub PAT) | critical |
| `gho_[A-Za-z0-9]{36}` (GitHub OAuth) | critical |
| `ghs_[A-Za-z0-9]{36}` (GitHub App token) | high |
| `sk_live_[A-Za-z0-9]{24,}` (Stripe Secret) | critical |
| `pk_live_[A-Za-z0-9]{24,}` (Stripe Publishable) | medium |
| `BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY` | critical |
| `AIza[0-9A-Za-z_-]{35}` (GCP API Key) | high |
| `xox[baprs]-` (Slack Token) | high |
| `eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}` (JWT) | high |
| High-entropy strings near keywords like `secret=`, `token=`, `apikey=` | medium |

Example sweep (Rust workspace):
```bash
git grep -EnI 'AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{36}|sk_live_[A-Za-z0-9]{24,}|BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY' \
  -- ':!target' ':!node_modules' ':!dist' ':!.venv'
```

## Step 2: Verify each hit

For each match:
- Is the value real or a placeholder/test fixture?
- Is the file in `.gitignore` already? (run `git check-ignore <path>`)
- Has it ever been committed? (`git log -p <path> | head -200`)

If a real secret has been committed: **rotate it immediately** — git
history rewrites are not a substitute for rotation.

## Step 3: Store findings as Forge memory

For confirmed real exposures, store a lesson so future sessions warn
the user:

```bash
forge-next remember --type lesson --title "Secret rotated: <provider>" \
  --content "<file>:<line> contained a <provider> credential committed at <SHA>. Rotated <date>. Add gitleaks/pre-commit hook before next commit."
```

For false-positives that keep tripping the grep, store a preference so
recall surfaces the carve-out:

```bash
forge-next remember --type preference --title "Test fixture: <pattern>" \
  --content "<file>:<line> contains a synthetic <pattern> for unit tests. Safe to ignore in future scans."
```

## Step 4: Recommend a pre-commit hook

A one-time grep is reactive. For ongoing protection, recommend the user
install one of:
- [gitleaks](https://github.com/gitleaks/gitleaks) (`brew install gitleaks`, configurable)
- [pre-commit](https://pre-commit.com/) with `detect-secrets` hook

Forge will flag known exposures, but does not block a commit on its
own — the pre-commit hook does.

## Principles

- NEVER copy a real secret value into a memory record — fingerprint
  only (provider + last 4 chars).
- Skip symlinks to prevent workspace escape.
- Respect `.gitignore`.
- Treat any committed-and-pushed secret as compromised regardless of
  history rewrite — rotate.
