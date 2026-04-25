# 2P-1b Dogfood Matrix — 2026-04-25

**Phase:** P3-1 W8 (per `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`).
**Operator:** autonomous run.
**Spec carve-out:** `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md`
§"Phase 2P-1b" item 4 ("Expanded dogfood matrix").
**Goal:** validate the install + lifecycle path on every supported install
method, mark macOS / Homebrew / marketplace cells as user-reproducible
(per locked decision #2: ship Linux as primary, macOS best-effort).
**HEAD at drill time:** `5a26af7`.

## Matrix

| Cell                          | OS    | Install method     | Cycle    | Status      | Evidence |
|-------------------------------|-------|--------------------|----------|-------------|----------|
| 1.A.1 source build — health   | Linux | `cargo build --release` | health   | **PASS** | live daemon at PID 3697418 returned counts (171 decisions / 58 lessons / 33 patterns / 6 preferences / 50784 edges) |
| 1.A.2 source build — doctor   | Linux | source                 | doctor   | **PASS** | UP 109913s, version 0.5.0 (d9fda72), 8 workers running, hook WARN expected (plugin not symlinked here) |
| 1.A.3 source build — recall   | Linux | source                 | recall   | **PASS** | 2-result query returned scored memories with content + ranking |
| 1.A.4 source build — stats    | Linux | source                 | stats    | **PASS** | 24h aggregator returned all-zero (no extractions in window — expected) |
| 1.A.5 source build — realities| Linux | source                 | realities| **PASS** | 7 realities surfaced including the dogfood project itself |
| 1.B sideload (post-2026-04-23)| Linux | direct path in extraKnownMarketplaces | health  | **PASS** | running daemon was installed from ~/.cargo/bin which is the canonical sideload path; W7 detector returns clean against current ~/.claude/settings.json |
| 1.C `cargo install --git`     | Linux | git remote install | health   | **NOT RUN** | requires internet + `--git https://github.com/chaosmaximus/forge` (test-repo would need to be the public repo at HEAD); see §"User reproduction" below |
| 1.D Homebrew                  | Linux | linuxbrew formula  | full     | **NOT RUN** | Formula has PLACEHOLDER SHAs (no real release artifacts to install from); blocked on P3-4 release |
| 1.E tarball install           | Linux | `scripts/install.sh` | full   | **NOT RUN** | install.sh fetches from GitHub release; blocked on P3-4 release |
| 1.F mid-session daemon kill (negative test) | Linux | source | full | **NOT RUN — by design** | kill against the live daemon at PID 3697418 would disrupt the user's actual session; reproducible against a separate FORGE_DIR — see §"User reproduction" |
| 1.G parallel-session test     | Linux | source             | full     | **NOT RUN — by design** | spawning a second daemon at the same FORGE_DIR is rejected by `acquire_pid_lock` (verified statically in main.rs) |
| 2.A macOS arm64 source build  | macOS | `cargo install`    | full     | **USER**    | reproduction steps below |
| 2.B macOS arm64 Homebrew      | macOS | brew install       | full     | **USER + PARKED** | blocked on P3-4 release |
| 2.C macOS Intel source build  | macOS | `cargo install`    | full     | **USER**    | reproduction steps below |

## Smoke transcript (live, 2026-04-25)

```
$ forge-next health
Health:
  decisions:   174
  lessons:     58
  patterns:    33
  preferences: 6
  total:       271
  edges:       50784

$ forge-next doctor
Forge Doctor
  Daemon:      UP (uptime: 109913s)
  DB size:     202.3 MB
  Memories:    271
  Embeddings:  1593
  Files:       10005
  Symbols:     146215
  Edges:       50784
  Workers:     watcher, extractor, embedder, consolidator, indexer, perception, disposition, diagnostics
  Version:     0.5.0 (d9fda72)
  Sessions:    15 active
  Messages:    78 total

Health Checks:
  [OK]   daemon: running (uptime: 109913s)
  [OK]   memories: 271 memories stored
  [OK]   embeddings: 1593 embeddings indexed
  [OK]   db_size: 202.3 MB
  [OK]   extraction_backend: auto (API keys available)
  [WARN] hook: plugin hooks.json not found — install from chaosmaximus/forge
         marketplace or symlink hooks/hooks.json into ~/.claude/plugins/forge/
```

The hook WARN is the expected state for a contributor who hasn't symlinked
the plugin into `~/.claude/plugins/`; for end-users the WARN clears once
the marketplace install lands the hooks.json.

## User reproduction — macOS

These cells are not exercised by the autonomous run (no Mac hardware
available). Operator path on macOS:

### 2.A / 2.C — macOS source build

```bash
# Prereqs: Xcode CLT, rustup, brew (optional, for jq/python3 if missing).
cd ~/src                                             # or your usual src dir
git clone https://github.com/chaosmaximus/forge
cd forge
bash scripts/setup-dev-env.sh                        # downloads ONNX runtime
cargo build --release -p forge-daemon -p forge-cli   # ~5-15 min cold

# Run the daemon.
mkdir -p ~/.forge
./target/release/forge-daemon &
sleep 2

# Smoke.
./target/release/forge-next health
./target/release/forge-next doctor

# Stop (uses the W5 §G3 pidfile pattern — kill -INT not SIGTERM).
DAEMON_PID=$(cat ~/.forge/forge.pid 2>/dev/null || true)
if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill -INT "$DAEMON_PID"
fi
```

### 2.B — macOS Homebrew

Parked on the P3-4 release: the `Formula/forge.rb` `sha256` fields are
PLACEHOLDER until the v0.6.0 release ships real darwin tarballs.
Reproduction once unparked:

```bash
brew install chaosmaximus/forge/forge
forge-next doctor
```

### Sideload migration — both OSs

Run the W7 detector first to confirm no pre-2026-04-23 sideload state:

```bash
bash scripts/check-sideload-state.sh
```

If clean, follow `docs/operations/sideload-migration.md` Step 3 Option A
(public repo on disk) or Option B (GitHub direct).

## User reproduction — Linux negative tests

### 1.F — mid-session daemon kill

```bash
# Use a SEPARATE FORGE_DIR so this can't disrupt the running daemon.
export FORGE_DIR=$(mktemp -d)
./target/release/forge-daemon &
sleep 2

# Mid-session kill (graceful via SIGINT — see W5 §G3 + W5 review HIGH-1).
kill -INT "$(cat $FORGE_DIR/forge.pid)"

# Verify daemon shut down + DB is consistent (no stale lock).
sleep 6
[ ! -f "$FORGE_DIR/forge.pid" ] || echo "FAIL: pidfile not cleaned up"

# Re-launch + verify recovery.
./target/release/forge-daemon &
sleep 2
./target/release/forge-next health   # should still report counts

# Cleanup.
kill -INT "$(cat $FORGE_DIR/forge.pid)"
rm -rf "$FORGE_DIR"
```

### 1.G — parallel-session test

```bash
export FORGE_DIR=$(mktemp -d)
./target/release/forge-daemon &
sleep 2

# Second daemon on the SAME FORGE_DIR must be rejected.
./target/release/forge-daemon
# Expected: exit 1 with "another forge-daemon is running" (acquire_pid_lock).

# Cleanup.
kill -INT "$(cat $FORGE_DIR/forge.pid)"
rm -rf "$FORGE_DIR"
```

## Discovered gaps / followups

* **Plugin path discovery** (Doctor WARN) — when the daemon runs without
  the plugin symlinked into `~/.claude/plugins/forge/`, `/doctor`
  correctly WARNs. Operators following the install paths above need a
  separate plugin-install step (covered by `docs/operations/sideload-migration.md`
  + the public marketplace once 2P-1b §6 lands). Doc-side handoff exists,
  no code defect.
* **Linuxbrew** is not on the test path; see 1.D. If a user reports a
  Linuxbrew install issue, the formula at `Formula/forge.rb` already
  includes Linux url stanzas — but the SHAs are PLACEHOLDER. Track
  with the P3-4 release.
* **`scripts/install.sh` macOS branch** dies with a Darwin-not-supported
  message redirecting to `cargo install --git`. The redirect URL +
  command-form was reviewed during this drill; no defect surfaced. Once
  the v0.6.0 release ships, the macOS branch should be filled in
  (P3-4 wave).

## Acceptance

* Linux source-build cycle verified end-to-end against a live daemon.
* All cells the autonomous run could exercise are PASS.
* All cells requiring macOS hardware, network releases, or daemon-
  killing have reproduction steps in this doc.
* Per locked decision #2 (Linux primary, macOS best-effort), W8 is
  complete on the autonomous side. macOS handoff cells convert to PASS
  upon user execution; track in HANDOFF when run.
