# Sideload → public plugin migration

If you installed the Forge plugin from the private `forge-app` repo **before
2026-04-23** (when the ban on `forge:*` skills was lifted and the plugin moved
into the public repo), you need to move your installation onto the public
source. This guide covers the switch.

**TL;DR:** remove the private plugin entry from `~/.claude/settings.json`,
add the public marketplace, restart Claude Code. Your daemon, its DB
(`~/.forge/forge.db`), and your session/skill history are untouched — only
the plugin-surface wiring moves.

## Who this is for

- You enabled a plugin named `forge@forge-app-marketplace` or similar.
- Your `~/.claude/plugins/marketplaces/` has an entry pointing at a
  directory inside `forge-app/` (or a GitHub clone of that private repo).
- You see `forge-app` mentioned in any `~/.claude/settings.json` under
  `enabledPlugins` or `extraKnownMarketplaces`.

If none of those match, you either never sideloaded or you are already on
the public plugin — no action needed.

## Step 1 — snapshot current state

```bash
jq '.enabledPlugins, .extraKnownMarketplaces' ~/.claude/settings.json \
  > ~/forge-plugin-backup.json
cp ~/.claude/settings.json ~/.claude/settings.json.pre-sideload-migration
```

## Step 2 — remove the private plugin entry

Edit `~/.claude/settings.json`:

```jsonc
{
  "enabledPlugins": {
    // remove any line like:
    //   "forge@forge-app-marketplace": true
    //   "forge@forge-private": true
  },
  "extraKnownMarketplaces": {
    // remove any block whose source.path points inside forge-app/
  }
}
```

## Step 3 — add the public marketplace

Option A — **install from the public repo on disk** (recommended for
contributors):

```jsonc
{
  "enabledPlugins": {
    "forge@forge-marketplace": true
  },
  "extraKnownMarketplaces": {
    "forge-marketplace": {
      "source": { "source": "directory", "path": "/absolute/path/to/public/forge" },
      "autoUpdate": false
    }
  }
}
```

Also create a symlink so Claude Code finds the marketplace:

```bash
ln -sfn /absolute/path/to/public/forge \
  ~/.claude/plugins/marketplaces/forge-marketplace
```

Option B — **install from GitHub directly** (for users who don't work on the
Forge codebase):

```jsonc
{
  "enabledPlugins": {
    "forge@forge-marketplace": true
  },
  "extraKnownMarketplaces": {
    "forge-marketplace": {
      "source": { "source": "github", "repo": "chaosmaximus/forge" }
    }
  }
}
```

## Step 4 — restart Claude Code

Close and reopen. The `/reload-plugins` slash command is insufficient here —
the marketplace registry is read at startup.

Verify with `/doctor`:

- `plugin-root: forge` should resolve to the public path.
- `hooks` check should be OK (the fix for the hook schema bug from
  2026-04-24 landed in the public plugin; the private sideload never
  had it).

## Step 5 — clean up the private marketplace (optional)

Once Step 4 works, you can remove the stale symlink / cached data:

```bash
rm -f ~/.claude/plugins/marketplaces/forge-app-marketplace
rm -rf ~/.claude/plugins/cache/forge-app-marketplace
```

Do **not** touch `~/.forge/` — that's your daemon's memory, unrelated to
the plugin install path.

## What changed between sideload and public

- **Hook schema fix** (`d660562`, 2026-04-24) — every hook that emits
  `hookSpecificOutput` now includes the required `hookEventName`. Without
  this, `additionalContext` was silently dropped on every invocation and
  you saw "Hook JSON output validation failed" non-blocking errors.
- **Daemon-side Hook HealthCheck** (2P-1a T6a, `b48991e`) — `/doctor` now
  probes the plugin's `hooks.json` presence instead of claiming OK
  unconditionally.
- **Phase 23 Behavioral Skill Inference** (2A-4c2, `a48a3c4`) is NEW in
  the daemon. Updating the plugin alone won't surface it — rebuild the
  daemon at or above `90d9b74`.

## Troubleshooting

- **`/reload-plugins` still shows the old name.** Remove the marketplace
  entry in step 2 completely (not just disable it), restart Claude Code.
- **`/doctor` says `plugin-root: /not-found`.** The symlink in step 3 is
  missing or the `source.path` in `extraKnownMarketplaces` is wrong.
- **HUD is blank.** `~/.claude/statusline-command.sh` may still point at
  an old `forge-hud` binary. Update to
  `/absolute/path/to/public/forge/target/release/forge-hud`.
