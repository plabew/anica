#!/usr/bin/env bash
set -euo pipefail

# Bootstrap media runtime for local Anica development.
# Downloads a pre-built LGPL-only runtime from GitHub Releases if not present.
#
# Usage:
#   ./scripts/setup_media_tools.sh
#   ./scripts/setup_media_tools.sh --force

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
    warn "jq not found; cannot read manifest."
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
  local url base_url
  url="$(jq -r ".platforms.${os}.download_url" "${MANIFEST_PATH}")"
  base_url="$(jq -r '.release_base_url' "${MANIFEST_PATH}")"
  if [[ "${url}" != "null" && "${base_url}" != "null" ]]; then
    printf "%s" "${url}" | sed "s|{release_base_url}|${base_url}|g"
  else
    printf "%s" "${url}"
  fi
}

# Find the runtime root inside the extracted directory.
# The tar.gz may have an extra top-level folder (e.g. anica_runtime_macos_20260610).
find_runtime_root_in_staging() {
  local staging="$1"
  # If the staging root already contains ffmpeg/gstreamer, use it directly.
  if [[ -d "${staging}/ffmpeg" && -d "${staging}/gstreamer" ]]; then
    printf "%s" "${staging}"
    return 0
  fi
  # Otherwise look one level deeper for the actual runtime folder.
  local subdir
  subdir="$(find "${staging}" -maxdepth 2 -type d \( -name "ffmpeg" -o -name "gstreamer" \) | head -1 | xargs dirname 2>/dev/null || true)"
  if [[ -n "${subdir}" && -d "${subdir}" ]]; then
    printf "%s" "${subdir}"
    return 0
  fi
  return 1
}

# Copy a directory tree from src to dst.
copy_runtime_tree() {
  local src="$1" dst="$2"
  mkdir -p "$(dirname "${dst}")"
  rm -rf "${dst}"
  if have rsync; then
    rsync -a "${src}/" "${dst}/"
  else
    cp -R "${src}/." "${dst}/"
  fi
}

# Download and extract the runtime archive for the current platform.
main() {
  local os
  os="$(os_name)"
  if [[ "${os}" != "macos" && "${os}" != "windows" && "${os}" != "linux" ]]; then
    warn "Unsupported OS: ${os}."
    exit 1
  fi

  log "Anica runtime bootstrap"
  log "Manifest: ${MANIFEST_PATH}"

  local ffmpeg_version gst_version
  ffmpeg_version="$(manifest_version "ffmpeg")"
  gst_version="$(manifest_version "gstreamer")"
  if [[ -z "${ffmpeg_version}" || "${ffmpeg_version}" == "null" ]]; then
    warn "No FFmpeg version in manifest."
    exit 1
  fi
  if [[ -z "${gst_version}" || "${gst_version}" == "null" ]]; then
    warn "No GStreamer version in manifest."
    exit 1
  fi

  local platform_dir="${RUNTIME_DIR}/${os}"
  local current_dir="${RUNTIME_DIR}/current/${os}"

  # Check if called with --sync-only (just sync versioned to current, no download)
  if [[ "${1:-}" == "--sync-only" ]]; then
    if [[ -d "${platform_dir}/ffmpeg/${ffmpeg_version}" && -d "${platform_dir}/gstreamer/${gst_version}" ]]; then
      log "Syncing versioned runtime to current (sync-only)..."
      mkdir -p "${current_dir}"
      copy_runtime_tree "${platform_dir}/ffmpeg/${ffmpeg_version}" "${current_dir}/ffmpeg"
      copy_runtime_tree "${platform_dir}/gstreamer/${gst_version}" "${current_dir}/gstreamer"
      log "Runtime ready: ${current_dir}"
      exit 0
    else
      warn "Versioned runtime not found for sync-only."
      exit 1
    fi
  fi

  # Already present?
  if [[ -f "${current_dir}/ffmpeg/bin/ffmpeg" && -f "${current_dir}/gstreamer/bin/gst-launch-1.0" ]]; then
    log "Runtime already present at ${current_dir}"
    exit 0
  fi

  # If versioned folders exist, just sync to current.
  if [[ -d "${platform_dir}/ffmpeg/${ffmpeg_version}" && -d "${platform_dir}/gstreamer/${gst_version}" ]]; then
    log "Syncing versioned runtime to current..."
    mkdir -p "${current_dir}"
    copy_runtime_tree "${platform_dir}/ffmpeg/${ffmpeg_version}" "${current_dir}/ffmpeg"
    copy_runtime_tree "${platform_dir}/gstreamer/${gst_version}" "${current_dir}/gstreamer"
    log "Runtime ready: ${current_dir}"
    exit 0
  fi

  # Download from manifest.
  local url
  url="$(manifest_download_url "${os}")"
  if [[ -z "${url}" || "${url}" == "null" ]]; then
    warn "No download URL for ${os}."
    exit 1
  fi

  local archive_path="${RUNTIME_DIR}/anica-runtime-${os}.tar.gz"

  log "Downloading Anica runtime..."
  log "URL: ${url}"
  mkdir -p "${RUNTIME_DIR}"
  curl -fL --progress-bar "${url}" -o "${archive_path}" || {
    warn "Download failed."
    exit 1
  }

  # Extract to a temp directory.
  local staging_dir
  staging_dir="$(mktemp -d "${RUNTIME_DIR}/.extract-${os}.XXXXXX")"
  log "Extracting to ${staging_dir}..."
  tar -xzf "${archive_path}" -C "${staging_dir}"

  # Find the actual runtime root (may be nested inside a top-level folder).
  local runtime_root
  runtime_root="$(find_runtime_root_in_staging "${staging_dir}")" || {
    warn "Could not find ffmpeg/gstreamer inside extracted archive."
    rm -rf "${staging_dir}" "${archive_path}"
    exit 1
  }

  log "Found runtime root: ${runtime_root}"

  # Check if the runtime root already contains versioned subfolders (e.g. ffmpeg/8.0.1).
  # If so, copy the entire runtime root directly to the platform dir.
  # If not, copy the individual tool folders into versioned paths.
  mkdir -p "${platform_dir}" "${current_dir}"
  if [[ -d "${runtime_root}/ffmpeg/${ffmpeg_version}" && -d "${runtime_root}/gstreamer/${gst_version}" ]]; then
    log "Runtime already versioned. Copying directly to ${platform_dir}..."
    copy_runtime_tree "${runtime_root}" "${platform_dir}"
  else
    log "Copying ffmpeg to versioned path..."
    copy_runtime_tree "${runtime_root}/ffmpeg" "${platform_dir}/ffmpeg/${ffmpeg_version}"
    log "Copying gstreamer to versioned path..."
    copy_runtime_tree "${runtime_root}/gstreamer" "${platform_dir}/gstreamer/${gst_version}"
  fi

  # Sync to current (unversioned) for app consumption.
  log "Syncing to current/..."
  copy_runtime_tree "${platform_dir}/ffmpeg/${ffmpeg_version}" "${current_dir}/ffmpeg"
  copy_runtime_tree "${platform_dir}/gstreamer/${gst_version}" "${current_dir}/gstreamer"

  # Clean up.
  rm -f "${archive_path}"
  rm -rf "${staging_dir}"

  log "Runtime ready: ${current_dir}"
  log "Versioned: ${platform_dir}/ffmpeg/${ffmpeg_version}, ${platform_dir}/gstreamer/${gst_version}"
}

main "$@"