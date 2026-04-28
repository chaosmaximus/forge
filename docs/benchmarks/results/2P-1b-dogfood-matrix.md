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
| 1.A.1 source build — health   | Linux | `cargo build --release` | health   | **PASS** | live daemon at PID 3697418 returned counts (174 decisions / 58 lessons / 33 patterns / 6 preferences / 50784 edges; total 271) |
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

Two distinct cases produce the hook WARN:

1. **Contributor without symlink** — expected; clears once the developer
   symlinks the public repo into `~/.claude/plugins/forge/` (W7 doc).
2. **Marketplace-installed user with broken plugin shipment** — *not*
   expected; the marketplace plugin should ship `hooks/hooks.json` and
   `/doctor` should clear the WARN. If the WARN fires after a successful
   marketplace install, file an issue — the marketplace-supplied plugin
   is missing hooks.json (or the install path is misconfigured).

## User reproduction — macOS

These cells are not exercised by the autonomous run (no Mac hardware
available). Operator path on macOS:

### 2.A / 2.C — macOS source build

```bash
# Prereqs: Xcode CLT, rustup, brew (optional, for jq/python3 if missing).
cd ~/src                                             # or your usual src dir
git clone https://github.com/chaosmaximus/forge
cd forge

# scripts/setup-dev-env.sh is Linux-only (downloads manylinux ORT via
# apt-get). On macOS, skip it — `pyke/ort` resolves the system ORT
# automatically when cargo builds. If a build error names ORT, install
# `brew install onnxruntime` first.
# bash scripts/setup-dev-env.sh                       # Linux only

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
kill -INT "$(cat "$FORGE_DIR/forge.pid")"

# Verify daemon shut down + DB is consistent (no stale lock).
sleep 6
[ ! -f "$FORGE_DIR/forge.pid" ] || echo "FAIL: pidfile not cleaned up"

# Re-launch + verify recovery.
./target/release/forge-daemon &
sleep 2
./target/release/forge-next health   # should still report counts

# Cleanup.
kill -INT "$(cat "$FORGE_DIR/forge.pid")"
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
kill -INT "$(cat "$FORGE_DIR/forge.pid")"
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
* All 6 autonomous-runnable cells (1.A.1-5, 1.B) PASS.
* 8 cells handed off — 3 release-blocked (1.C, 1.D, 1.E), 2 by-design
  negative (1.F, 1.G — risky against the live user daemon), 3 macOS
  (2.A, 2.B, 2.C — no Mac hardware in autonomous run).
* All 8 handoff cells have full reproduction steps in this doc.
* Per locked decision #2 (Linux primary, macOS best-effort), W8 is
  complete on the autonomous side. Handoff cells convert to PASS upon
  user execution; track in HANDOFF when each runs.

---

## v0.6.0 release-stack re-verification — 2026-04-28

**Phase:** P3-4 #101 step 5 (release-stack Linux multi-method verify).
**HEAD:** `dccfbb3` (CHANGELOG narrative landed). Bumped from
`0.6.0-rc.3` → `0.6.0` at `58660fa`.
**Operator:** autonomous run. Fresh `cargo clean` → `cargo build
--release --workspace` → `cargo build --release -p forge-daemon
--features bench --bin forge-bench` → daemon SIGTERM-respawn at
new build → live dogfood.

| Cell                                    | OS    | Install method                          | Cycle   | Status   | Evidence |
|-----------------------------------------|-------|-----------------------------------------|---------|----------|----------|
| 1.A.1 source build — health             | Linux | `cargo build --release --workspace`     | health  | **PASS** | `forge-next health` returns counts: 0 decisions / 1 lesson / 0 patterns / 0 preferences / 2,955 edges. |
| 1.A.2 source build — doctor             | Linux | source                                  | doctor  | **PASS** | Daemon UP 45s; `Version: 0.6.0 (dccfbb3)`; 8 workers running (watcher, extractor, embedder, consolidator, indexer, perception, disposition, diagnostics); 4 active sessions; backup-hygiene WARN as expected (5 `*.bak`, 934 MB — F-LOW-4 v0.6.1 deferral). |
| 1.A.3 source build — manas-health       | Linux | source                                  | manas   | **PASS** | All 8 Manas layers report (Platform 5, Tool 52, Skill 1, Domain DNA 8, Experience 1, Perception 0, Declared 4, Latent 1) plus Ahankara (0 facets) + Disposition (4 traits). |
| 1.A.4 source build — observe envelope   | Linux | source                                  | observe | **PASS** | `forge-next observe --shape row-count` returns valid JSON envelope — `kind=inspect`, `shape=row_count`, `window=1h`, `effective_filter` populated, `effective_group_by`, `stale=false`, `truncated=false`, `row_count=11`. Confirms shape uniformity per W1.37. |
| 1.A.5 source build — `--version`        | Linux | source                                  | version | **PASS** | `forge-next 0.6.0 (dccfbb3)` and `forge-daemon 0.6.0 (dccfbb3)` and `forge-bench 0.6.0` — vergen short-SHA pinning consistent across binaries. |
| 1.A.6 SIGTERM graceful respawn          | Linux | source                                  | restart | **PASS** | `kill -TERM <pid>` on prior daemon → auto-spawn hook brought up new daemon at fresh `dccfbb3` build via `~/.local/bin/forge-daemon` symlink → ` ~/.forge/forge.pid` updated; socket re-bound; uptime counter restarts; no DB corruption. Confirms P3-2 W7 SIGTERM handler + F4 auto-spawn LD_LIBRARY_PATH propagation. |
| 1.A.7 forge-bench `--version`           | Linux | `cargo build --release --features bench` | version | **PASS** | Built separately via `cargo build --release -p forge-daemon --features bench --bin forge-bench` (12.1 MB binary). README documents this is the canonical install path for the bench harness — feature-gated to keep default `cargo install --git` lean. |
| 1.B sideload — symlink → target/release | Linux | `~/.local/bin → target/release` symlink | full    | **PASS** | `~/.local/bin/forge-{daemon,next}` symlinks resolve to fresh build; auto-spawn hook picks up symlink path; PID 3319616 running fresh `dccfbb3`. Sideload-via-symlink path is the canonical local-dev shape per `docs/operations/sideload-migration.md`. |

### Smoke transcript (live, 2026-04-28)

```
$ forge-next --version
forge-next 0.6.0 (dccfbb3)

$ forge-daemon --version
forge-daemon 0.6.0 (dccfbb3)

$ forge-bench --version
forge-bench 0.6.0

$ forge-next doctor
Forge Doctor
  Daemon:      UP (uptime: 45s)
  DB size:     62.4 MB
  Memories:    1
  Embeddings:  1
  Files:       …
  Symbols:     29
  Edges:       2955
  Workers:     watcher, extractor, embedder, consolidator, indexer,
               perception, disposition, diagnostics
  Version:     0.6.0 (dccfbb3)
  Sessions:    4 active
  Messages:    0 total

Health Checks:
  [OK]   daemon: running (uptime: 45s)
  [OK]   memories: 1 memories stored
  [OK]   embeddings: 1 embeddings indexed
  [OK]   db_size: 62.4 MB
  [WARN] backup_hygiene: 5 *.bak file(s), 934 MB in ~/.forge — …
  [OK]   extraction_backend: auto (API keys available)
  [OK]   hook: running outside a Claude Code plugin install …

$ forge-next manas-health
Manas 8-Layer Memory Health
───────────────────────────
Layer 1 (Platform):       5 entries
Layer 2 (Tool):          52 tools
Layer 3 (Skill):          1 skills
Layer 4 (Domain DNA):     8 patterns
Layer 5 (Experience):     1 memories
Layer 6 (Perception):     0 unconsumed
Layer 7 (Declared):       4 documents
Layer 8 (Latent):         1 embeddings
───────────────────────────
Ahankara (Identity):    0 facets
Disposition:            4 traits (caution, thoroughness)

$ forge-next observe --shape row-count | head -5
{
  "status": "ok",
  "data": {
    "kind": "inspect",
    "shape": "row_count",
    …
  }
}
```

### Sanity gates re-run after version bump

All 4 gates green at `dccfbb3`:

* `bash scripts/check-harness-sync.sh` — 158 JSON methods + 109 CLI
  subcommands, no drift.
* `bash scripts/check-protocol-hash.sh` — `0ad998ba944d…` (Request
  hash unchanged across the rc.3→0.6.0 bump as expected).
* `bash scripts/check-license-manifest.sh` — 3 file(s), coverage clean.
* `bash scripts/check-review-artifacts.sh` — 30 review(s) valid, no
  open blocking findings.

### Linux release-cycle acceptance for v0.6.0

* Source-build path (1.A.1-7) PASS end-to-end.
* Symlink-sideload path (1.B) PASS.
* SIGTERM-respawn path (1.A.6) PASS — proves daemon graceful shutdown
  + auto-spawn hook + RPATH bake-in all coexist.
* Release-mode test surface — 1,577 / 1,577 daemon lib tests pass
  (debug + release identical after `composite_score`
  `assert!`-promotion at `ab207f6`).
* Clippy: 0 warnings (workspace-wide).
* Bench-binary feature gate — verified `cargo build --release` (no
  flags) leaves `forge-bench` out, and `--features bench` produces
  it cleanly. Matches README §"Build From Source".

User-action handoff cells unchanged — 1.C / 1.D / 1.E remain blocked
on `gh release create v0.6.0` (per
`feedback_release_stack_deferred.md`). 2.A / 2.B / 2.C remain
user-handoff per locked decision #2.
