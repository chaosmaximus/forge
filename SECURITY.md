# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in Forge, please report it responsibly:

1. **Do NOT** open a public GitHub issue
2. Email security concerns to [your email] or use GitHub's private vulnerability reporting
3. Include: description, steps to reproduce, potential impact
4. We will respond within 48 hours

## Security Architecture

### Hook Scripts
- All hook scripts use `set -euo pipefail`
- Sensitive file protection resolves symlinks via `readlink -f`
- Post-edit formatter validates file paths are within the workspace
- Session-end memory sync verifies JS file provenance
- Binary downloads include SHA256 checksum verification

### Agent Permissions
- Planner: read-only (Bash disallowed, Write/Edit disallowed)
- Generator: full access but isolated in git worktree
- Evaluator: read-only with Bash constrained to test execution

### Data Protection
- No secrets are stored by the plugin
- STATE.md/HANDOFF.md may contain project context -- documented warning for public repos
- MEMORY.md gitignore check before writing session notes
- Codex adversarial review sends code to OpenAI -- users should be aware

## Security Testing
- All scripts pass ShellCheck at warning severity
- Codex (GPT-5.4) adversarial security review performed on all hook scripts
- BATS unit tests cover sensitive file protection (10 test cases)
- Integration tests verify hook blocking behavior
