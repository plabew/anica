#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[anica-runner] macOS development runs are supported on Apple Silicon only." >&2
  exit 1
fi

# Read the stable GStreamer version from the manifest.
gst_version="1.28.1"
manifest_path="${repo_root}/tools/media_tools_manifest.json"
if command -v jq >/dev/null 2>&1 && [[ -f "${manifest_path}" ]]; then
  gst_version="$(jq -r ".common.gstreamer.version" "${manifest_path}" 2>/dev/null || echo "1.28.1")"
fi
export ANICA_HOST_GSTREAMER_PLUGIN_DIR="${repo_root}/tools/runtime/current/macos/gstreamer/${gst_version}/lib/gstreamer-1.0"
export ANICA_HOST_GSTREAMER_REGISTRY="${repo_root}/target/.anica-host-gstreamer-registry.bin"

exec "${script_dir}/cargo_run_with_acp_check.sh" "$@"
