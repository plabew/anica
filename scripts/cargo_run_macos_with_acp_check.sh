#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[anica-runner] macOS development runs are supported on Apple Silicon only." >&2
  exit 1
fi

anica_gstreamer_opt_in_enabled() {
  case "${ANICA_ENABLE_GSTREAMER:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
  esac
  case "${ANICA_VIDEO_BACKEND:-}" in
    gstreamer|GSTREAMER|gst|GST) return 0 ;;
  esac
  return 1
}

resolve_current_gstreamer_plugin_dir() {
  local gst_root="${repo_root}/tools/runtime/current/macos/gstreamer"
  local candidate

  if [[ -d "${gst_root}/lib/gstreamer-1.0" ]]; then
    printf "%s" "${gst_root}/lib/gstreamer-1.0"
    return 0
  fi
  while IFS= read -r candidate; do
    if [[ -d "${candidate}/lib/gstreamer-1.0" ]]; then
      printf "%s" "${candidate}/lib/gstreamer-1.0"
      return 0
    fi
  done < <(find "${gst_root}" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)
  return 1
}

if anica_gstreamer_opt_in_enabled; then
  # Host GStreamer hints are opt-in while FFmpeg preview is the default backend.
  if plugin_dir="$(resolve_current_gstreamer_plugin_dir)"; then
    export ANICA_HOST_GSTREAMER_PLUGIN_DIR="${plugin_dir}"
  fi
  export ANICA_HOST_GSTREAMER_REGISTRY="${repo_root}/target/.anica-host-gstreamer-registry.bin"
fi

exec "${script_dir}/cargo_run_with_acp_check.sh" "$@"
