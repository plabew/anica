#!/usr/bin/env bash
set -euo pipefail

# Bootstrap media dependencies for local Anica development.
# Default behavior:
# - Build and install LGPL-only FFmpeg into ~/.anica/tools (no system overwrite)
#
# Usage examples:
#   ./scripts/setup_media_tools.sh
#   ./scripts/setup_media_tools.sh --mode system --yes
#   ./scripts/setup_media_tools.sh --tools-home "$HOME/.anica/tools" --ffmpeg-version 8.0.1

MODE="local-lgpl" # local-lgpl | system
SKIP_FFMPEG=0
YES=0
FFMPEG_VERSION="${FFMPEG_VERSION:-8.0.1}"

if [[ "${HOME:-}" == "" ]]; then
  echo "[setup] HOME is not set; cannot resolve default tools path." >&2
  exit 1
fi
TOOLS_HOME_DEFAULT="${HOME}/.anica/tools"
TOOLS_HOME="${ANICA_TOOLS_HOME:-$TOOLS_HOME_DEFAULT}"

abspath() {
  local path="${1:-}"
  if [[ -z "${path}" ]]; then
    return 1
  fi
  if [[ "${path}" == /* ]]; then
    printf "%s\n" "${path}"
    return 0
  fi
  printf "%s\n" "$(pwd)/${path}"
}

usage() {
  cat <<'USAGE'
Usage: scripts/setup_media_tools.sh [options]

Options:
  --mode <local-lgpl|system>       FFmpeg install mode (default: local-lgpl)
  --skip-ffmpeg                    Skip FFmpeg build/install
  --tools-home <path>              Override tools home (default: ~/.anica/tools)
  --ffmpeg-version <version>       FFmpeg version (default: 8.0.1)
  --yes                            Non-interactive package install

  -h, --help                       Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="${2:-}"
      shift 2
      ;;
    --skip-ffmpeg)
      SKIP_FFMPEG=1
      shift
      ;;
    --tools-home)
      TOOLS_HOME="${2:-}"
      shift 2
      ;;
    --ffmpeg-version)
      FFMPEG_VERSION="${2:-}"
      shift 2
      ;;
    --yes)
      YES=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[setup] Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$MODE" != "local-lgpl" && "$MODE" != "system" ]]; then
  echo "[setup] Invalid --mode: $MODE" >&2
  exit 2
fi

TOOLS_HOME="$(abspath "${TOOLS_HOME}")"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFEST_PATH="${REPO_ROOT}/tools/media_tools_manifest.json"

FFMPEG_SRC_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"
FFMPEG_PREFIX="${TOOLS_HOME}/ffmpeg/${FFMPEG_VERSION}"
FFMPEG_BIN="${FFMPEG_PREFIX}/bin/ffmpeg"
FFPROBE_BIN="${FFMPEG_PREFIX}/bin/ffprobe"
FFMPEG_CURRENT_LINK="${TOOLS_HOME}/ffmpeg/current"
FFMPEG_STABLE_BIN_DIR="${TOOLS_HOME}/ffmpeg/bin"

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
    *) echo "other" ;;
  esac
}

confirm_or_exit() {
  local prompt="$1"
  if [[ $YES -eq 1 ]]; then
    return 0
  fi
  read -r -p "${prompt} [y/N] " reply
  if [[ "$reply" != "y" && "$reply" != "Y" ]]; then
    echo "[setup] Aborted."
    exit 1
  fi
}

install_ffmpeg_system_macos() {
  if ! have brew; then
    warn "Homebrew not found. Install from https://brew.sh first."
    return 1
  fi
  if have ffmpeg && have ffprobe; then
    log "System ffmpeg/ffprobe already present."
    return 0
  fi
  log "Installing system FFmpeg via Homebrew..."
  brew install ffmpeg
}

install_ffmpeg_system_linux() {
  if have ffmpeg && have ffprobe; then
    log "System ffmpeg/ffprobe already present."
    return 0
  fi
  if have apt-get; then
    local apt_flags=(-y)
    if [[ $YES -eq 0 ]]; then
      apt_flags=()
    fi
    log "Installing system FFmpeg via apt..."
    sudo apt-get update
    sudo apt-get install "${apt_flags[@]}" ffmpeg
    return 0
  fi
  if have dnf; then
    local dnf_flags=()
    if [[ $YES -eq 1 ]]; then
      dnf_flags=(-y)
    fi
    log "Installing system FFmpeg via dnf..."
    sudo dnf install "${dnf_flags[@]}" ffmpeg
    return 0
  fi
  if have pacman; then
    local pacman_flags=()
    if [[ $YES -eq 1 ]]; then
      pacman_flags=(--noconfirm)
    fi
    log "Installing system FFmpeg via pacman..."
    sudo pacman -S "${pacman_flags[@]}" ffmpeg
    return 0
  fi
  warn "Unsupported Linux package manager for auto FFmpeg install."
  return 1
}

ensure_local_ffmpeg_build_deps_macos() {
  if ! have brew; then
    warn "Homebrew is required for local FFmpeg build on macOS."
    return 1
  fi
  local pkgs=(pkg-config nasm)
  local pkg
  for pkg in "${pkgs[@]}"; do
    if brew list --versions "$pkg" >/dev/null 2>&1; then
      continue
    fi
    log "Installing build dependency via Homebrew: $pkg"
    brew install "$pkg"
  done
}

ensure_local_ffmpeg_build_deps_linux() {
  if have apt-get; then
    local apt_flags=(-y)
    if [[ $YES -eq 0 ]]; then
      apt_flags=()
    fi
    log "Installing FFmpeg build dependencies via apt..."
    sudo apt-get update
    sudo apt-get install "${apt_flags[@]}" \
      build-essential pkg-config curl xz-utils nasm yasm
    return 0
  fi
  if have dnf; then
    local dnf_flags=()
    if [[ $YES -eq 1 ]]; then
      dnf_flags=(-y)
    fi
    log "Installing FFmpeg build dependencies via dnf..."
    sudo dnf install "${dnf_flags[@]}" \
      gcc gcc-c++ make pkgconfig curl xz nasm yasm
    return 0
  fi
  if have pacman; then
    local pacman_flags=()
    if [[ $YES -eq 1 ]]; then
      pacman_flags=(--noconfirm)
    fi
    log "Installing FFmpeg build dependencies via pacman..."
    sudo pacman -S "${pacman_flags[@]}" \
      base-devel pkgconf curl xz nasm yasm
    return 0
  fi
  warn "Unsupported Linux package manager for auto build deps."
  return 1
}

patch_macos_local_ffmpeg_runtime() {
  local ffmpeg_prefix="$1"
  local ffmpeg_lib="${ffmpeg_prefix}/lib"
  local ffmpeg_bin="${ffmpeg_prefix}/bin"

  if [[ ! -d "${ffmpeg_lib}" || ! -d "${ffmpeg_bin}" ]]; then
    return 0
  fi

  list_macho_deps() {
    local target="$1"
    local kind
    kind="$(file -b "${target}" 2>/dev/null || true)"
    if [[ "${kind}" != *"Mach-O"* ]]; then
      return 0
    fi
    otool -L "${target}" 2>/dev/null | awk 'NR>1 {print $1}' || true
  }

  local lib_file
  while IFS= read -r lib_file; do
    install_name_tool -id "@rpath/$(basename "${lib_file}")" "${lib_file}" 2>/dev/null || true
    install_name_tool -add_rpath "@loader_path" "${lib_file}" 2>/dev/null || true
  done < <(find "${ffmpeg_lib}" -maxdepth 1 -type f -name "*.dylib")

  local bin_file dep dep_base
  for bin_file in "${ffmpeg_bin}/ffmpeg" "${ffmpeg_bin}/ffprobe"; do
    if [[ ! -x "${bin_file}" ]]; then
      continue
    fi
    while IFS= read -r dep; do
      dep_base="$(basename "${dep}")"
      if [[ -f "${ffmpeg_lib}/${dep_base}" ]]; then
        install_name_tool -change "${dep}" "@rpath/${dep_base}" "${bin_file}" 2>/dev/null || true
      fi
    done < <(list_macho_deps "${bin_file}")
    install_name_tool -add_rpath "@executable_path/../lib" "${bin_file}" 2>/dev/null || true
  done
}

build_local_ffmpeg_lgpl() {
  if [[ -x "$FFMPEG_BIN" && -x "$FFPROBE_BIN" ]]; then
    log "Local FFmpeg already exists at $FFMPEG_PREFIX"
  else
    # Run compilation in a sub-function so a signal/trap does not prevent
    # link creation below when binaries are partially installed.
    _ffmpeg_compile_and_install() {
      local os
      os="$(os_name)"
      case "$os" in
        macos) ensure_local_ffmpeg_build_deps_macos ;;
        linux) ensure_local_ffmpeg_build_deps_linux ;;
        *)
          warn "local-lgpl mode is not supported on this OS via shell script."
          return 1
          ;;
      esac

      local build_root
      build_root="$(mktemp -d "${TMPDIR:-/tmp}/anica-ffmpeg-build-XXXXXX")"
      trap 'rm -rf "'"${build_root}"'"' EXIT

      log "Downloading FFmpeg source: $FFMPEG_SRC_URL"
      curl -fsSL "$FFMPEG_SRC_URL" -o "$build_root/ffmpeg.tar.xz"
      tar -xJf "$build_root/ffmpeg.tar.xz" -C "$build_root"

      local src_dir="$build_root/ffmpeg-${FFMPEG_VERSION}"
      if [[ ! -d "$src_dir" ]]; then
        warn "FFmpeg source directory not found after extraction."
        return 1
      fi

      mkdir -p "$FFMPEG_PREFIX"
      pushd "$src_dir" >/dev/null
      local cfg=(
        "--prefix=$FFMPEG_PREFIX"
        "--disable-gpl"
        "--disable-nonfree"
        "--disable-autodetect"
        "--enable-zlib"
        "--enable-shared"
        "--disable-static"
        "--disable-debug"
        "--disable-doc"
        "--disable-ffplay"
        "--enable-pthreads"
        "--enable-version3"
        "--enable-decoder=png"
        "--enable-decoder=apng"
        "--enable-demuxer=image2"
        "--enable-demuxer=image2pipe"
      )
      if [[ "$os" == "macos" ]]; then
        cfg+=("--enable-videotoolbox" "--enable-audiotoolbox")
      fi

      log "Configuring FFmpeg (${MODE})..."
      ./configure "${cfg[@]}"
      log "Building FFmpeg..."
      if have nproc; then
        make -j"$(nproc)"
      else
        make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"
      fi
      make install
      popd >/dev/null

      if [[ "${os}" == "macos" ]]; then
        patch_macos_local_ffmpeg_runtime "${FFMPEG_PREFIX}"
      fi

      trap - EXIT
      rm -rf "$build_root"
    }

    _ffmpeg_compile_and_install || warn "FFmpeg compilation had errors (status $?). Checking if binary was partially installed..."
    unset -f _ffmpeg_compile_and_install
  fi

  # Create symlinks even after partial build success.
  if [[ -x "$FFMPEG_BIN" ]]; then
    mkdir -p "$(dirname "$FFMPEG_CURRENT_LINK")" "$FFMPEG_STABLE_BIN_DIR"
    ln -sfn "$FFMPEG_PREFIX" "$FFMPEG_CURRENT_LINK"
    ln -sfn "$FFMPEG_CURRENT_LINK/bin/ffmpeg" "$FFMPEG_STABLE_BIN_DIR/ffmpeg"
    ln -sfn "$FFMPEG_CURRENT_LINK/bin/ffprobe" "$FFMPEG_STABLE_BIN_DIR/ffprobe"
    log "Local FFmpeg installed: $FFMPEG_BIN"
    log "Export this env for Anica (optional):"
    echo "export ANICA_TOOLS_HOME=\"$TOOLS_HOME\""
    echo "export ANICA_FFMPEG_PATH=\"$FFMPEG_STABLE_BIN_DIR/ffmpeg\""
  else
    warn "FFmpeg binary not found after build: $FFMPEG_BIN"
  fi
}

ensure_ffmpeg() {
  if [[ $SKIP_FFMPEG -eq 1 ]]; then
    log "Skipping FFmpeg build/install (--skip-ffmpeg)."
    return 0
  fi

  if [[ "$MODE" == "system" ]]; then
    case "$(os_name)" in
      macos) install_ffmpeg_system_macos ;;
      linux) install_ffmpeg_system_linux ;;
      *)
        warn "Unsupported OS for system auto-install."
        ;;
    esac

    if have ffmpeg && have ffprobe; then
      log "Using system ffmpeg: $(command -v ffmpeg)"
      ffmpeg -version | sed -n '1,3p'
    else
      warn "ffmpeg/ffprobe still missing; install manually."
    fi
    return 0
  fi

  build_local_ffmpeg_lgpl
}

main() {
  log "Anica media tools bootstrap"
  log "Manifest: $MANIFEST_PATH"
  log "Mode: $MODE"
  log "Tools home: $TOOLS_HOME"

  if [[ $YES -eq 0 && "$MODE" == "system" ]]; then
    confirm_or_exit "System mode may install/upgrade global packages. Continue?"
  fi

  if [[ "$MODE" == "local-lgpl" ]]; then
    mkdir -p "$TOOLS_HOME"
  else
    mkdir -p "$TOOLS_HOME" 2>/dev/null || warn "Cannot create tools home at $TOOLS_HOME (non-fatal in system mode)."
  fi

  # Allow FFmpeg build to fail without aborting the whole script.
  ensure_ffmpeg || warn "FFmpeg build/install failed (status $?). The Rust runtime layer will attempt to use any existing FFmpeg binary."

  log "Done."
}

main "$@"
