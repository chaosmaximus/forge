#!/usr/bin/env bash
# setup-dev-env.sh — idempotent fresh-clone dev setup for forge-daemon on Linux.
#
# Downloads Microsoft's manylinux_2_17 ONNX Runtime build into `.tools/` so the
# workspace can link against it on glibc <2.38 hosts (see .cargo/config.toml).
# macOS and glibc ≥2.38 Linux hosts do not need this — the pyke.io default
# binary works directly. Running this on those hosts is harmless; just sets up
# an unused fallback.
#
# Safe to re-run. Exits 0 if prerequisites + ORT are already in place.
set -euo pipefail

WORKSPACE="$(cd "$(dirname "$0")/.." && pwd)"
cd "$WORKSPACE"

ORT_VERSION="1.23.0"
ORT_DIR=".tools/onnxruntime-linux-x64-${ORT_VERSION}"
ORT_TGZ="onnxruntime-linux-x64-${ORT_VERSION}.tgz"
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/${ORT_TGZ}"

missing=()
command -v pkg-config >/dev/null 2>&1 || missing+=(pkg-config)
# libssl-dev is required for reqwest's native-tls path (via openssl-sys).
if ! dpkg -l libssl-dev >/dev/null 2>&1 && ! pkg-config --exists openssl 2>/dev/null; then
    missing+=(libssl-dev)
fi
if [ ${#missing[@]} -gt 0 ]; then
    echo "Missing system packages: ${missing[*]}" >&2
    echo "Install with: sudo apt-get install -y ${missing[*]}" >&2
    exit 1
fi

if [ -f "$ORT_DIR/lib/libonnxruntime.so.${ORT_VERSION}" ]; then
    echo "ORT ${ORT_VERSION} already present at $ORT_DIR"
    exit 0
fi

echo "Downloading ONNX Runtime ${ORT_VERSION} -> .tools/"
mkdir -p .tools
cd .tools
curl -sfL -o "$ORT_TGZ" "$ORT_URL"
tar -xzf "$ORT_TGZ"
rm "$ORT_TGZ"
echo "ORT installed at $WORKSPACE/$ORT_DIR"
