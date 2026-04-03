# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.6.x (current) | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in Forge, please report it responsibly:

1. **Do NOT** open a public GitHub issue
2. Email: konurud@gmail.com or use GitHub's private vulnerability reporting
3. Include: description, steps to reproduce, potential impact
4. We will respond within 48 hours

## Security Architecture

### Daemon (forge-daemon)
- **Socket permissions:** Unix socket at `~/.forge/forge.sock` with `0600` permissions (owner-only)
- **Parameterized queries:** All SQLite operations use parameterized statements (no SQL injection)
- **Property validation:** Memory property keys validated against `^[A-Za-z_][A-Za-z0-9_]{0,63}$`
- **Secret scanning:** SHA256 fingerprints only — never stores actual secret values
- **Symlink defense:** `symlink_metadata()` verification before file operations
- **UTF-8 safe truncation:** Content truncated at valid UTF-8 boundaries
- **50MB file limit:** Prevents memory exhaustion from large inputs

### App (Tauri v2)
- **CSP:** `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'`
- **Socket validation:** `lstat` + `is_socket()` + UID ownership check before connecting to daemon
- **Shell whitelist:** PTY only spawns shells from `/bin/`, `/usr/bin/`, `/opt/homebrew/bin/`, `/nix/`
- **tmux path validation:** Same whitelist applied to tmux binary
- **Session name sanitization:** Prevents shell injection and flag injection in tmux session names
- **PTY cleanup:** kill → join reader thread → wait to reap — no zombie processes
- **IPC permissions:** Minimal Tauri capabilities (core, event, dialog, notification)
- **Disposed guard:** Async PTY setup checks for component unmount before registering listeners

### Hook Scripts
- `set -euo pipefail` on all scripts
- Session-start hook uses `forge-next compile-context` for proactive context injection
- Pre-edit hook runs guardrails check via daemon socket
- All file paths resolved via `readlink` to prevent symlink traversal

## Security Reviews

- **5 Codex (gpt-5.4) adversarial reviews** completed across the full codebase
- **2 superpowers (Claude Opus) adversarial reviews** completed
- All CRITICAL and HIGH findings fixed and verified
- 378 tests across the workspace, zero failures

## Data Protection

- **Local-first:** All data stays on the user's machine. No cloud services, no telemetry.
- **No secrets stored:** Memory extraction filters out credential-like content. Secret scanner uses SHA256 fingerprints.
- **Encrypted sync (planned):** P2P memory sync will use SSH tunnel transport with age encryption.
