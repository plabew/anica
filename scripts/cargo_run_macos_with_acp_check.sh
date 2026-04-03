#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[anica-runner] macOS development runs are supported on Apple Silicon only." >&2
  exit 1
fi

export ANICA_HOST_GSTREAMER_PLUGIN_DIR="${repo_root}/.cargo/gstreamer-plugins/macos-host"
export ANICA_HOST_GSTREAMER_REGISTRY="${repo_root}/target/.anica-host-gstreamer-registry.bin"

exec "${script_dir}/cargo_run_with_acp_check.sh" "$@"
