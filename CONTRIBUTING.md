# Contributing to Forge

## Development Setup

1. Clone the repo: `git clone https://github.com/DurgaSaiK/forge.git`
2. Install test dependencies: `sudo apt install jq shellcheck` and install BATS
3. Run tests: `bash tests/run-all.sh`
4. Test with Claude Code: `claude --plugin-dir /path/to/forge`
5. Validate plugin: `claude plugin validate /path/to/forge`

## Making Changes

1. Create a branch: `git checkout -b fix/description`
2. Make your changes
3. Run the full test suite: `bash tests/run-all.sh` -- all tests must pass
4. Run ShellCheck on any modified scripts: `shellcheck scripts/*.sh`
5. If modifying skills/agents, test with `claude --plugin-dir` to verify loading
6. Update CHANGELOG.md with your changes
7. Commit with semantic prefixes: `feat:`, `fix:`, `security:`, `test:`, `docs:`
8. Open a PR

## Architecture

- Skills define WHEN to trigger (descriptions) and HOW to work (body)
- Agents define WHO does the work (tools, model, constraints)
- Hooks enforce quality gates programmatically
- CSVs inject domain knowledge
- Templates provide starting structures

## Testing Layers

| Layer | What it tests | How to run |
|-------|-------------|------------|
| Static | JSON, YAML, CSV, ShellCheck | `bash tests/static/*.sh` |
| Unit (BATS) | Hook scripts with mocked stdin | `bats tests/unit/` |
| Integration | Plugin structure, hook behavior | `bash tests/integration/*.sh` |
| Claude Code | Skill invocation, agent spawning | `bash tests/claude-code/*.sh` |
| Codex | Adversarial security review | `codex exec --full-auto < tests/codex-adversarial-prompt.md` |
