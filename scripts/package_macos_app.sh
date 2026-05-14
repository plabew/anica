#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Anica"
BUNDLE_ID="com.lovelyzombieyho.anica"
PROFILE="release"
OUT_DIR="dist"
SKIP_BUILD=0
SKIP_CODESIGN=0
CODESIGN_IDENTITY="${ANICA_CODESIGN_IDENTITY:--}"
GSTREAMER_PREFIX=""
FFMPEG_RUNTIME=""

GSTREAMER_PLUGIN_WHITELIST=(
  libgstapp.dylib
  libgstapplemedia.dylib
  libgstaudioconvert.dylib
  libgstaudioparsers.dylib
  libgstaudioresample.dylib
  libgstautodetect.dylib
  libgstcoreelements.dylib
  libgstgio.dylib
  libgstisomp4.dylib
  libgstosxaudio.dylib
  libgstpbtypes.dylib
  libgstplayback.dylib
  libgsttypefindfunctions.dylib
  libgstvideoconvertscale.dylib
  libgstvideoparsersbad.dylib
  libgstvideorate.dylib
  libgstvolume.dylib
)

GSTREAMER_PLUGIN_EXCLUDES=(
  libgstx264.dylib
  libgstx265.dylib
  libgstlibav.dylib
)

usage() {
  cat <<'USAGE'
Usage: scripts/package_macos_app.sh [options]

Options:
  --profile <debug|release>         Cargo profile (default: release)
  --out-dir <path>                  Output directory for the .app bundle (default: dist)
  --gstreamer-prefix <path>         Source GStreamer prefix (default: brew --prefix gstreamer)
  --ffmpeg-runtime <path>           Source FFmpeg runtime root
  --codesign-identity <identity>    codesign identity (default: ad-hoc "-")
  --skip-build                      Reuse existing target/<profile> binaries
  --skip-codesign                   Skip codesign for bundle artifacts
  -h, --help                        Show this help

Output:
  <out-dir>/Anica.app
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --gstreamer-prefix)
      GSTREAMER_PREFIX="${2:-}"
      shift 2
      ;;
    --ffmpeg-runtime)
      FFMPEG_RUNTIME="${2:-}"
      shift 2
      ;;
    --codesign-identity)
      CODESIGN_IDENTITY="${2:-}"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --skip-codesign)
      SKIP_CODESIGN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[package][error] Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[package][error] This packaging script is for macOS only." >&2
  exit 1
fi

if [[ "${PROFILE}" != "debug" && "${PROFILE}" != "release" ]]; then
  echo "[package][error] --profile must be debug or release." >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/${PROFILE}"
APP_BUNDLE="${REPO_ROOT}/${OUT_DIR}/${APP_NAME}.app"
CONTENTS_DIR="${APP_BUNDLE}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
RUNTIME_ROOT="${RESOURCES_DIR}/runtime/current/macos"
GST_DST="${RUNTIME_ROOT}/gstreamer"
FFMPEG_DST="${RUNTIME_ROOT}/ffmpeg"
APP_BIN="${MACOS_DIR}/anica"
ACP_BIN="${RESOURCES_DIR}/anica-acp"

log() {
  echo "[package] $*"
}

warn() {
  echo "[package][warn] $*" >&2
}

die() {
  echo "[package][error] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"
}

need_cmd cargo
need_cmd otool
need_cmd install_name_tool

if [[ "${SKIP_CODESIGN}" != "1" ]]; then
  need_cmd codesign
fi

is_macho_file() {
  local file="$1"
  local kind
  [[ -f "${file}" ]] || return 1
  kind="$(file -b "${file}" 2>/dev/null || true)"
  [[ "${kind}" == *"Mach-O"* ]]
}

copy_dir_follow_links() {
  local src="$1"
  local dst="$2"
  local rel
  mkdir -p "$(dirname "${dst}")"
  rm -rf "${dst}"
  mkdir -p "${dst}"
  (
    cd "${src}"
    while IFS= read -r -d '' rel; do
      mkdir -p "${dst}/${rel#./}"
    done < <(find -L . -type d -print0)
    while IFS= read -r -d '' rel; do
      mkdir -p "$(dirname "${dst}/${rel#./}")"
      cp -fL "${src}/${rel#./}" "${dst}/${rel#./}"
    done < <(find -L . -type f -print0)
  )
}

copy_file_follow_links() {
  local src="$1"
  local dst="$2"
  mkdir -p "$(dirname "${dst}")"
  cp -fL "${src}" "${dst}"
}

codesign_one() {
  local file="$1"
  [[ "${SKIP_CODESIGN}" == "1" ]] && return 0
  [[ -e "${file}" ]] || return 0
  codesign --force --sign "${CODESIGN_IDENTITY}" --timestamp=none "${file}" >/dev/null
}

codesign_bundle() {
  [[ "${SKIP_CODESIGN}" == "1" ]] && return 0
  codesign --force --deep --sign "${CODESIGN_IDENTITY}" --timestamp=none "${APP_BUNDLE}" >/dev/null
}

codesign_macho_tree() {
  [[ "${SKIP_CODESIGN}" == "1" ]] && return 0
  local file
  while IFS= read -r file; do
    is_macho_file "${file}" || continue
    codesign --force --sign "${CODESIGN_IDENTITY}" --timestamp=none "${file}" >/dev/null
  done < <(find "${APP_BUNDLE}" -type f 2>/dev/null | sort)
}

copy_gstreamer_plugins() {
  local src_dir="$1"
  local dst_dir="$2"
  local plugin_name
  mkdir -p "${dst_dir}"
  for plugin_name in "${GSTREAMER_PLUGIN_WHITELIST[@]}"; do
    if [[ -f "${src_dir}/${plugin_name}" ]]; then
      copy_file_follow_links "${src_dir}/${plugin_name}" "${dst_dir}/${plugin_name}"
    else
      warn "Whitelisted GStreamer plugin missing: ${plugin_name}"
    fi
  done
}

prune_excluded_gstreamer_plugins() {
  local dst_dir="$1"
  local plugin_name
  for plugin_name in "${GSTREAMER_PLUGIN_EXCLUDES[@]}"; do
    rm -f "${dst_dir}/${plugin_name}"
  done
}

set_or_replace_rpath() {
  local file="$1"
  local rpath="$2"
  install_name_tool -add_rpath "${rpath}" "${file}" 2>/dev/null || true
}

rewrite_deps_to_rpath() {
  local file="$1"
  local runtime_lib="$2"
  local dep base
  is_macho_file "${file}" || return 0
  while IFS= read -r dep; do
    [[ -z "${dep}" ]] && continue
    base="$(basename "${dep}")"
    if [[ -f "${runtime_lib}/${base}" && "${dep}" != "@rpath/${base}" ]]; then
      install_name_tool -change "${dep}" "@rpath/${base}" "${file}" 2>/dev/null || true
    fi
  done < <(otool -L "${file}" 2>/dev/null | awk 'NR>1 {print $1}')
}

set_dylib_ids() {
  local runtime_lib="$1"
  local file
  while IFS= read -r file; do
    is_macho_file "${file}" || continue
    install_name_tool -id "@rpath/$(basename "${file}")" "${file}" 2>/dev/null || true
  done < <(find "${runtime_lib}" -maxdepth 1 -type f -name "*.dylib*" 2>/dev/null)
}

copy_missing_homebrew_deps() {
  local runtime_lib="$1"
  shift
  local changed=1
  local target dep base
  local -a targets
  local extra
  targets=("$@")
  while [[ "${changed}" == "1" ]]; do
    changed=0
    for target in "${targets[@]}"; do
      [[ -e "${target}" ]] || continue
      while IFS= read -r dep; do
        [[ -z "${dep}" ]] && continue
        case "${dep}" in
          /opt/homebrew/*|/usr/local/*)
            base="$(basename "${dep}")"
            if [[ ! -f "${runtime_lib}/${base}" ]]; then
              copy_file_follow_links "${dep}" "${runtime_lib}/${base}"
              changed=1
            fi
            ;;
        esac
      done < <(otool -L "${target}" 2>/dev/null | awk 'NR>1 {print $1}')
    done
    if [[ "${changed}" == "1" ]]; then
      while IFS= read -r extra; do
        [[ -z "${extra}" ]] && continue
        targets+=("${extra}")
      done < <(find "${runtime_lib}" -maxdepth 1 -type f -name "*.dylib*" 2>/dev/null | sort)
    fi
  done
}

patch_ffmpeg_runtime_tree() {
  local root="$1"
  local lib_dir="${root}/lib"
  local bin_dir="${root}/bin"
  local bin

  [[ -d "${lib_dir}" ]] || return 0
  set_dylib_ids "${lib_dir}"

  while IFS= read -r bin; do
    rewrite_deps_to_rpath "${bin}" "${lib_dir}"
    set_or_replace_rpath "${bin}" "@executable_path/../lib"
    codesign_one "${bin}"
  done < <(find "${bin_dir}" -maxdepth 1 -type f 2>/dev/null | sort)

  while IFS= read -r bin; do
    rewrite_deps_to_rpath "${bin}" "${lib_dir}"
    set_or_replace_rpath "${bin}" "@loader_path"
    codesign_one "${bin}"
  done < <(find "${lib_dir}" -maxdepth 1 -type f -name "*.dylib*" 2>/dev/null | sort)
}

patch_gstreamer_runtime_tree() {
  local root="$1"
  local lib_dir="${root}/lib"
  local bin_dir="${root}/bin"
  local plugin_dir="${lib_dir}/gstreamer-1.0"
  local scanner="${root}/libexec/gstreamer-1.0/gst-plugin-scanner"
  local file

  [[ -d "${lib_dir}" ]] || return 0
  set_dylib_ids "${lib_dir}"

  while IFS= read -r file; do
    rewrite_deps_to_rpath "${file}" "${lib_dir}"
    set_or_replace_rpath "${file}" "@loader_path"
    codesign_one "${file}"
  done < <(find "${lib_dir}" -maxdepth 1 -type f -name "*.dylib*" 2>/dev/null | sort)

  while IFS= read -r file; do
    rewrite_deps_to_rpath "${file}" "${lib_dir}"
    set_or_replace_rpath "${file}" "@executable_path/../lib"
    codesign_one "${file}"
  done < <(find "${bin_dir}" -maxdepth 1 -type f 2>/dev/null | sort)

  while IFS= read -r file; do
    rewrite_deps_to_rpath "${file}" "${lib_dir}"
    set_or_replace_rpath "${file}" "@loader_path/.."
    codesign_one "${file}"
  done < <(find "${plugin_dir}" -type f -name "*.dylib" 2>/dev/null | sort)

  if [[ -x "${scanner}" ]]; then
    rewrite_deps_to_rpath "${scanner}" "${lib_dir}"
    set_or_replace_rpath "${scanner}" "@executable_path/../../lib"
    codesign_one "${scanner}"
  fi
}

patch_app_binary() {
  local binary="$1"
  local runtime_lib="$2"
  local dep base
  is_macho_file "${binary}" || return 0
  while IFS= read -r dep; do
    [[ -z "${dep}" ]] && continue
    base="$(basename "${dep}")"
    if [[ -f "${runtime_lib}/${base}" && "${dep}" != "@rpath/${base}" ]]; then
      install_name_tool -change "${dep}" "@rpath/${base}" "${binary}" 2>/dev/null || true
    fi
  done < <(otool -L "${binary}" 2>/dev/null | awk 'NR>1 {print $1}')
  set_or_replace_rpath "${binary}" "@executable_path/../Resources/runtime/current/macos/gstreamer/lib"
  codesign_one "${binary}"
}

write_info_plist() {
  cat > "${CONTENTS_DIR}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>anica</string>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${APP_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${APP_VERSION}</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST
}

resolve_cargo_package_version() {
  local cargo_toml="$1"
  awk '
    BEGIN { in_package = 0 }
    /^\[package\][[:space:]]*$/ { in_package = 1; next }
    /^\[[^]]+\][[:space:]]*$/ { if (in_package) exit; in_package = 0 }
    in_package && $0 ~ /^[[:space:]]*version[[:space:]]*=/ {
      line = $0
      gsub(/^[^"]*"/, "", line)
      gsub(/".*$/, "", line)
      if (length(line) > 0) {
        print line
        exit
      }
    }
  ' "${cargo_toml}"
}

resolve_ffmpeg_runtime_root() {
  local root="$1"
  local candidates=(
    "${root}/current"
    "${root}"
    "${root}/ffmpeg"
    "${root}/ffmpeg/current"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -x "${candidate}/bin/ffmpeg" ]]; then
      printf "%s\n" "${candidate}"
      return 0
    fi
  done
  return 1
}

CARGO_TOML="${REPO_ROOT}/Cargo.toml"
APP_VERSION="$(resolve_cargo_package_version "${CARGO_TOML}")"
[[ -n "${APP_VERSION}" ]] || die "Failed to read [package].version from ${CARGO_TOML}"
log "Using app version from Cargo.toml: ${APP_VERSION}"

if [[ -z "${GSTREAMER_PREFIX}" ]]; then
  GSTREAMER_PREFIX="$(brew --prefix gstreamer)"
fi
[[ -d "${GSTREAMER_PREFIX}" ]] || die "GStreamer prefix not found: ${GSTREAMER_PREFIX}"

if [[ -z "${FFMPEG_RUNTIME}" ]]; then
  if [[ -d "${REPO_ROOT}/tools/runtime/current/macos/ffmpeg" ]]; then
    FFMPEG_RUNTIME="${REPO_ROOT}/tools/runtime/current/macos/ffmpeg"
  else
    die "FFmpeg runtime not found. Use --ffmpeg-runtime <path>."
  fi
fi
FFMPEG_RUNTIME="$(resolve_ffmpeg_runtime_root "${FFMPEG_RUNTIME}")" || die "Invalid FFmpeg runtime root: ${FFMPEG_RUNTIME}"

if [[ "${SKIP_BUILD}" != "1" ]]; then
  log "Building cargo binaries (${PROFILE})..."
  if [[ "${PROFILE}" == "release" ]]; then
    (cd "${REPO_ROOT}" && cargo build --release --bins)
  else
    (cd "${REPO_ROOT}" && cargo build --bins)
  fi
fi

[[ -x "${TARGET_DIR}/anica" ]] || die "Missing app binary: ${TARGET_DIR}/anica"
[[ -x "${TARGET_DIR}/anica-acp" ]] || die "Missing ACP binary: ${TARGET_DIR}/anica-acp"

log "Creating app bundle at ${APP_BUNDLE}"
rm -rf "${APP_BUNDLE}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${GST_DST}" "${FFMPEG_DST}"
write_info_plist

copy_file_follow_links "${TARGET_DIR}/anica" "${APP_BIN}"
copy_file_follow_links "${TARGET_DIR}/anica-acp" "${ACP_BIN}"
copy_dir_follow_links "${REPO_ROOT}/assets" "${RESOURCES_DIR}/assets"
copy_dir_follow_links "${REPO_ROOT}/docs" "${RESOURCES_DIR}/docs"
copy_file_follow_links "${REPO_ROOT}/LICENSE" "${RESOURCES_DIR}/LICENSE"
copy_file_follow_links "${REPO_ROOT}/NOTICE" "${RESOURCES_DIR}/NOTICE"
copy_file_follow_links "${REPO_ROOT}/SECURITY.md" "${RESOURCES_DIR}/SECURITY.md"

log "Copying FFmpeg runtime from ${FFMPEG_RUNTIME}"
copy_dir_follow_links "${FFMPEG_RUNTIME}" "${FFMPEG_DST}"
patch_ffmpeg_runtime_tree "${FFMPEG_DST}"

log "Copying GStreamer runtime from ${GSTREAMER_PREFIX}"
mkdir -p "${GST_DST}/bin" "${GST_DST}/lib" "${GST_DST}/libexec/gstreamer-1.0"
for bin_name in gst-launch-1.0 gst-inspect-1.0 gst-typefind-1.0; do
  if [[ -x "${GSTREAMER_PREFIX}/bin/${bin_name}" ]]; then
    copy_file_follow_links "${GSTREAMER_PREFIX}/bin/${bin_name}" "${GST_DST}/bin/${bin_name}"
  fi
done
if [[ -d "${GSTREAMER_PREFIX}/lib/gstreamer-1.0" ]]; then
  copy_gstreamer_plugins "${GSTREAMER_PREFIX}/lib/gstreamer-1.0" "${GST_DST}/lib/gstreamer-1.0"
fi
if [[ -x "${GSTREAMER_PREFIX}/libexec/gstreamer-1.0/gst-plugin-scanner" ]]; then
  copy_file_follow_links \
    "${GSTREAMER_PREFIX}/libexec/gstreamer-1.0/gst-plugin-scanner" \
    "${GST_DST}/libexec/gstreamer-1.0/gst-plugin-scanner"
fi

GST_TARGETS=()
while IFS= read -r target; do
  [[ -z "${target}" ]] && continue
  GST_TARGETS+=("${target}")
done < <(
  {
    find "${GST_DST}/bin" -maxdepth 1 -type f 2>/dev/null
    find "${GST_DST}/lib" -maxdepth 1 -type f -name "*.dylib*" 2>/dev/null
    find "${GST_DST}/lib/gstreamer-1.0" -type f -name "*.dylib" 2>/dev/null
    [[ -f "${GST_DST}/libexec/gstreamer-1.0/gst-plugin-scanner" ]] && echo "${GST_DST}/libexec/gstreamer-1.0/gst-plugin-scanner"
    echo "${APP_BIN}"
  } | sort -u
)
log "Resolving external Homebrew dylibs for bundled GStreamer..."
copy_missing_homebrew_deps "${GST_DST}/lib" "${GST_TARGETS[@]}"
log "Pruning excluded GStreamer plugins..."
prune_excluded_gstreamer_plugins "${GST_DST}/lib/gstreamer-1.0"
log "Patching bundled GStreamer Mach-O links..."
patch_gstreamer_runtime_tree "${GST_DST}"
log "Patching main app binary to bundled GStreamer..."
patch_app_binary "${APP_BIN}" "${GST_DST}/lib"

codesign_one "${ACP_BIN}"
log "Re-signing all Mach-O files inside app bundle..."
codesign_macho_tree
log "Signing app bundle..."
codesign_bundle

log "Bundle ready: ${APP_BUNDLE}"
log "Open it with: open \"${APP_BUNDLE}\""
log "Next for public distribution: sign with Developer ID and notarize."
