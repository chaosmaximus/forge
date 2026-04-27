# Local-plugin install — for cc-voice (and other early adopters)

Forge isn't on the Claude Code marketplace yet. Until it is, install
the plugin directly from the source tree at
`/mnt/colab-disk/DurgaSaiK/forge/forge` (or wherever you cloned it).

## 1. Build fresh binaries (one-time)

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
# First time only: download the manylinux_2_17 ONNX Runtime to .tools/
# (skip on macOS or glibc >= 2.38 hosts; harmless either way)
bash scripts/setup-dev-env.sh
cargo build --release -p forge-cli -p forge-daemon
ln -sf "$(pwd)/target/release/forge-next"   ~/.local/bin/forge-next
ln -sf "$(pwd)/target/release/forge-daemon" ~/.local/bin/forge-daemon
```

### Linux glibc < 2.38 (Ubuntu 22.04 LTS, Debian 12)

**Resolved by P3-4 Wave X (X2)**: the Linux release binary now bakes
DT_RUNPATH entries for the `.tools/` ONNX Runtime sidecar. As long as
you ran `bash scripts/setup-dev-env.sh` (which downloads the
manylinux_2_17 ORT to `.tools/`) and the binary lives at
`<workspace>/target/release/forge-daemon` (or symlinked from there),
the dynamic linker resolves `libonnxruntime.so.1` automatically — no
`LD_LIBRARY_PATH` shell-rc edit, no wrapper script needed.

Verify with:
```bash
readelf -d ~/.local/bin/forge-daemon | grep RUNPATH
# expect: $ORIGIN/../lib:$ORIGIN/../../.tools/...:$ORIGIN/../../../.tools/...
```

**Legacy escape hatch**: `scripts/with-ort.sh` still exists and still
prepends `LD_LIBRARY_PATH`. It's redundant with X2's RUNPATH for the
forge binaries, but it remains useful if you build a downstream
binary in this workspace that *doesn't* inherit the workspace's
`.cargo/config.toml` rustflags. Setting `LD_LIBRARY_PATH` manually
also still works — DT_RUNPATH (default for modern `ld --enable-new-dtags`)
is ranked BELOW `LD_LIBRARY_PATH`, so user overrides win.

macOS and glibc ≥ 2.38 Linux hosts use pyke's default ORT binary and
the `.tools/` directory remains unused; the RUNPATH entries point at
non-existent dirs and the loader skips them harmlessly.

### Verify

```bash
forge-next --version
forge-daemon --version
```

If you've installed Forge before, kill the old daemon so the new
binary takes over (the new one auto-spawns on first `forge-next`
call):

```bash
pgrep -af forge-daemon
kill -INT $(pgrep -f "target/release/forge-daemon")
forge-next health
```

## 2. Wire the plugin (two clean options)

### Option A — symlink into Claude Code's plugin dir (simplest)

```bash
ln -s /mnt/colab-disk/DurgaSaiK/forge/forge ~/.claude/plugins/forge
```

Restart Claude Code. The forge plugin should appear under
`/plugin list` with no errors. Skills, hooks, and agents auto-load
from the symlinked tree.

### Option B — local marketplace path

In Claude Code:

```
/plugin marketplace add /mnt/colab-disk/DurgaSaiK/forge/forge
/plugin install forge@local
```

## 3. Bind your project (per project — one-time)

For each project (`cc-voice`, `forge`, your own repos):

```bash
cd /path/to/your-project
forge-next project init my-project --path "$(pwd)"
forge-next project show my-project
```

If you skip this, `compile-context --cwd <path>` (which the
SessionStart hook now passes by default) auto-creates the project
record from CWD on first contact. Either way, future SessionStarts
in that directory bind cleanly without leaking another project's
index.

## 4. Trust-but-verify the agent's first turn

Before opening Claude Code in the project, dry-run what Forge will
inject:

```bash
forge-next compile-context --project my-project --dry-run
```

The output is the literal `<forge-context>` XML. Look for the
`<code-structure>` tag — possible outcomes:

```xml
<!-- Genuinely unknown project (no row in the reality table). -->
<code-structure project="my-project" resolution="no-match"/>

<!-- Project record exists (e.g. from `project init` or auto-create
     on a prior contact) but no files indexed yet. domain reflects
     whatever was detected (rust / python / typescript / unknown). -->
<code-structure project="my-project" domain="unknown" files="0" symbols="0" resolution="auto-created"/>

<!-- Project record exists AND files are indexed. -->
<code-structure project="my-project" domain="rust" files="42" symbols="120" resolution="exact">...</code-structure>
```

**`--dry-run` caveat (P3-4 Wave Y / Y2):** dry-run intentionally
skips the auto-create side effect, so a *truly* fresh project
(never seen by Forge) always renders `resolution="no-match"` under
`--dry-run`. Drop `--dry-run` for a one-shot run if you want
auto-create to fire and then verify with another `--dry-run` to see
`auto-created`. Or run `forge-next project init my-project --path
"$(pwd)"` explicitly first — both paths produce the same end-state.

If you see `resolution="exact"` with a file count from a *different*
project — that's the cc-voice §1.2 leak. File an issue (it shouldn't
happen at HEAD `3fcc1eb` or later).

## 5. Recover from a misregistered session

If SessionStart fired in the wrong dir (e.g. you started Claude in
the parent and `cd`'d into a subdir later), the session is bound to
the wrong project. Fix without restarting:

```bash
forge-next sessions
forge-next update-session --id <SESSION_ID> \
  --project my-project \
  --cwd "$(pwd)"
```

## 6. See what's going wrong (when something is)

```bash
FORGE_HOOK_VERBOSE=1 claude
forge-next doctor
forge-next manas-health
```

If `doctor` reports `[stale daemon, CLI built from <sha> — restart]`,
your daemon is older than the source tree. Rebuild + restart per
step 1 above.

## 7. Reference

* CLI cheat sheet: `docs/cli-reference.md`
* Setup skill: `skills/forge-setup/SKILL.md`
* CC voice's original feedback report: `feedback/2026-04-26-setup-and-isolation.md`
* Wave Z release notes: `CHANGELOG.md`
