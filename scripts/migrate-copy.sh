#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# migrate-copy.sh — Phase 2P-1a migration orchestrator.
#
# Copies the agent-facing plugin surface from the frozen private
# forge-app SHA into the public forge repo, applies license retargets,
# adds SPDX headers where safe, emits restoration artifacts, and runs
# the scrub gate. See:
#   - docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md
#     §2.1 / §2.2 / §2.3 / §2.4 and Task T3a of §4
#   - docs/superpowers/plans/2P-1a-inventory.md §1 / §2 MIGRATE rows
#
# Usage:
#   scripts/migrate-copy.sh --dry-run      (default; no side effects)
#   scripts/migrate-copy.sh --apply        (writes files; reverts on scrub fail)
#   scripts/migrate-copy.sh --help
#
# Exit codes:
#   0 on success (dry-run or apply),
#   1 on scrub failure,
#   2 on precondition failure (SHA drift, missing inputs, wrong args).

set -euo pipefail

# ---------------------------------------------------------------------------
# Constants — edit only when the inventory re-freezes.
# ---------------------------------------------------------------------------

FROZEN_SHA="480527b57c01aeed4052db13ed07c9140302786b"
SOURCE_DIR="/mnt/colab-disk/DurgaSaiK/forge/forge-app"

# ---------------------------------------------------------------------------
# Inventory §1/§2 MIGRATE rows — canonical copy list.
#
# Each entry is a path RELATIVE to forge-app root. Directories end with "/"
# so rsync recurses them; files are copied verbatim. Inventory §5 describes
# the forge-app post-prune allowlist (informative for T5a, not used here).
# ---------------------------------------------------------------------------

MIGRATE_PATHS=(
    # --- Claude Code plugin manifest layer ---
    ".claude-plugin/"
    # --- Agent teams ---
    "agents/"
    # --- Hook layer ---
    "hooks/"
    # --- Skills (all 15 subdirs + forge-build-workflow.md shared ref) ---
    "skills/"
    # --- Homebrew formula ---
    "Formula/"
    # --- Plugin templates ---
    "templates/"
    # --- Hook shell scripts (11 files) ---
    "scripts/hooks/post-bash.sh"
    "scripts/hooks/post-compact.sh"
    "scripts/hooks/post-edit.sh"
    "scripts/hooks/pre-bash.sh"
    "scripts/hooks/pre-edit.sh"
    "scripts/hooks/session-end.sh"
    "scripts/hooks/session-start.sh"
    "scripts/hooks/stop.sh"
    "scripts/hooks/subagent-start.sh"
    "scripts/hooks/task-completed.sh"
    "scripts/hooks/user-prompt.sh"
    # --- Non-hook scripts (3 files; inventory §1) ---
    "scripts/post-edit-enhanced.sh"
    "scripts/protect-sensitive-files.sh"
    "scripts/task-completed-gate.sh"
    # --- Tests (4 subdirs + orchestrator + adversarial prompt) ---
    "tests/unit/"
    "tests/integration/"
    "tests/static/"
    "tests/claude-code/"
    "tests/run-all.sh"
    "tests/codex-adversarial-prompt.md"
)

# ---------------------------------------------------------------------------
# Derived / internal.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXCLUDE_FILE="$SCRIPT_DIR/migrate-exclude.txt"
LEXICON_FILE="$SCRIPT_DIR/migrate-lexicon.txt"
SCRUB_SCRIPT="$SCRIPT_DIR/migrate-scrub.sh"
PATCH_DIR="$SCRIPT_DIR"

MODE="dry-run"
COPIED_COUNT=0
RETARGET_COUNT=0
SPDX_ADDED_COUNT=0
LEDGER=""   # path to ledger file (apply mode only)

# ---------------------------------------------------------------------------
# Logging.
# ---------------------------------------------------------------------------

log()  { printf '[migrate-copy] %s\n' "$*"; }
warn() { printf '[migrate-copy] WARN: %s\n' "$*" >&2; }
die()  { printf '[migrate-copy] ERROR: %s\n' "$*" >&2; exit "${2:-2}"; }
phase() { printf '\n[migrate-copy] === %s ===\n' "$*"; }

# ---------------------------------------------------------------------------
# Ledger helpers — apply-mode only. Records paths we touched so a failed
# scrub can roll them back. Dry-run NEVER writes a ledger.
# ---------------------------------------------------------------------------

ledger_init() {
    [ "$MODE" = "apply" ] || return 0
    LEDGER="$(mktemp "${TMPDIR:-/tmp}/migrate-copy-ledger.XXXXXX")"
    log "ledger: $LEDGER"
}

ledger_record() {
    [ "$MODE" = "apply" ] || return 0
    [ -n "$LEDGER" ] || return 0
    # Store absolute paths so revert is unambiguous even if cwd changes.
    printf '%s\n' "$1" >> "$LEDGER"
}

ledger_revert() {
    [ "$MODE" = "apply" ] || return 0
    [ -n "$LEDGER" ] || return 0
    [ -f "$LEDGER" ] || return 0
    warn "Reverting touched paths from ledger..."
    # Reverse order so children delete before parents.
    local path
    while IFS= read -r path; do
        if [ -e "$path" ] || [ -L "$path" ]; then
            rm -rf -- "$path" 2>/dev/null || true
        fi
    done < <(tac "$LEDGER")
    warn "Revert complete. Workspace restored to pre-copy state (best-effort)."
    rm -f -- "$LEDGER"
}

# ---------------------------------------------------------------------------
# CLI parsing.
# ---------------------------------------------------------------------------

usage() {
    cat <<'EOF'
migrate-copy.sh — Phase 2P-1a migration orchestrator

USAGE
  scripts/migrate-copy.sh --dry-run    (default; prints the plan, no writes)
  scripts/migrate-copy.sh --apply      (writes files; reverts on scrub fail)
  scripts/migrate-copy.sh --help       (this message)

WHAT IT DOES
  1. Verifies forge-app HEAD matches the frozen SHA pinned in this script.
  2. rsync-copies the MIGRATE-row paths from the inventory into the public repo.
  3. Applies license retargets (plugin.json, marketplace.json, Formula/forge.rb).
  4. Adds SPDX headers to migrated .sh and .md files that lack one (not .json).
  5. Emits the four §2.4 restoration artifacts (install.sh, getting-started
     section, + two .patch stubs for the Rust-side changes).
  6. Runs scripts/migrate-scrub.sh on the public repo root. Apply-mode reverts
     on scrub failure using a ledger of touched paths.

EXIT CODES
  0  success (dry-run or apply)
  1  scrub failure
  2  precondition failure (SHA drift, missing inputs, wrong args)
EOF
}

if [ "$#" -eq 0 ]; then
    usage
    exit 2
fi

case "${1:-}" in
    --dry-run) MODE="dry-run" ;;
    --apply)   MODE="apply" ;;
    --help|-h) usage; exit 0 ;;
    *) usage; die "unknown arg: ${1:-}"; ;;
esac

log "mode: $MODE"
log "source: $SOURCE_DIR @ $FROZEN_SHA"
log "target: $REPO_ROOT"

# ---------------------------------------------------------------------------
# Preconditions.
# ---------------------------------------------------------------------------

phase "preconditions"

[ -d "$SOURCE_DIR" ]    || die "source dir missing: $SOURCE_DIR"
[ -d "$REPO_ROOT" ]     || die "target repo missing: $REPO_ROOT"
[ -f "$EXCLUDE_FILE" ]  || die "exclude file missing: $EXCLUDE_FILE"
[ -f "$LEXICON_FILE" ]  || die "lexicon missing: $LEXICON_FILE"
[ -x "$SCRUB_SCRIPT" ]  || die "scrub script missing / not executable: $SCRUB_SCRIPT"
command -v rsync >/dev/null 2>&1 || die "rsync not on PATH"
command -v git   >/dev/null 2>&1 || die "git not on PATH"

# Confirm forge-app HEAD matches the frozen SHA.
actual_sha="$(git -C "$SOURCE_DIR" rev-parse HEAD 2>/dev/null || true)"
if [ -z "$actual_sha" ]; then
    die "could not read forge-app HEAD at $SOURCE_DIR"
fi
if [ "$actual_sha" != "$FROZEN_SHA" ]; then
    die "forge-app HEAD drifted: expected $FROZEN_SHA, got $actual_sha"
fi
log "forge-app HEAD matches frozen SHA: OK"

# Confirm every MIGRATE source path exists at the frozen SHA.
missing=0
for p in "${MIGRATE_PATHS[@]}"; do
    if [ ! -e "$SOURCE_DIR/$p" ]; then
        warn "inventory path missing in source: $p"
        missing=$((missing + 1))
    fi
done
[ "$missing" -eq 0 ] || die "$missing inventory path(s) missing in source"
log "all ${#MIGRATE_PATHS[@]} MIGRATE path(s) present in source: OK"

# Set up ledger (apply only).
ledger_init

# ---------------------------------------------------------------------------
# Phase 1: rsync MIGRATE paths into the public repo.
# ---------------------------------------------------------------------------

phase "phase 1: copy (rsync)"

RSYNC_BASE=(rsync -a --delete-excluded --exclude-from="$EXCLUDE_FILE")
if [ "$MODE" = "dry-run" ]; then
    # -n = dry-run; --itemize-changes gives a one-line diff per file.
    RSYNC_BASE+=(-n --itemize-changes)
fi

# Per-path whitelist walk. For each MIGRATE path we invoke rsync with a
# narrow source / destination pair rather than copying the whole tree and
# relying on excludes. This keeps the copy set intention-faithful even if
# forge-app grows new top-level files later.
for p in "${MIGRATE_PATHS[@]}"; do
    src="$SOURCE_DIR/$p"
    dst="$REPO_ROOT/$p"

    # For dir entries (trailing /) rsync needs matching trailing / on dst.
    # For file entries ensure parent dir exists first.
    if [[ "$p" == */ ]]; then
        if [ "$MODE" = "apply" ]; then
            mkdir -p -- "$dst"
            ledger_record "$dst"
        fi
    else
        dst_parent="$(dirname -- "$dst")"
        if [ "$MODE" = "apply" ]; then
            mkdir -p -- "$dst_parent"
            ledger_record "$dst"
        fi
    fi

    log "copy: $p"
    if [ "$MODE" = "apply" ]; then
        "${RSYNC_BASE[@]}" -- "$src" "$dst"
    else
        # Dry-run: capture itemized output for visibility + counting.
        "${RSYNC_BASE[@]}" -- "$src" "$dst" >/dev/null 2>&1 || true
    fi
done

# Count copied regular files under the MIGRATE paths in the target.
for p in "${MIGRATE_PATHS[@]}"; do
    path_target="$REPO_ROOT/$p"
    if [ -d "$path_target" ]; then
        n=$(find "$path_target" -type f 2>/dev/null | wc -l)
        COPIED_COUNT=$((COPIED_COUNT + n))
    elif [ -f "$path_target" ]; then
        COPIED_COUNT=$((COPIED_COUNT + 1))
    elif [ "$MODE" = "dry-run" ]; then
        # Predict count from source.
        if [ -d "$SOURCE_DIR/$p" ]; then
            n=$(find "$SOURCE_DIR/$p" -type f 2>/dev/null | wc -l)
            COPIED_COUNT=$((COPIED_COUNT + n))
        else
            COPIED_COUNT=$((COPIED_COUNT + 1))
        fi
    fi
done

log "files in copy set: $COPIED_COUNT"

# ---------------------------------------------------------------------------
# Phase 2: license retargets (spec §2.3).
#
# We operate on files in the target path whenever possible (apply mode); in
# dry-run we simulate and count hits against the source so the report is
# representative.
# ---------------------------------------------------------------------------

phase "phase 2: license retarget"

retarget_plugin_json() {
    # In plugin.json:
    #   "license": "Proprietary"  -> "Apache-2.0"
    #   "author": { "name": "Bhairavi Tech" } -> { "name": "Forge Contributors" }
    #   "homepage": ".bhairavi.tech..."       -> github releases URL
    local f="$1"
    [ -f "$f" ] || return 0

    local touched=0
    if grep -q '"license"[[:space:]]*:[[:space:]]*"Proprietary"' "$f"; then touched=1; fi
    if grep -q 'Bhairavi Tech' "$f"; then touched=1; fi
    if grep -q 'forge\.bhairavi\.tech' "$f"; then touched=1; fi
    if grep -q 'bhairavi\.tech' "$f"; then touched=1; fi

    if [ "$touched" -eq 0 ]; then return 0; fi

    if [ "$MODE" = "apply" ]; then
        # Use sed with safe delimiters. Order matters: replace more-specific
        # patterns before shorter fragments so we don't double-rewrite.
        sed -i \
            -e 's|"license"[[:space:]]*:[[:space:]]*"Proprietary"|"license": "Apache-2.0"|g' \
            -e 's|"name"[[:space:]]*:[[:space:]]*"Bhairavi Tech"|"name": "Forge Contributors"|g' \
            -e 's|"email"[[:space:]]*:[[:space:]]*"support@bhairavi\.tech"|"email": "noreply@forge.dev"|g' \
            -e 's|https://forge\.bhairavi\.tech|https://github.com/chaosmaximus/forge|g' \
            -e 's|forge\.bhairavi\.tech|github.com/chaosmaximus/forge|g' \
            "$f"
    fi
    RETARGET_COUNT=$((RETARGET_COUNT + 1))
    log "retarget: $(basename -- "$(dirname -- "$f")")/$(basename -- "$f")"
}

retarget_formula_rb() {
    local f="$1"
    [ -f "$f" ] || return 0
    if ! grep -q 'forge\.bhairavi\.tech' "$f" && ! grep -q 'Bhairavi Tech' "$f"; then
        return 0
    fi
    if [ "$MODE" = "apply" ]; then
        # Rewrite Homebrew download URLs to the public GitHub release host.
        # Ruby string interpolation (#{version}) stays intact — sed replaces
        # only the host segment.
        sed -i \
            -e 's|https://forge\.bhairavi\.tech/releases/|https://github.com/chaosmaximus/forge/releases/download/v#{version}/|g' \
            -e 's|forge\.bhairavi\.tech|github.com/chaosmaximus/forge|g' \
            -e 's|Bhairavi Tech|Forge Contributors|g' \
            "$f"
    fi
    RETARGET_COUNT=$((RETARGET_COUNT + 1))
    log "retarget: Formula/forge.rb"
}

if [ "$MODE" = "apply" ]; then
    retarget_plugin_json "$REPO_ROOT/.claude-plugin/plugin.json"
    retarget_plugin_json "$REPO_ROOT/.claude-plugin/marketplace.json"
    retarget_formula_rb  "$REPO_ROOT/Formula/forge.rb"
else
    # Dry-run: files haven't been copied yet — predict retargets from the
    # source tree so the summary count is representative.
    retarget_plugin_json "$SOURCE_DIR/.claude-plugin/plugin.json"
    retarget_plugin_json "$SOURCE_DIR/.claude-plugin/marketplace.json"
    retarget_formula_rb  "$SOURCE_DIR/Formula/forge.rb"
fi

log "license retargets applied: $RETARGET_COUNT"

# ---------------------------------------------------------------------------
# Phase 3: SPDX headers on .sh and .md (NOT .json — Codex v2 HIGH).
# ---------------------------------------------------------------------------

phase "phase 3: SPDX headers"

add_spdx_sh() {
    local f="$1"
    [ -f "$f" ] || return 0
    # Skip if already has SPDX.
    if grep -q 'SPDX-License-Identifier' "$f"; then return 0; fi
    SPDX_ADDED_COUNT=$((SPDX_ADDED_COUNT + 1))
    if [ "$MODE" = "dry-run" ]; then return 0; fi

    # Preserve shebang: if first line is `#!...`, insert SPDX as line 2;
    # else prepend at the very top.
    local first
    first="$(head -n 1 -- "$f" 2>/dev/null || true)"
    local tmp
    tmp="$(mktemp)"
    if [[ "$first" == "#!"* ]]; then
        {
            printf '%s\n' "$first"
            printf '# SPDX-License-Identifier: Apache-2.0\n'
            tail -n +2 -- "$f"
        } > "$tmp"
    else
        {
            printf '# SPDX-License-Identifier: Apache-2.0\n'
            cat -- "$f"
        } > "$tmp"
    fi
    # Preserve mode bits.
    chmod --reference="$f" "$tmp" 2>/dev/null || true
    mv -- "$tmp" "$f"
}

add_spdx_md() {
    local f="$1"
    [ -f "$f" ] || return 0
    if grep -q 'SPDX-License-Identifier' "$f"; then return 0; fi
    SPDX_ADDED_COUNT=$((SPDX_ADDED_COUNT + 1))
    if [ "$MODE" = "dry-run" ]; then return 0; fi

    local tmp
    tmp="$(mktemp)"
    {
        printf '<!-- SPDX-License-Identifier: Apache-2.0 -->\n'
        cat -- "$f"
    } > "$tmp"
    mv -- "$tmp" "$f"
}

# Walk only the MIGRATE paths in the target; never touch pre-existing repo files.
spdx_walk() {
    local rel="$1"
    local path_target="$REPO_ROOT/$rel"

    if [ -d "$path_target" ]; then
        while IFS= read -r -d '' f; do
            case "$f" in
                *.sh) add_spdx_sh "$f" ;;
                *.md) add_spdx_md "$f" ;;
            esac
        done < <(find "$path_target" -type f \( -name '*.sh' -o -name '*.md' \) -print0 2>/dev/null)
    elif [ -f "$path_target" ]; then
        case "$path_target" in
            *.sh) add_spdx_sh "$path_target" ;;
            *.md) add_spdx_md "$path_target" ;;
        esac
    elif [ "$MODE" = "dry-run" ]; then
        # In dry-run the files are not yet in the target; count against source.
        local path_source="$SOURCE_DIR/$rel"
        if [ -d "$path_source" ]; then
            while IFS= read -r -d '' f; do
                case "$f" in
                    *.sh|*.md)
                        if ! grep -q 'SPDX-License-Identifier' "$f"; then
                            SPDX_ADDED_COUNT=$((SPDX_ADDED_COUNT + 1))
                        fi
                        ;;
                esac
            done < <(find "$path_source" -type f \( -name '*.sh' -o -name '*.md' \) -print0 2>/dev/null)
        elif [ -f "$path_source" ]; then
            case "$path_source" in
                *.sh|*.md)
                    if ! grep -q 'SPDX-License-Identifier' "$path_source"; then
                        SPDX_ADDED_COUNT=$((SPDX_ADDED_COUNT + 1))
                    fi
                    ;;
            esac
        fi
    fi
}

for p in "${MIGRATE_PATHS[@]}"; do
    spdx_walk "$p"
done

log "SPDX headers added: $SPDX_ADDED_COUNT"

# ---------------------------------------------------------------------------
# Phase 4: restoration artifacts (spec §2.4).
#   4a. scripts/install.sh       — Linux-only public installer (restored).
#   4b. docs/getting-started.md  — append "Installing the Claude Code Plugin".
#   4c. scripts/restore-doctor-hook-check.patch       — T3a applies by hand.
#   4d. scripts/restore-test-hook-e2e.patch           — T3a applies by hand.
# ---------------------------------------------------------------------------

phase "phase 4: restoration artifacts"

INSTALL_SH="$REPO_ROOT/scripts/install.sh"
GETTING_STARTED="$REPO_ROOT/docs/getting-started.md"
PATCH_DOCTOR="$PATCH_DIR/restore-doctor-hook-check.patch"
PATCH_HOOK_E2E="$PATCH_DIR/restore-test-hook-e2e.patch"

write_install_sh() {
    log "restore: scripts/install.sh (Linux-only)"
    if [ "$MODE" = "dry-run" ]; then return 0; fi
    ledger_record "$INSTALL_SH"
    cat > "$INSTALL_SH" <<'INSTALL_EOF'
#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# install.sh — Linux x86_64 installer for Forge.
#
# Downloads the latest public release tarball from GitHub, extracts the
# binaries (forge-daemon, forge-next, forge, forge-hud), and installs them
# to ~/.local/bin. macOS support will ship in 2P-1b; this installer is
# explicitly Linux-only for 2P-1a (spec §6 acceptance).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/chaosmaximus/forge/master/scripts/install.sh | bash
#   # or
#   bash scripts/install.sh
set -euo pipefail

RELEASE_URL="https://github.com/chaosmaximus/forge/releases/latest/download/forge-linux-x86_64.tar.gz"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

die() { printf 'forge-install: %s\n' "$*" >&2; exit 1; }

os="$(uname -s 2>/dev/null || echo unknown)"
arch="$(uname -m 2>/dev/null || echo unknown)"

case "$os" in
    Linux) ;;
    Darwin)
        die "macOS installer ships in 2P-1b. For now: cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli"
        ;;
    *)
        die "unsupported OS: $os (Linux x86_64 only in 2P-1a)"
        ;;
esac
case "$arch" in
    x86_64|amd64) ;;
    *) die "unsupported arch: $arch (x86_64 only)";;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar  >/dev/null 2>&1 || die "tar is required"

mkdir -p -- "$INSTALL_DIR"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

printf 'forge-install: downloading %s\n' "$RELEASE_URL"
curl -fsSL --retry 3 "$RELEASE_URL" -o "$tmpdir/forge.tar.gz"

printf 'forge-install: extracting\n'
tar -xzf "$tmpdir/forge.tar.gz" -C "$tmpdir"

installed=0
for bin in forge-daemon forge-next forge forge-hud; do
    src="$(find "$tmpdir" -maxdepth 3 -type f -name "$bin" | head -n 1 || true)"
    if [ -n "$src" ]; then
        install -m 0755 "$src" "$INSTALL_DIR/$bin"
        printf '  installed: %s\n' "$INSTALL_DIR/$bin"
        installed=$((installed + 1))
    fi
done

if [ "$installed" -eq 0 ]; then
    die "no forge binaries found in release tarball"
fi

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) printf 'forge-install: add %s to PATH (e.g. export PATH="%s:$PATH")\n' "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac

printf 'forge-install: done (%s binaries installed)\n' "$installed"
INSTALL_EOF
    chmod +x "$INSTALL_SH"
}

append_getting_started() {
    local marker="## Installing the Claude Code Plugin"
    if [ -f "$GETTING_STARTED" ] && grep -qF "$marker" "$GETTING_STARTED"; then
        log "restore: docs/getting-started.md already has plugin section (skip)"
        return 0
    fi
    log "restore: docs/getting-started.md append plugin section"
    if [ "$MODE" = "dry-run" ]; then return 0; fi

    # Record target so revert can undo a newly-created file, but NOT a
    # pre-existing one (we'd lose the user's copy). For append-only we
    # instead stash the current contents in the ledger ourselves:
    if [ -f "$GETTING_STARTED" ]; then
        # Ledger only tracks deletion-on-revert. To safely roll back an
        # append, snapshot the file beside the ledger.
        local snapshot
        snapshot="${LEDGER}.snap.$$.${RANDOM}.getting-started.md"
        cp -p -- "$GETTING_STARTED" "$snapshot"
        printf 'RESTORE\t%s\t%s\n' "$snapshot" "$GETTING_STARTED" >> "${LEDGER}.restores" 2>/dev/null || true
    else
        ledger_record "$GETTING_STARTED"
        mkdir -p -- "$(dirname -- "$GETTING_STARTED")"
        : > "$GETTING_STARTED"
    fi

    cat >> "$GETTING_STARTED" <<'DOC_EOF'

## Installing the Claude Code Plugin

Forge ships a Claude Code plugin (manifest in `.claude-plugin/`) that registers
hooks, skills, and subagents so Claude Code sessions automatically register
with the running daemon, stream memory writes, and surface matching skills in
context.

### Option A: Symlink-install from a local clone (fastest for development)

```bash
git clone https://github.com/chaosmaximus/forge.git
mkdir -p ~/.claude/plugins
ln -snf "$PWD/forge" ~/.claude/plugins/forge
```

### Option B: Marketplace install

From a Claude Code session, invoke the plugin marketplace and install the
`forge` plugin. (Full marketplace publication lands in 2P-1b — until then use
Option A.)

### Verify hooks fire

Start the daemon in one terminal:

```bash
forge-daemon
```

Open a new Claude Code session. You should see the daemon log a
`register_session` entry within a few seconds — this confirms
`scripts/hooks/session-start.sh` executed. Ask Claude any question, then:

```bash
forge-next recall "<any keyword from your prompt>"
```

You should see at least one memory whose `session_id` matches the session you
just opened. If not, run `forge-next doctor` and check the "Hook" health row.
DOC_EOF
}

write_patch_doctor() {
    log "restore-stub: scripts/restore-doctor-hook-check.patch"
    if [ "$MODE" = "dry-run" ]; then return 0; fi
    ledger_record "$PATCH_DOCTOR"
    cat > "$PATCH_DOCTOR" <<'PATCH_EOF'
# SPDX-License-Identifier: Apache-2.0
#
# restore-doctor-hook-check.patch
#
# Phase 2P-1a T3a leaves Rust source edits to a human reviewer. This patch
# stub documents the `doctor` Hook-health check that the 2026-04-12 split
# removed from `crates/cli/src/commands/system.rs`. Apply with:
#
#   git apply scripts/restore-doctor-hook-check.patch
#
# The diff is intentionally narrow — it adds one additional entry to the
# `checks` section rendered by `doctor()` so `forge-next doctor` surfaces
# whether `~/.claude/plugins/forge` (or equivalent plugin install) has been
# seen by the daemon in the last N minutes.
#
# NOTE: the daemon side of the check already exists in the `Doctor` response's
# `checks` vector; this patch is purely CLI-side presentation. No new
# protocol variants, no new daemon workers.

--- a/crates/cli/src/commands/system.rs
+++ b/crates/cli/src/commands/system.rs
@@ -66,6 +66,13 @@ pub async fn doctor() {
             if !checks.is_empty() {
                 println!();
                 println!("Health Checks:");
+                // 2P-1a restoration: surface the Hook health row explicitly
+                // when it's present so users can tell if session-start.sh
+                // has registered within the daemon's freshness window.
+                if let Some(hook) = checks.iter().find(|c| c.name.eq_ignore_ascii_case("hook")) {
+                    let indicator = match hook.status.as_str() { "ok" => "[OK]", "warn" => "[WARN]", "error" => "[ERROR]", _ => "[?]" };
+                    println!("  {} Hook: {}", indicator, hook.message);
+                }
                 for check in &checks {
                     let indicator = match check.status.as_str() {
                         "ok" => "[OK]",
PATCH_EOF
}

write_patch_hook_e2e() {
    log "restore-stub: scripts/restore-test-hook-e2e.patch"
    if [ "$MODE" = "dry-run" ]; then return 0; fi
    ledger_record "$PATCH_HOOK_E2E"
    cat > "$PATCH_HOOK_E2E" <<'PATCH_EOF'
# SPDX-License-Identifier: Apache-2.0
#
# restore-test-hook-e2e.patch
#
# Phase 2P-1a T3a leaves the creation of new Rust test files to the human
# reviewer. This stub describes the intended shape of
# `crates/daemon/tests/test_hook_e2e.rs`, which the 2026-04-12 split
# deleted. Apply with:
#
#   git apply scripts/restore-test-hook-e2e.patch
#
# The test spins up a daemon on an ephemeral port, POSTs a synthesised
# `register_session` payload identical to what `scripts/hooks/session-start.sh`
# would emit, then verifies:
#   (a) the daemon responds 200 with a session id,
#   (b) a subsequent `recall` query with the matching `session_id` returns
#       at least one memory written via a `record` call bracketed by the
#       session window,
#   (c) the Doctor response's `checks` vector contains a "Hook" entry
#       with status "ok".
#
# Skeleton (not applied automatically — human review required):
#
#   #[tokio::test]
#   async fn hook_session_start_end_to_end() {
#       let daemon = spawn_test_daemon().await;
#       let session_id = daemon.register_session(session_start_payload()).await.unwrap();
#       daemon.record(memory_in_session(&session_id)).await.unwrap();
#       let hits = daemon.recall("<keyword>", Some(&session_id)).await.unwrap();
#       assert!(!hits.is_empty(), "recall should return memories for the session");
#       let doctor = daemon.doctor().await.unwrap();
#       assert!(doctor.checks.iter().any(|c| c.name.eq_ignore_ascii_case("hook") && c.status == "ok"));
#   }
PATCH_EOF
}

write_install_sh
append_getting_started
write_patch_doctor
write_patch_hook_e2e

# ---------------------------------------------------------------------------
# Phase 5: scrub gate.
# ---------------------------------------------------------------------------

phase "phase 5: scrub"

if [ "$MODE" = "dry-run" ]; then
    log "skipping scrub invocation in dry-run (target may be empty or stale)"
    scrub_rc=0
else
    # Scan only the migrated paths under REPO_ROOT — scanning the whole
    # workspace would flag legitimate pre-existing content (target/ build
    # artifacts, tests/fixtures/scrub/ deliberate leaks, .tools/ toolchain
    # downloads). Loop per-path, aggregate exit codes.
    scrub_rc=0
    for p in "${MIGRATE_PATHS[@]}"; do
        target="$REPO_ROOT/$p"
        [ -e "$target" ] || continue
        # For file targets, scan the containing dir non-recursively via a
        # temp wrapper; scrub script expects a dir. Cheapest: scan the
        # parent dir but restrict find depth via a small adapter here.
        if [ -d "$target" ]; then
            set +e
            "$SCRUB_SCRIPT" "$target"
            rc=$?
            set -e
        else
            # Single-file target: stage it into a tmpdir and scan that.
            stage_dir="$(mktemp -d)"
            # Preserve the relative name so error messages stay interpretable.
            cp --parents -- "$p" "$stage_dir/" 2>/dev/null || cp -- "$target" "$stage_dir/$(basename "$p")"
            set +e
            "$SCRUB_SCRIPT" "$stage_dir"
            rc=$?
            set -e
            rm -rf -- "$stage_dir"
        fi
        if [ "$rc" -ne 0 ]; then
            scrub_rc="$rc"
        fi
    done
fi

if [ "$scrub_rc" -ne 0 ]; then
    warn "============================================================"
    warn "SCRUB FAILED (exit $scrub_rc) — migration aborted"
    warn "============================================================"
    if [ "$MODE" = "apply" ]; then
        ledger_revert
        # Also revert file-snapshot restores recorded for append targets.
        if [ -f "${LEDGER}.restores" ]; then
            while IFS=$'\t' read -r _tag snap orig; do
                if [ -f "$snap" ] && [ -n "$orig" ]; then
                    cp -p -- "$snap" "$orig" || true
                    rm -f -- "$snap"
                fi
            done < "${LEDGER}.restores"
            rm -f -- "${LEDGER}.restores"
        fi
    fi
    exit 1
fi

# Clean up snapshot sidecars on success (apply mode).
if [ "$MODE" = "apply" ] && [ -f "${LEDGER}.restores" ]; then
    while IFS=$'\t' read -r _tag snap _orig; do
        [ -f "$snap" ] && rm -f -- "$snap" || true
    done < "${LEDGER}.restores"
    rm -f -- "${LEDGER}.restores"
fi

# ---------------------------------------------------------------------------
# Phase 6: summary.
# ---------------------------------------------------------------------------

phase "summary"

cat <<EOF
  mode:                 $MODE
  frozen SHA:           $FROZEN_SHA
  migrate paths:        ${#MIGRATE_PATHS[@]}
  files copied:         $COPIED_COUNT
  license retargets:    $RETARGET_COUNT
  SPDX headers added:   $SPDX_ADDED_COUNT
  scrub result:         $([ "$MODE" = "dry-run" ] && echo "SKIPPED (dry-run)" || echo "PASSED")
  restoration files:    install.sh, getting-started.md section, 2 .patch stubs
EOF

if [ "$MODE" = "dry-run" ]; then
    log "dry-run complete — re-run with --apply to write changes."
fi

exit 0
