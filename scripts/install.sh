#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# install.sh — Linux x86_64 installer for Forge.
#
# Downloads the latest public release tarball from GitHub, extracts the
# binaries (forge-daemon, forge-next — 2 binaries as of 2P-1a; forge and
# forge-hud are daemon-internal and not shipped standalone), and installs
# them to ~/.local/bin. macOS support ships in 2P-1b; this installer is
# explicitly Linux-only for 2P-1a (spec §6 acceptance).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/chaosmaximus/forge/master/scripts/install.sh | bash
#   # or
#   bash scripts/install.sh
#
# Release tarball naming per .github/workflows/release.yml: the asset is
# `forge-<tag>-<target-triple>.tar.gz`, e.g. forge-v0.4.0-x86_64-unknown-linux-gnu.tar.gz.
# We resolve "latest" via the GitHub API to avoid hard-coding a version.
set -euo pipefail

REPO="chaosmaximus/forge"
TARGET_TRIPLE="x86_64-unknown-linux-gnu"
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

# Resolve the latest release tag, then compose the asset URL.
printf 'forge-install: resolving latest release for %s\n' "$REPO"
tag="$(curl -fsSL --retry 3 \
    "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -oE '"tag_name":\s*"[^"]+"' \
    | head -n 1 \
    | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"
[ -n "$tag" ] || die "could not resolve latest release tag from GitHub API"

ARCHIVE="forge-${tag}-${TARGET_TRIPLE}.tar.gz"
RELEASE_URL="https://github.com/$REPO/releases/download/${tag}/${ARCHIVE}"

printf 'forge-install: downloading %s\n' "$RELEASE_URL"
curl -fsSL --retry 3 "$RELEASE_URL" -o "$tmpdir/forge.tar.gz"

printf 'forge-install: extracting\n'
tar -xzf "$tmpdir/forge.tar.gz" -C "$tmpdir"

installed=0
for bin in forge-daemon forge-next; do
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
