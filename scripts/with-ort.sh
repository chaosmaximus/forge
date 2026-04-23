#!/usr/bin/env bash
# with-ort.sh — cargo runner that prepends .tools/onnxruntime/lib to
# LD_LIBRARY_PATH so binaries (daemon, test runners) find libonnxruntime.so at
# exec time. Wired via .cargo/config.toml's [target.*] runner key. No-op on
# hosts where .tools/ is absent (e.g. macOS, glibc ≥2.38 Linux using pyke's
# default binary).
set -e
WORKSPACE="$(cd "$(dirname "$0")/.." && pwd)"
ORT_LIB="${WORKSPACE}/.tools/onnxruntime-linux-x64-1.23.0/lib"
if [ -d "$ORT_LIB" ]; then
    export LD_LIBRARY_PATH="${ORT_LIB}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
fi
exec "$@"
