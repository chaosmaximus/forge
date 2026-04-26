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

### Linux glibc < 2.38 caveat (Ubuntu 22.04 LTS, Debian 12)

The release binary links against `libonnxruntime.so.1` from `.tools/`
but does NOT bake an RPATH (planned fix; tracked as task #220). On
glibc < 2.38 you need to either:

**Option 1 — set `LD_LIBRARY_PATH` in your shell rc:**
```bash
echo 'export LD_LIBRARY_PATH="/mnt/colab-disk/DurgaSaiK/forge/forge/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"' >> ~/.bashrc
source ~/.bashrc
```

**Option 2 — wrap calls via `scripts/with-ort.sh`:**
```bash
alias forge-next="bash /mnt/colab-disk/DurgaSaiK/forge/forge/scripts/with-ort.sh forge-next"
```

macOS and glibc ≥ 2.38 hosts use pyke's default ORT binary and
don't need either workaround — the `.tools/` directory is unused.

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
`<code-structure>` tag — for a fresh project you should see one of:

```xml
<code-structure project="my-project" resolution="no-match"/>
<code-structure project="my-project" domain="rust" files="0" symbols="0" resolution="auto-created"/>
<code-structure project="my-project" domain="rust" files="42" symbols="120" resolution="exact">...</code-structure>
```

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
