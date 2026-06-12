#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Anica Editor"
BUNDLE_ID="com.lovelyzombieyho.anica"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/release"
DIST_DIR="${REPO_ROOT}/dist"
APP_DIR="${DIST_DIR}/${APP_NAME}.app"
MACOS_DIR="${APP_DIR}/Contents/MacOS"
RESOURCES_DIR="${APP_DIR}/Contents/Resources"
RUNTIME_DIR="${RESOURCES_DIR}/runtime/current/macos"
FFMPEG_SRC="${REPO_ROOT}/tools/runtime/current/macos/ffmpeg"
FFMPEG_DST="${RUNTIME_DIR}/ffmpeg"
SKIP_BUILD=0

usage() {
  cat <<USAGE
Usage: scripts/package_macos_app.sh [--skip-build] [--ffmpeg-prefix <path>]

Builds a macOS .app bundle with the Anica binary, assets, and FFmpeg runtime.
USAGE
}

log() { echo "[package] $*"; }
die() { echo "[package][error] $*" >&2; exit 1; }
copy_tree() {
  local src="$1" dst="$2"
  [[ -d "${src}" ]] || die "Missing directory: ${src}"
  mkdir -p "$(dirname "${dst}")"
  rm -rf "${dst}"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a "${src}/" "${dst}/"
  else
    cp -R "${src}/." "${dst}/"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --ffmpeg-prefix) FFMPEG_SRC="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

if [[ "${SKIP_BUILD}" != "1" ]]; then
  log "Building release binary"
  cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml"
fi

APP_BIN="${TARGET_DIR}/anica"
[[ -x "${APP_BIN}" ]] || die "Release binary not found: ${APP_BIN}"
[[ -d "${FFMPEG_SRC}" ]] || die "FFmpeg runtime not found: ${FFMPEG_SRC}"

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${RUNTIME_DIR}"
cp "${APP_BIN}" "${MACOS_DIR}/anica"
chmod +x "${MACOS_DIR}/anica"

if [[ -d "${REPO_ROOT}/assets" ]]; then
  copy_tree "${REPO_ROOT}/assets" "${RESOURCES_DIR}/assets"
fi
copy_tree "${FFMPEG_SRC}" "${FFMPEG_DST}"

cat > "${APP_DIR}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>anica</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.1</string>
  <key>CFBundleVersion</key><string>0.1.1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

log "Bundle ready: ${APP_DIR}"
