#!/usr/bin/env bash
# Forge — Install Script
# Downloads pre-built binaries and installs to ~/.local/bin
#
# Usage:
#   curl -fsSL https://forge.bhairavi.tech/install | sh
#
# Environment variables:
#   FORGE_VERSION   — specific version (default: latest)
#   FORGE_INSTALL    — install directory (default: ~/.local/bin)

set -euo pipefail

FORGE_VERSION="${FORGE_VERSION:-latest}"
FORGE_INSTALL="${FORGE_INSTALL:-${HOME}/.local/bin}"
FORGE_REPO="chaosmaximus/forge"
BASE_URL="https://github.com/${FORGE_REPO}/releases"

# --- Helpers ---

info()  { printf '\033[0;36m%s\033[0m\n' "$*"; }
warn()  { printf '\033[0;33m%s\033[0m\n' "$*" >&2; }
error() { printf '\033[0;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || error "required command not found: $1"
}

# --- Detect platform ---

detect_platform() {
  local os arch

  case "$(uname -s)" in
    Darwin)  os="apple-darwin" ;;
    Linux)   os="unknown-linux-gnu" ;;
    *)       error "unsupported OS: $(uname -s)" ;;
  esac

  case "$(uname -m)" in
    x86_64|amd64)  arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)             error "unsupported architecture: $(uname -m)" ;;
  esac

  echo "${arch}-${os}"
}

# --- Resolve version ---

resolve_version() {
  if [ "$FORGE_VERSION" = "latest" ]; then
    need_cmd curl
    local url="${BASE_URL}/latest"
    # Follow redirect to get the actual version tag
    FORGE_VERSION=$(curl -fsSI -o /dev/null -w '%{redirect_url}' "$url" 2>/dev/null \
      | sed 's|.*/tag/||' || true)

    if [ -z "$FORGE_VERSION" ]; then
      # Fallback: try GitHub API
      need_cmd grep
      FORGE_VERSION=$(curl -fsSL "https://api.github.com/repos/${FORGE_REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')
    fi

    [ -z "$FORGE_VERSION" ] && error "could not determine latest version"
  fi

  info "Version: ${FORGE_VERSION}"
}

# --- Download and install ---

install() {
  local platform="$1"
  local archive="forge-${FORGE_VERSION}-${platform}.tar.gz"
  local url="${BASE_URL}/download/${FORGE_VERSION}/${archive}"
  local tmp

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  info "Downloading ${archive}..."
  need_cmd curl
  curl -fSL --progress-bar "$url" -o "${tmp}/${archive}" \
    || error "download failed — check that version ${FORGE_VERSION} exists for ${platform}"

  info "Extracting..."
  need_cmd tar
  tar xzf "${tmp}/${archive}" -C "$tmp"

  info "Installing to ${FORGE_INSTALL}..."
  mkdir -p "$FORGE_INSTALL"

  for bin in forge-daemon forge-next; do
    if [ -f "${tmp}/${bin}" ]; then
      cp "${tmp}/${bin}" "${FORGE_INSTALL}/${bin}"
      chmod +x "${FORGE_INSTALL}/${bin}"
    fi
  done

  # Verify
  if [ ! -x "${FORGE_INSTALL}/forge-next" ]; then
    error "installation failed — forge-next not found in archive"
  fi
}

# --- Ensure PATH ---

ensure_path() {
  if echo "$PATH" | grep -q "${FORGE_INSTALL}"; then
    return
  fi

  local shell_rc=""
  case "${SHELL:-}" in
    */zsh)  shell_rc="${HOME}/.zshrc" ;;
    */bash) shell_rc="${HOME}/.bashrc" ;;
    *)      shell_rc="${HOME}/.profile" ;;
  esac

  if [ -n "$shell_rc" ] && ! grep -q '.local/bin' "$shell_rc" 2>/dev/null; then
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$shell_rc"
    info "Added ~/.local/bin to PATH in ${shell_rc}"
  fi

  export PATH="${FORGE_INSTALL}:${PATH}"
}

# --- Main ---

main() {
  info "Installing Forge..."
  echo ""

  need_cmd uname
  local platform
  platform="$(detect_platform)"
  info "Platform: ${platform}"

  resolve_version
  install "$platform"
  ensure_path

  echo ""
  info "Forge installed successfully!"
  echo ""
  echo "  forge-next recall \"search query\"     # search memories (auto-starts daemon)"
  echo "  forge-next remember --type decision --title \"...\" --content \"...\""
  echo "  forge-next health                    # memory layer counts"
  echo "  forge-next doctor                    # system diagnostics"
  echo ""
  echo "  The daemon auto-starts on first command. No manual setup needed."
  echo ""
}

main
