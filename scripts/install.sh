#!/usr/bin/env bash
# Forge — Install script (plugin auto-install + developer build fallback)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
VERSION="${FORGE_VERSION:-latest}"
BASE_URL="https://forge.bhairavi.tech/releases"

mkdir -p "$BIN_DIR"

# Input validation
if [[ "$VERSION" =~ [^a-zA-Z0-9._-] ]]; then
  echo "[forge] Error: FORGE_VERSION contains invalid characters: $VERSION" >&2
  exit 1
fi

# Try downloading pre-built binary first
download_binary() {
  local OS_TAG ARCH_TAG TARGET ARCHIVE URL TMP_DIR

  case "$(uname -s)" in
    Darwin) OS_TAG="apple-darwin" ;;
    Linux)  OS_TAG="unknown-linux-gnu" ;;
    *)      return 1 ;;
  esac

  case "$(uname -m)" in
    arm64|aarch64) ARCH_TAG="aarch64" ;;
    x86_64|amd64)  ARCH_TAG="x86_64" ;;
    *)             return 1 ;;
  esac

  TARGET="${ARCH_TAG}-${OS_TAG}"
  ARCHIVE="forge-${VERSION}-${TARGET}.tar.gz"
  URL="${BASE_URL}/${ARCHIVE}"

  TMP_DIR=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$TMP_DIR'" RETURN

  echo "[forge] Downloading binaries (${TARGET})..."
  if command -v curl &>/dev/null; then
    curl -fsSL --retry 3 --retry-delay 2 "$URL" -o "${TMP_DIR}/forge.tar.gz" 2>/dev/null || return 1
  elif command -v wget &>/dev/null; then
    wget -q --tries=3 -O "${TMP_DIR}/forge.tar.gz" "$URL" 2>/dev/null || return 1
  else
    return 1
  fi

  # Restricted extraction — only expected files
  tar -xzf "${TMP_DIR}/forge.tar.gz" -C "$TMP_DIR" forge-daemon forge-next 2>/dev/null || \
    tar -xzf "${TMP_DIR}/forge.tar.gz" -C "$TMP_DIR" 2>/dev/null || return 1

  local found=0
  for bin in forge-daemon forge-next; do
    if [ -f "${TMP_DIR}/${bin}" ]; then
      mv "${TMP_DIR}/${bin}" "${BIN_DIR}/${bin}"
      chmod +x "${BIN_DIR}/${bin}"
      found=1
    fi
  done

  [ "$found" -eq 1 ] && return 0 || return 1
}

# Try download first, fall back to build from source
if download_binary; then
  echo "[forge] Installed from pre-built binary."
elif command -v cargo &>/dev/null && [ -f "${REPO_ROOT}/Cargo.toml" ]; then
  echo "[forge] Download failed. Building from source..."
  cd "$REPO_ROOT"
  cargo build --release -p forge-daemon -p forge-cli
  ln -sf "${REPO_ROOT}/target/release/forge-daemon" "${BIN_DIR}/forge-daemon"
  ln -sf "${REPO_ROOT}/target/release/forge-next" "${BIN_DIR}/forge-next"
  echo "[forge] Built from source."
else
  echo "[forge] Error: Could not download binary or find cargo. Install Rust or check internet." >&2
  exit 1
fi

# Ensure PATH includes ~/.local/bin
if ! echo "$PATH" | tr ':' '\n' | grep -Fqx "${BIN_DIR}"; then
  SHELL_RC="${HOME}/.bashrc"
  [ -f "${HOME}/.zshrc" ] && SHELL_RC="${HOME}/.zshrc"
  if ! grep -qF "/.local/bin" "$SHELL_RC" 2>/dev/null; then
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$SHELL_RC"
  fi
  export PATH="${BIN_DIR}:${PATH}"
fi

echo ""
echo "[forge] Installed:"
echo "  forge-daemon → ${BIN_DIR}/forge-daemon"
echo "  forge-next   → ${BIN_DIR}/forge-next"
echo ""
echo "[forge] The daemon auto-starts on first command."
