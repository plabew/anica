#!/usr/bin/env bash
set -euo pipefail

# Bootstrap media runtime for local Anica development.
# Downloads a pre-built LGPL-only runtime from GitHub Releases if not present.
#
# Usage examples:
#   ./scripts/setup_media_tools.sh
#   ./scripts/setup_media_tools.sh --yes

YES=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFEST_PATH="${REPO_ROOT}/tools/media_tools_manifest.json"
RUNTIME_DIR="${REPO_ROOT}/tools/runtime"

log() {
  echo "[setup] $*"
}

warn() {
  echo "[setup][warn] $*" >&2
}

have() {
  command -v "$1" >/dev/null 2>&1
}

os_name() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
    *) echo "other" ;;
  esac
}

# Read a version from the media_tools_manifest.json.
manifest_version() {
  local tool="$1"
  if ! have jq; then
    warn "jq not found; cannot read manifest. Install via 'brew install jq' or 'apt install jq'."
    return 1
  fi
  jq -r ".common.${tool}.version" "${MANIFEST_PATH}"
}

# Read the platform download URL from the manifest.
manifest_download_url() {
  local os="$1"
  if ! have jq; then
    return 1
  fi
  jq -r ".platforms.${os}.download_url" "${MANIFEST_PATH}"
}

# Resolve {release_base_url} placeholder in the manifest URL.
resolve_release_url() {
  local url="$1"
  local base_url
  base_url="$(jq -r '.release_base_url' "${MANIFEST_PATH}" 2>/dev/null || true)"
  if [[ -n "${base_url}" && "${base_url}" != "null" ]]; then
    printf "%s" "${url}" | sed "s|{release_base_url}|${base_url}|g"
  else
    printf "%s" "${url}"
  fi
}

# Download and extract the runtime archive for the current platform.
download_runtime() {
  local os
  os="$(os_name)"

  if [[ "${os}" != "macos" && "${os}" != "windows" && "${os}" != "linux" ]]; then
    warn "Unsupported OS: ${os}. Cannot auto-download runtime."
    return 1
  fi

  local url
  url="$(manifest_download_url "${os}")"
  if [[ -z "${url}" || "${url}" == "null" ]]; then
    warn "No download URL found in manifest for ${os}."
    return 1
  fi
  url="$(resolve_release_url "${url}")"

  local ext="tar.gz"

  local dest_dir="${RUNTIME_DIR}/${os}"
  local archive_name="anica-runtime-${os}.${ext}"
  local archive_path="${RUNTIME_DIR}/${archive_name}"

  if [[ -d "${dest_dir}/ffmpeg" && -d "${dest_dir}/gstreamer" ]]; then
    log "Runtime already present: ${dest_dir}"
    return 0
  fi

  log "Downloading Anica runtime for ${os}..."
  log "URL: ${url}"

  mkdir -p "${RUNTIME_DIR}"

  if ! have curl; then
    warn "curl not found. Cannot download runtime."
    return 1
  fi

  curl -fL --progress-bar "${url}" -o "${archive_path}" || {
    warn "Download failed: ${url}"
    return 1
  }

  log "Extracting runtime to ${dest_dir}..."
  mkdir -p "${dest_dir}"

  tar -xzf "${archive_path}" -C "${dest_dir}"

  rm -f "${archive_path}"
  log "Runtime ready: ${dest_dir}"
}

main() {
  log "Anica runtime bootstrap"
  log "Manifest: ${MANIFEST_PATH}"

  download_runtime

  log "Done."
}

main "$@"
