# 2P-1 Rollback Playbook

How to unwind Phase 2P-1a (migration of plugin + hooks + skills + agents
from private `forge-app` into public `chaosmaximus/forge`) and Phase
2P-1b hardening, if a post-release regression makes it necessary.

**Audience:** release operator with commit access to `chaosmaximus/forge`,
maintainer access to the Homebrew tap, and GitHub Release delete rights.
**RTO target:** 20 minutes from decision to revoked release.
**Last tabletop exercise:** 2026-04-25 — see
`docs/operations/rollback-drills/2026-04-25-tabletop.md` for findings
(5 gaps surfaced; Step 4 pkill issue closed in this revision; G1, G4, G5
tracked as backlog).

---

## When to roll back

A rollback is correct when BOTH are true:

1. A regression reaches production users (binary installs via Homebrew,
   `cargo install`, or tarball downloads) — a PR-level bug that can be
   patched forward should be patched, not rolled back.
2. The regression's blast radius is larger than the cost of the rollback
   (users' DBs, hooks, sessions). Memory-corrupting schema migration =
   roll back. Cosmetic HUD bug = patch forward.

If either condition is missing, the playbook below is the wrong tool.
Fix forward and cut a new patch release.

---

## Step 0 — declare + broadcast (minute 0-2)

- File a tracking issue: `[rollback] v0.x.y — <one-line cause>`.
- Post to any channels users watch (release notes comment, X, Discord,
  etc.): "Forge vX.Y.Z rollback in progress, do not upgrade; existing
  installs on older versions are safe."
- Set `.github/pending-rollback` flag (branch protection hook — add a
  rule that refuses merges to master while this file exists). Commit
  the flag with message `rollback: block merges until vX.Y.Z revoked`.
  **Caveat (drill 2026-04-25 §G1):** the flag file alone is
  informational. Enforcement requires either (a) a GitHub branch
  protection rule rejecting merges while the file exists, or (b) a CI
  step that fails when the file is present. Option (b) is in-repo and
  recommended; not yet wired (P3-1 W5 backlog).

---

## Step 1 — revoke GitHub release + binaries (minute 2-5)

```bash
# Replace TAG with the version being rolled back.
TAG=v0.5.0

# Delete the release (binaries go with it). Default behavior keeps the
# tag intact so Git history and third-party tag references don't break;
# a later repush can be a new tag like v0.5.1.
gh release delete "$TAG" --yes

# If we DO want to delete the tag (e.g. nothing else will ever reference
# it), pass --cleanup-tag (or run the API + push commands explicitly):
# gh release delete "$TAG" --yes --cleanup-tag
# gh api --method DELETE "repos/chaosmaximus/forge/git/refs/tags/$TAG"
# git push origin --delete "$TAG"
```

**Why keep the tag by default:** download URLs referencing the tag still
404 (the release assets are what installers fetch), but Git history and
third-party docs referring to the tag don't break.

---

## Step 2 — revoke Homebrew bottle (minute 5-10)

If Formula/forge.rb was bumped to point at the bad release:

```bash
# In the public repo:
git revert -n <sha-of-formula-bump-commit>
git commit -m "revert: Formula/forge.rb rollback from $TAG"
git push origin master
```

Homebrew fetches `forge.rb` fresh on every `brew install`, so a revert
is enough. No bottle-revocation step unless the tap ships prebuilt
bottles (we don't today — installs compile from source via the URL +
SHA256 in the formula).

If the tap is a separate repository, open a revert PR there mirroring
the same change.

---

## Step 3 — revert public source (minute 10-15)

Two modes, pick one:

### Mode A — forward-revert (default, safer)

```bash
git checkout master
git revert --no-commit <bad-sha-range>
git commit -m "revert(2P-1): roll back $TAG due to <cause>"
git push origin master
```

Use when the bad commits are a contiguous range and a clean revert
applies without conflicts.

### Mode B — reset to a known-good SHA (destructive, rare)

Only if forward-revert introduces more risk than resetting — e.g., the
bad range is entangled with subsequent good commits. Coordinate with
every contributor first; a force-push will invalidate their local
branches.

```bash
git checkout master
git reset --hard <last-known-good-sha>
git push --force-with-lease origin master
```

**Never `--force` without `--force-with-lease`** — `with-lease` refuses
the push if someone else's commit landed since your last fetch.

---

## Step 4 — advise sideload users (minute 15-18)

Plugin users who installed via `extraKnownMarketplaces` pointing at a
public clone need explicit advisory — `/reload-plugins` against a rolled-
back repo picks up the revert, but a daemon that was upgraded to the bad
release remains on disk.

Post to the tracking issue:

```
Sideload users: pull + rebuild after the revert lands:

    cd /path/to/forge
    git fetch
    git reset --hard origin/master    # discards WIP — back up first
    cargo build --release --bin forge-daemon --bin forge-next

    # Restart daemon — use the pidfile written at $FORGE_DIR/forge.pid
    # (path computed by forge_core::default_pid_path; lock acquired via
    # main.rs::acquire_pid_lock) rather than `pkill -f 'forge-daemon'`.
    # The substring match is unsafe — drill 2026-04-25 §G3 showed it
    # false-matches any process whose cmdline mentions forge-daemon,
    # including shells whose cwd is a forge repo.
    #
    # NOTE: as of P3-2 W7 (v0.6.0-rc.2), the daemon also handles SIGTERM
    # via `tokio::signal::unix::signal(SignalKind::terminate())`, so
    # `kill PID` (default SIGTERM) and `kill -INT PID` (SIGINT) both
    # produce identical graceful socket-drain. We keep `kill -INT` here
    # for compatibility with v0.5.x and v0.6.0-rc.1 daemons that pre-date
    # the SIGTERM handler — it works on every released daemon. The
    # original §G6 SIGTERM-gap finding (P3-1 W5 review HIGH-1) is now
    # closed strategically.
    DAEMON_PID=$(cat "${FORGE_DIR:-$HOME/.forge}/forge.pid" 2>/dev/null || true)
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill -INT "$DAEMON_PID"
        for _ in 1 2 3 4 5; do
            kill -0 "$DAEMON_PID" 2>/dev/null || break
            sleep 1
        done
    else
        echo "no live daemon detected at $DAEMON_PID — relaunching fresh"
    fi
    /path/to/target/release/forge-daemon &
```

Private-plugin sideload users (pre-ban-lift) should follow
`docs/operations/sideload-migration.md` instead of reverting.

---

## Step 5 — post-mortem + close (minute 18-20)

- Tracking issue: link the revert commit + Homebrew revert PR +
  deleted-release URL.
- Remove `.github/pending-rollback` flag.
- Schedule a post-mortem (even if the cause is obvious — missed
  documentation is usually the real root cause).
- Update `HANDOFF.md` §Lifted constraints with the rollback entry AND
  the forward-fix plan.

---

## DB compatibility matrix

Rolling back the **daemon** binary but keeping the user's existing
`~/.forge/forge.db` only works if the schema is forward-compatible.

| From (bad) → To (rollback)         | Safe? | Reason |
|------------------------------------|-------|--------|
| v0.5.0 → v0.4.x                    | **NO** | 2A-4c2 T1 added 4 NOT NULL columns to `skill`; 2P-1b §17 renamed the partial unique index. v0.4.x won't understand them and may fail to start. |
| v0.5.x → v0.5.x-1 (patch rollback) | YES    | Same schema, no ALTERs added inter-patch. |
| v0.5.0 → v0.5.0-hotfix             | YES    | Hotfix keeps the v0.5.0 schema. |

If rolling back crosses a schema boundary, the runbook is different:
restore from the pre-migration backup documented in the daemon's
consolidator + startup logs (`[daemon] ingested N declared knowledge
files`). We do not currently auto-backup on migration — **2P-1b §5a
TODO: add pre-migration DB snapshot**.

---

## Tabletop exercise checklist

Two cadences:

- **Paper drill (quarterly):** read-only walkthrough against a scratch
  clone, syntax-check each command, log gaps. Latest: 2026-04-25 (W5).
- **Full drill (annual):** end-to-end including a fake tag/release on a
  test repo, brew/cargo install verification, dogfood-script run, RTO
  measurement under realistic operator load.

Per-drill items (apply both cadences unless noted):

- [ ] Read every step of this playbook in order. *(paper + full)*
- [ ] Syntax-check every shell command in a scratch clone. *(paper + full)*
- [ ] Time each step; record wallclock. *(paper + full)*
- [ ] Seed the clone with a fake v0.99.0 tag + release. *(full only)*
- [ ] Execute Step 1-3 against the clone end-to-end. *(full only)*
- [ ] Verify `brew install` (or `cargo install`) against the reverted
      state returns the pre-v0.99.0 binary. *(full only)*
- [ ] Run `docs/benchmarks/results/*` dogfood script on the reverted
      daemon to confirm end-to-end path still works. *(full only)*
- [ ] File dated log under `docs/operations/rollback-drills/` with
      timings + findings. *(paper + full)*
