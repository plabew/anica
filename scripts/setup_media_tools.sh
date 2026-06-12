#!/usr/bin/env bash
set -euo pipefail

# Bootstrap FFmpeg runtime for local Anica development.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFEST_PATH="${REPO_ROOT}/tools/media_tools_manifest.json"
RUNTIME_DIR="${ANICA_TOOLS_HOME:-${REPO_ROOT}/tools/runtime}"

log() { echo "[setup] $*"; }
warn() { echo "[setup][warn] $*" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

os_name() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
    *) echo "other" ;;
  esac
}

manifest_value() {
  local expr="$1"
  have jq || return 1
  jq -r "${expr}" "${MANIFEST_PATH}"
}

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

resolve_tool_tree() {
  local root="$1" binary="$2" version="${3:-}" candidate
  if [[ -x "${root}/bin/${binary}" ]]; then
    printf "%s" "${root}"
    return 0
  fi
  if [[ -n "${version}" && -x "${root}/${version}/bin/${binary}" ]]; then
    printf "%s" "${root}/${version}"
    return 0
  fi
  while IFS= read -r candidate; do
    if [[ -x "${candidate}/bin/${binary}" ]]; then
      printf "%s" "${candidate}"
      return 0
    fi
  done < <(find "${root}" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)
  return 1
}

find_runtime_root_in_staging() {
  local staging="$1" subdir
  if [[ -d "${staging}/ffmpeg" ]]; then
    printf "%s" "${staging}"
    return 0
  fi
  subdir="$(find "${staging}" -maxdepth 2 -type d -name "ffmpeg" | head -1 | xargs dirname 2>/dev/null || true)"
  if [[ -n "${subdir}" && -d "${subdir}" ]]; then
    printf "%s" "${subdir}"
    return 0
  fi
  return 1
}

main() {
  local os ffmpeg_version platform_dir current_dir ffmpeg_binary
  os="$(os_name)"
  [[ "${os}" == "macos" || "${os}" == "windows" || "${os}" == "linux" ]] || { warn "Unsupported OS: ${os}."; exit 1; }
  ffmpeg_version="$(manifest_value '.common.ffmpeg.version // empty' || true)"
  [[ -n "${ffmpeg_version}" && "${ffmpeg_version}" != "null" ]] || { warn "No FFmpeg version in manifest."; exit 1; }

  platform_dir="${RUNTIME_DIR}/${os}"
  current_dir="${RUNTIME_DIR}/current/${os}"
  ffmpeg_binary="ffmpeg"
  [[ "${os}" == "windows" ]] && ffmpeg_binary="ffmpeg.exe"

  if [[ "${1:-}" == "--sync-only" ]]; then
    local src="${platform_dir}/ffmpeg/${ffmpeg_version}"
    [[ -d "${src}" ]] || { warn "Versioned FFmpeg runtime not found."; exit 1; }
    mkdir -p "${current_dir}"
    copy_runtime_tree "$(resolve_tool_tree "${src}" "${ffmpeg_binary}" "${ffmpeg_version}")" "${current_dir}/ffmpeg"
    log "Runtime ready: ${current_dir}"
    exit 0
  fi

  if [[ -x "${current_dir}/ffmpeg/bin/${ffmpeg_binary}" ]]; then
    log "Runtime already present at ${current_dir}"
    exit 0
  fi
  if [[ -d "${platform_dir}/ffmpeg/${ffmpeg_version}" ]]; then
    mkdir -p "${current_dir}"
    copy_runtime_tree "$(resolve_tool_tree "${platform_dir}/ffmpeg/${ffmpeg_version}" "${ffmpeg_binary}" "${ffmpeg_version}")" "${current_dir}/ffmpeg"
    log "Runtime ready: ${current_dir}"
    exit 0
  fi

  local url base_url tmp staging archive runtime_root
  url="$(manifest_value ".platforms.${os}.download_url // empty" || true)"
  base_url="$(manifest_value '.release_base_url // empty' || true)"
  [[ -n "${url}" && "${url}" != "null" ]] || { warn "No download URL for ${os}."; exit 1; }
  url="${url//\{release_base_url\}/${base_url}}"
  tmp="$(mktemp -d)"
  staging="${tmp}/staging"
  archive="${tmp}/runtime.tar.gz"
  mkdir -p "${staging}"
  log "Downloading ${url}"
  if have curl; then curl -L --fail -o "${archive}" "${url}"; else wget -O "${archive}" "${url}"; fi
  tar -xzf "${archive}" -C "${staging}"
  runtime_root="$(find_runtime_root_in_staging "${staging}")" || { warn "Could not find FFmpeg inside extracted archive."; exit 1; }
  mkdir -p "${platform_dir}/ffmpeg" "${current_dir}"
  copy_runtime_tree "$(resolve_tool_tree "${runtime_root}/ffmpeg" "${ffmpeg_binary}" "${ffmpeg_version}")" "${platform_dir}/ffmpeg/${ffmpeg_version}"
  copy_runtime_tree "${platform_dir}/ffmpeg/${ffmpeg_version}" "${current_dir}/ffmpeg"
  rm -rf "${tmp}"
  log "Runtime ready: ${current_dir}"
}

main "$@"
