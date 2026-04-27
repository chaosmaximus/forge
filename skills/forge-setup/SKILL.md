---
name: forge-setup
description: "Use on first run in a new project directory, or when Forge prerequisites need checking. Example triggers: 'set up Forge', 'check Forge prereqs', 'run Forge doctor', 'is Forge ready', 'initialize Forge', 'set up cc-voice'."
---

# Forge Setup

Get a new project bound to Forge in two minutes. Run through this in order; each step has a verify command.

## 1. Prerequisites

Claude Code version:
```bash
claude --version
```
Requires >= 2.1.32.

Forge binary on PATH:
```bash
forge-next --version
```
If `forge-next` is missing, install from the public repo. Note that the
crate name is `forge-cli` but the **binary** it produces is `forge-next`
— same for `forge-daemon` (crate `forge-daemon`, binary `forge-daemon`):

```bash
cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli
# Symlink onto PATH:
ln -sf ~/.cargo/bin/forge-next ~/.local/bin/forge-next
ln -sf ~/.cargo/bin/forge-daemon ~/.local/bin/forge-daemon
```

Daemon health:
```bash
forge-next health         # should print "ok"
forge-next doctor         # full system health
forge-next manas-health   # 8-layer memory health
forge-next identity       # agent identity (Ahankara)
```

If the daemon isn't running, the first `forge-next` invocation that needs
it will auto-spawn one. Verify with `pgrep -af forge-daemon`.

## 2. Bind this project to Forge (one-time)

Forge identifies what you're working on by **project name** — a stable
label you choose (e.g., `cc-voice`, `forge`, `my-app`). Each project has
its own indexed code graph, memories, and identity scope, so two projects
on the same daemon don't leak knowledge into each other.

Initialize the project:
```bash
forge-next project init cc-voice --path "$(pwd)"
# or with an explicit domain hint:
forge-next project init cc-voice --path "$(pwd)" --domain rust
```

Verify:
```bash
forge-next project show cc-voice
```

You should see the path, domain, and `Files indexed: 0` for a fresh
project. Once Claude Code starts indexing, the count climbs.

If you skip this step, the SessionStart hook will auto-create a project
record from the session's CWD on first contact. Explicit `init` is
preferred when you want to set the name yourself or pre-set the domain.

## 3. Ingest existing Claude memory (optional but high-value)

If you have a `~/.claude/projects/<project>/memory/` tree from prior
Claude Code sessions, import it into Forge so the agent recalls
previously-saved decisions and lessons:

```bash
forge-next ingest-claude
```

This reads the local Claude memory dir and seeds Forge's memory layers.
Per-project memories stay scoped to their project; global ones get the
`_global_` sentinel.

## 4. Trust-but-verify the SessionStart context

Before opening Claude Code, dry-run what Forge will inject into the
agent's first turn:

```bash
forge-next compile-context --project cc-voice --dry-run
```

The output is the literal `<forge-context>` XML the SessionStart hook
will inject. Check the `<code-structure>` tag — for a fresh project
you should see `resolution="no-match"` (or `resolution="auto-created"`
if you skipped step 2). If it shows `resolution="exact"` with a file
count from a different project, something's wrong — file an issue.

## 5. Companion plugins (check and recommend)

> Install commands below are based on standard Claude Code plugin syntax. If a command fails, check the plugin's documentation for updated install instructions.

| Plugin | Status | Purpose | Install Command |
|--------|--------|---------|----------------|
| codex-plugin-cc | [check] | Cross-model adversarial review | `/plugin marketplace add openai/codex-plugin-cc` |
| serena | [check] | LSP-grade code navigation | `/plugin install serena@claude-plugins-official` |
| context7 | [check] | Library documentation lookup | `/plugin install context7@claude-plugins-official` |
| playwright | [optional] | Browser E2E testing | `/plugin install playwright@claude-plugins-official` |

**Note:** Forge ships its own skills for TDD (`forge-tdd`), debugging
(`forge-debug`), verification (`forge-verify`), and memory
(`forge-next recall/remember`). The legacy `superpowers` and
`episodic-memory` plugins are no longer needed.

## 6. Production path configuration

If your project uses non-standard production directory names, customize
the patterns enforced by `forge-bash-check` (Codex hard gating):

Defaults: `infrastructure/**`, `terraform/**`, `k8s/**`, `helm/**`, `production/**`.

Common additions: `prod/**`, `deploy/**`, `live/**`, or project-specific
patterns. Set via `forge-next config set guardrails.production_paths
'["infrastructure/**", "deploy/**"]'`.

## 7. Project files

If no `CONSTITUTION.md` exists, offer to create one:
> "Want to set up a project constitution? This defines immutable
> principles (e.g., 'test-first', 'library-first', 'no raw SQL') that
> Forge enforces. Takes 2 minutes."

If yes: ask 3-5 questions, create from template.
If no: skip.

Create `STATE.md` with initial state.

## 8. Stitch MCP (optional)

Stitch MCP is **not bundled** with Forge. If you want visual-design
generation in `forge-new` Phase 5, install Stitch separately following
its docs and register it in your Claude Code MCP config. Otherwise skip
this step — `forge-new` detects absence and routes to the no-UI path.

## Done

"Forge is set up for `<project>`. Run `/forge:forge-new` for a new
project or `/forge:forge-feature` for existing code."

## Troubleshooting

### Context not injecting on session start

The Forge SessionStart hook intentionally swallows daemon errors so its
JSON output stays clean (Claude Code rejects non-JSON noise on the hook
channel). If `<forge-context>` is missing or `code-structure` shows
`resolution="no-match"` when you expect a real index, surface the
underlying failure with:

```bash
FORGE_HOOK_VERBOSE=1 claude   # or whatever invocation triggers your CC session
```

The flag is read by `scripts/hooks/session-start.sh`, `subagent-start.sh`,
and `post-edit.sh`. With it set, daemon stderr (socket missing,
`compile-context` errors, `register-session` timeouts) prints to your
terminal as `[forge-hook] ...` lines instead of being routed to
`/dev/null`. Once the hook works, unset the flag — the silent default
keeps day-to-day sessions clean.

### Hooks silently no-op

If you `cargo install forge-cli` and the binary lands as `forge-cli`
(crate name) instead of being symlinked as `forge-next`, the hooks
search `$FORGE_NEXT` → `~/.local/bin/forge-next` →
`/usr/local/bin/forge-next` and find nothing. Re-run the symlink step
in §1 above, or set `FORGE_NEXT=/path/to/forge-cli` in your shell rc.

## Reference: project commands cheat sheet

```bash
forge-next project init <name> [--path PATH] [--domain DOMAIN]   # one-time bind
forge-next project list                                          # what's tracked
forge-next project show <name>                                   # detail view
forge-next project detect [<path>]                               # introspect a path
forge-next compile-context --project <name> --dry-run            # preview agent context
forge-next ingest-claude                                         # import local Claude memory
```
