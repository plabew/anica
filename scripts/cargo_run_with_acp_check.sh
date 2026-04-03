#!/usr/bin/env bash
set -euo pipefail

# Cargo runner receives: <compiled-binary> [args...]
#
# Purpose:
# - Before launching `anica`, check whether `anica-acp` is missing or stale.
# - If ACP-related sources changed, rebuild `anica-acp` automatically.
# - Avoid manual `cargo build --bin anica-acp` during ACP debugging.
if [[ $# -lt 1 ]]; then
  exit 1
fi

exe_path="$1"
shift || true
ANICA_RESOLVED_RUNTIME_ROOT=""

configure_curated_host_gstreamer_plugins() {
  local plugin_dir="$1"
  local registry_path="$2"

  [[ -d "${plugin_dir}" ]] || return 1

  export GST_PLUGIN_PATH="${plugin_dir}"
  export GST_PLUGIN_PATH_1_0="${plugin_dir}"
  export GST_PLUGIN_SYSTEM_PATH_1_0="${plugin_dir}"
  export GST_PLUGIN_SYSTEM_PATH="${plugin_dir}"

  if [[ -n "${registry_path}" ]]; then
    mkdir -p "$(dirname "${registry_path}")" 2>/dev/null || true
    export GST_REGISTRY_1_0="${registry_path}"
    echo "[anica-runner] Using isolated host gstreamer registry: ${GST_REGISTRY_1_0}" >&2
  fi

  echo "[anica-runner] Using curated host GStreamer plugins: ${plugin_dir}" >&2
}

rewrite_macos_macho_links_to_runtime() {
  local target_bin="$1"
  local runtime_lib="$2"
  local changed patched_count
  changed=0
  patched_count=0
  if [[ ! -f "${target_bin}" || ! -d "${runtime_lib}" ]]; then
    return 0
  fi
  if ! command -v otool >/dev/null 2>&1 || ! command -v install_name_tool >/dev/null 2>&1; then
    return 0
  fi
  local kind
  kind="$(file -b "${target_bin}" 2>/dev/null || true)"
  if [[ "${kind}" != *"Mach-O"* ]]; then
    return 0
  fi

  local dep base runtime_lib_abs
  runtime_lib_abs="$(cd "${runtime_lib}" && pwd)"
  while IFS= read -r dep; do
    [[ -z "${dep}" ]] && continue
    base="$(basename "${dep}")"
    if [[ -f "${runtime_lib}/${base}" && "${dep}" == /opt/homebrew/* ]]; then
      if install_name_tool -change "${dep}" "@rpath/${base}" "${target_bin}" 2>/dev/null; then
        changed=1
        patched_count=$((patched_count + 1))
      fi
    fi
  done < <(otool -L "${target_bin}" 2>/dev/null | awk 'NR>1 {print $1}')

  # Ensure runtime library folder is resolvable by @rpath.
  if install_name_tool -add_rpath "${runtime_lib_abs}" "${target_bin}" 2>/dev/null; then
    changed=1
  fi

  if [[ "${changed}" == "1" ]] && command -v codesign >/dev/null 2>&1; then
    if ! codesign --force --sign - --timestamp=none "${target_bin}" >/dev/null 2>&1; then
      echo "[anica-runner] WARNING: failed to re-sign patched binary: ${target_bin}" >&2
      return 1
    fi
  fi

  if [[ "${patched_count}" -gt 0 ]]; then
    echo "[anica-runner] Patched ${patched_count} Homebrew dylib link(s): ${target_bin}" >&2
  fi
}

macos_binary_has_homebrew_media_deps() {
  local target_bin="$1"
  if [[ ! -f "${target_bin}" ]] || ! command -v otool >/dev/null 2>&1; then
    return 1
  fi
  otool -L "${target_bin}" 2>/dev/null \
    | awk 'NR>1 {print $1}' \
    | rg -q '^/opt/homebrew/(opt/(gstreamer|glib|gettext)|Cellar/(gstreamer|glib|gettext))/'
}

patch_macos_gstreamer_runtime_tree_once() {
  local runtime_root="$1"
  local runtime_bin="${runtime_root}/bin"
  local runtime_lib="${runtime_root}/lib"
  local plugin_root="${runtime_lib}/gstreamer-1.0"
  local scanner="${runtime_root}/libexec/gstreamer-1.0/gst-plugin-scanner"
  local stamp="${runtime_root}/.rpath_patch_v2.done"

  if [[ ! -d "${runtime_lib}" ]]; then
    return 0
  fi
  if [[ -f "${stamp}" ]]; then
    return 0
  fi
  if ! command -v otool >/dev/null 2>&1 || ! command -v install_name_tool >/dev/null 2>&1; then
    return 0
  fi

  is_macho_file() {
    local f="$1"
    local kind
    kind="$(file -b "${f}" 2>/dev/null || true)"
    [[ "${kind}" == *"Mach-O"* ]]
  }

  # Normalize dylib ids for top-level runtime libs.
  local f dep base
  while IFS= read -r f; do
    is_macho_file "${f}" || continue
    install_name_tool -id "@rpath/$(basename "${f}")" "${f}" 2>/dev/null || true
  done < <(find "${runtime_lib}" -maxdepth 1 -type f -name "*.dylib" 2>/dev/null)

  local -a patch_targets=()
  while IFS= read -r f; do patch_targets+=("${f}"); done < <(find "${runtime_bin}" -maxdepth 1 -type f 2>/dev/null)
  while IFS= read -r f; do patch_targets+=("${f}"); done < <(find "${runtime_lib}" -maxdepth 1 -type f -name "*.dylib" 2>/dev/null)
  while IFS= read -r f; do patch_targets+=("${f}"); done < <(find "${plugin_root}" -type f -name "*.dylib" 2>/dev/null || true)
  if [[ -x "${scanner}" ]]; then
    patch_targets+=("${scanner}")
  fi

  local patched_any
  patched_any=0

  for f in "${patch_targets[@]}"; do
    is_macho_file "${f}" || continue
    while IFS= read -r dep; do
      [[ -z "${dep}" ]] && continue
      base="$(basename "${dep}")"
      if [[ -f "${runtime_lib}/${base}" && "${dep}" != "@rpath/${base}" ]]; then
        if install_name_tool -change "${dep}" "@rpath/${base}" "${f}" 2>/dev/null; then
          patched_any=1
        fi
      fi
    done < <(otool -L "${f}" 2>/dev/null | awk 'NR>1 {print $1}')
  done

  # Add rpaths so @rpath resolves to runtime lib.
  while IFS= read -r f; do
    is_macho_file "${f}" || continue
    if install_name_tool -add_rpath "@executable_path/../lib" "${f}" 2>/dev/null; then
      patched_any=1
    fi
    if install_name_tool -add_rpath "${runtime_lib}" "${f}" 2>/dev/null; then
      patched_any=1
    fi
  done < <(find "${runtime_bin}" -maxdepth 1 -type f 2>/dev/null)

  while IFS= read -r f; do
    is_macho_file "${f}" || continue
    if install_name_tool -add_rpath "@loader_path" "${f}" 2>/dev/null; then
      patched_any=1
    fi
    if install_name_tool -add_rpath "${runtime_lib}" "${f}" 2>/dev/null; then
      patched_any=1
    fi
  done < <(find "${runtime_lib}" -maxdepth 1 -type f -name "*.dylib" 2>/dev/null)

  while IFS= read -r f; do
    is_macho_file "${f}" || continue
    if install_name_tool -add_rpath "@loader_path/.." "${f}" 2>/dev/null; then
      patched_any=1
    fi
    if install_name_tool -add_rpath "${runtime_lib}" "${f}" 2>/dev/null; then
      patched_any=1
    fi
  done < <(find "${plugin_root}" -type f -name "*.dylib" 2>/dev/null || true)

  if [[ -x "${scanner}" ]] && is_macho_file "${scanner}"; then
    if install_name_tool -add_rpath "@executable_path/../../lib" "${scanner}" 2>/dev/null; then
      patched_any=1
    fi
    if install_name_tool -add_rpath "${runtime_lib}" "${scanner}" 2>/dev/null; then
      patched_any=1
    fi
  fi

  if [[ "${patched_any}" == "1" ]] && command -v codesign >/dev/null 2>&1; then
    for f in "${patch_targets[@]}"; do
      is_macho_file "${f}" || continue
      codesign --force --sign - --timestamp=none "${f}" >/dev/null 2>&1 || true
    done
  fi

  touch "${stamp}" 2>/dev/null || true
}

setup_repo_media_runtime() {
  local script_dir repo_root os arch platform runtime_root
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="macos" ;;
    Linux) os="linux" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *) os="other" ;;
  esac

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
  esac

  platform="${os}-${arch}"
  runtime_root=""
  local candidates=(
    "${repo_root}/tools/runtime/${platform}"
    "${repo_root}/tools/runtime/${os}"
    "${repo_root}/tools/runtime/current/${platform}"
    "${repo_root}/tools/runtime/current/${os}"
    "${repo_root}/tools/runtime/current"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ ! -d "${candidate}" ]]; then
      continue
    fi
    if [[ -x "${candidate}/ffmpeg/bin/ffmpeg" || -d "${candidate}/gstreamer/bin" || -d "${candidate}/gstreamer/lib" ]]; then
      runtime_root="${candidate}"
      break
    fi
  done
  if [[ -z "${runtime_root}" ]]; then
    return 0
  fi
  ANICA_RESOLVED_RUNTIME_ROOT="${runtime_root}"

  if [[ -z "${ANICA_MEDIA_RUNTIME_STRICT:-}" ]]; then
    export ANICA_MEDIA_RUNTIME_STRICT=1
  fi
  if [[ -z "${ANICA_ALLOW_SYSTEM_MEDIA:-}" ]]; then
    export ANICA_ALLOW_SYSTEM_MEDIA=0
  fi

  local ffmpeg_bin ffprobe_bin ffmpeg_lib gst_bin gst_lib gst_plugins gst_typelib gst_launch_bin
  local host_gst_launch host_linked_gstreamer
  ffmpeg_bin="${runtime_root}/ffmpeg/bin/ffmpeg"
  ffprobe_bin="${runtime_root}/ffmpeg/bin/ffprobe"
  ffmpeg_lib="${runtime_root}/ffmpeg/lib"
  gst_bin="${runtime_root}/gstreamer/bin"
  gst_lib="${runtime_root}/gstreamer/lib"
  gst_plugins="${gst_lib}/gstreamer-1.0"
  gst_typelib="${gst_lib}/girepository-1.0"
  gst_launch_bin="${gst_bin}/gst-launch-1.0"
  if [[ "${os}" == "windows" && -x "${gst_bin}/gst-launch-1.0.exe" ]]; then
    gst_launch_bin="${gst_bin}/gst-launch-1.0.exe"
  fi
  gst_scanner="${runtime_root}/gstreamer/libexec/gstreamer-1.0/gst-plugin-scanner"
  host_gst_launch="$(command -v gst-launch-1.0 2>/dev/null || true)"
  host_linked_gstreamer=0

  # Prefer host/Brew GStreamer by default for local debugging. Set
  # ANICA_FORCE_VENDORED_GSTREAMER=1 to force vendored runtime selection.
  if [[ -n "${host_gst_launch}" && "${ANICA_FORCE_VENDORED_GSTREAMER:-0}" != "1" ]]; then
    host_linked_gstreamer=1
  fi

  if [[ "${os}" == "macos" ]]; then
    # setup_media_tools already patches the runtime tree. Re-running the full patch
    # on every launch is expensive and can crash older macOS bash builds.
    if [[ "${ANICA_FORCE_GSTREAMER_RUNTIME_RPATH_PATCH:-0}" == "1" ]]; then
      patch_macos_gstreamer_runtime_tree_once "${runtime_root}/gstreamer"
    else
      touch "${runtime_root}/gstreamer/.rpath_patch_v2.done" 2>/dev/null || true
    fi
    # Preemptively rewrite Mach-O links to runtime libs to avoid host/runtime mixing.
    # Do not rewrite the main app binary by default on macOS; mutating target/debug/anica can
    # invalidate code signatures and cause SIGKILL (Code Signature Invalid).
    if [[ "${ANICA_PATCH_MAIN_BINARY_LINKS:-0}" == "1" ]]; then
      rewrite_macos_macho_links_to_runtime "${exe_path}" "${gst_lib}" || true
    fi
    rewrite_macos_macho_links_to_runtime "${gst_bin}/gst-launch-1.0" "${gst_lib}" || true
    rewrite_macos_macho_links_to_runtime "${gst_bin}/gst-inspect-1.0" "${gst_lib}" || true
    rewrite_macos_macho_links_to_runtime "${gst_bin}/gst-typefind-1.0" "${gst_lib}" || true
    rewrite_macos_macho_links_to_runtime "${gst_scanner}" "${gst_lib}" || true
  fi

  # If the built `anica` binary still links to Homebrew media dylibs, forcing
  # vendored runtime env causes mixed-library crashes on macOS.
  if [[ "${os}" == "macos" ]] && command -v otool >/dev/null 2>&1 && [[ -f "${exe_path}" ]]; then
    if macos_binary_has_homebrew_media_deps "${exe_path}"; then
      host_linked_gstreamer=1
      echo "[anica-runner] WARNING: ${exe_path} still links to Homebrew media dylibs; falling back to host GStreamer for this run." >&2
    fi
  fi

  if [[ -x "${ffmpeg_bin}" ]]; then
    export ANICA_FFMPEG_PATH="${ffmpeg_bin}"
    export PATH="$(dirname "${ffmpeg_bin}"):${PATH}"
    if [[ -x "${ffprobe_bin}" ]]; then
      export PATH="$(dirname "${ffprobe_bin}"):${PATH}"
    fi
    echo "[anica-runner] Using vendored ffmpeg runtime: ${ffmpeg_bin}" >&2
  fi

  if [[ "${host_linked_gstreamer}" == "1" && -n "${host_gst_launch}" ]]; then
    export ANICA_GSTREAMER_PATH="${host_gst_launch}"
    if [[ "${ANICA_FORCE_VENDORED_GSTREAMER:-0}" != "1" ]]; then
      echo "[anica-runner] Using host GStreamer (preferred): ${host_gst_launch}" >&2
    else
      echo "[anica-runner] Detected host-linked GStreamer in anica binary; using host GStreamer: ${host_gst_launch}" >&2
    fi
    # Ensure no vendored runtime vars leak into a host-linked process.
    unset GST_PLUGIN_PATH GST_PLUGIN_PATH_1_0 GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_SYSTEM_PATH
    unset GST_PLUGIN_SCANNER GST_PLUGIN_SCANNER_1_0 GI_TYPELIB_PATH GST_REGISTRY_1_0
    if [[ "${os}" == "macos" ]]; then
      configure_curated_host_gstreamer_plugins \
        "${ANICA_HOST_GSTREAMER_PLUGIN_DIR:-}" \
        "${ANICA_HOST_GSTREAMER_REGISTRY:-}" || true
    fi
  else
    if [[ -d "${gst_bin}" ]]; then
      export PATH="${gst_bin}:${PATH}"
    fi
    if [[ -x "${gst_launch_bin}" ]]; then
      export ANICA_GSTREAMER_PATH="${gst_launch_bin}"
      echo "[anica-runner] Using vendored gstreamer runtime: ${gst_launch_bin}" >&2
    fi
    if [[ -d "${gst_plugins}" ]]; then
      # Runtime-isolated plugin paths (do not append host paths).
      export GST_PLUGIN_PATH="${gst_plugins}"
      export GST_PLUGIN_PATH_1_0="${gst_plugins}"
      export GST_PLUGIN_SYSTEM_PATH_1_0="${gst_plugins}"
      export GST_PLUGIN_SYSTEM_PATH="${gst_plugins}"
    fi
    if [[ -x "${gst_scanner}" ]]; then
      export GST_PLUGIN_SCANNER="${gst_scanner}"
      export GST_PLUGIN_SCANNER_1_0="${gst_scanner}"
    fi
    if [[ -d "${gst_typelib}" ]]; then
      export GI_TYPELIB_PATH="${gst_typelib}${GI_TYPELIB_PATH:+:${GI_TYPELIB_PATH}}"
    fi
    if [[ -d "${runtime_root}/gstreamer" ]]; then
      mkdir -p "${runtime_root}/gstreamer/cache" 2>/dev/null || true
      export GST_REGISTRY_1_0="${runtime_root}/gstreamer/cache/registry.bin"
      echo "[anica-runner] Using isolated gstreamer registry: ${GST_REGISTRY_1_0}" >&2
    fi
  fi
  if [[ "${host_linked_gstreamer}" != "1" && -d "${gst_lib}" ]]; then
    if [[ "$os" == "linux" ]]; then
      export LD_LIBRARY_PATH="${gst_lib}${ffmpeg_lib:+:${ffmpeg_lib}}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
    elif [[ "$os" == "macos" && "${ANICA_ENABLE_DYLD_RUNTIME_PATH:-0}" == "1" ]]; then
      # macOS default: keep DYLD_LIBRARY_PATH untouched to avoid overriding
      # system ImageIO/CoreText dependencies (can cause SIGBUS in emoji rasterization).
      # Set ANICA_ENABLE_DYLD_RUNTIME_PATH=1 only for low-level runtime debugging.
      export DYLD_LIBRARY_PATH="${gst_lib}${ffmpeg_lib:+:${ffmpeg_lib}}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}"
    elif [[ "$os" == "macos" ]]; then
      # Safer than DYLD_LIBRARY_PATH for app process, but still lets plugins dlopen bare lib names.
      export DYLD_FALLBACK_LIBRARY_PATH="${gst_lib}${ffmpeg_lib:+:${ffmpeg_lib}}${DYLD_FALLBACK_LIBRARY_PATH:+:${DYLD_FALLBACK_LIBRARY_PATH}}"
    fi
  fi
}

setup_repo_media_runtime

# Skip when disabled explicitly.
if [[ "${ANICA_ACP_AUTO_BUILD:-1}" != "0" ]]; then
  exe_name="$(basename "${exe_path}")"

  # Only validate/build ACP helper before launching main app binary.
  if [[ "${exe_name}" == "anica" ]]; then
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    repo_root="$(cd "${script_dir}/.." && pwd)"
    profile_dir="debug"

    if [[ "${exe_path}" == *"/release/"* ]]; then
      profile_dir="release"
    fi

    candidate_bin_dir="$(cd "$(dirname "${exe_path}")" && pwd)"
    if [[ "$(basename "${candidate_bin_dir}")" == "${profile_dir}" ]]; then
      bin_dir="${candidate_bin_dir}"
      target_dir="$(cd "${bin_dir}/.." && pwd)"
    else
      target_dir="${repo_root}/target"
      bin_dir="${target_dir}/${profile_dir}"
    fi

    acp_bin="${bin_dir}/anica-acp"
    needs_build=0

    if [[ ! -x "${acp_bin}" ]]; then
      needs_build=1
    else
      declare -a tracked_sources=(
        "${repo_root}/Cargo.toml"
        "${repo_root}/Cargo.lock"
        "${repo_root}/src/bin/anica-acp.rs"
      )

      # Support future split modules under src/bin/anica-acp/
      if [[ -d "${repo_root}/src/bin/anica-acp" ]]; then
        while IFS= read -r file; do
          tracked_sources+=("${file}")
        done < <(find "${repo_root}/src/bin/anica-acp" -type f -name "*.rs" | sort)
      fi

      for src in "${tracked_sources[@]}"; do
        if [[ -f "${src}" && "${src}" -nt "${acp_bin}" ]]; then
          needs_build=1
          break
        fi
      done
    fi

    cargo_args=(
      build
      --bin
      anica-acp
      --manifest-path
      "${repo_root}/Cargo.toml"
      --target-dir
      "${target_dir}"
    )
    if [[ "${profile_dir}" == "release" ]]; then
      cargo_args+=(--release)
    fi

    if [[ "${needs_build}" == "1" ]]; then
      echo "[anica-runner] anica-acp stale/missing; rebuilding..." >&2
      cargo "${cargo_args[@]}"
    fi
  fi
fi

run_main_binary_with_lc_rpath_autorepair() {
  local script_dir repo_root stderr_log stderr_pipe_dir stderr_pipe tee_pid
  local first_status os_name fallback_tools_home tools_home
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
  os_name="$(uname -s)"
  case "${os_name}" in
    Darwin) os_name="macos" ;;
    Linux) os_name="linux" ;;
    MINGW*|MSYS*|CYGWIN*) os_name="windows" ;;
    *) os_name="other" ;;
  esac
  fallback_tools_home="${repo_root}/tools/runtime/current/${os_name}"
  tools_home="${ANICA_RESOLVED_RUNTIME_ROOT:-${fallback_tools_home}}"
  stderr_log="$(mktemp "${TMPDIR:-/tmp}/anica-runner-stderr.XXXXXX")"
  stderr_pipe_dir="$(mktemp -d "${TMPDIR:-/tmp}/anica-runner-stderr-pipe.XXXXXX")"
  stderr_pipe="${stderr_pipe_dir}/stderr.pipe"
  mkfifo "${stderr_pipe}"
  tee "${stderr_log}" < "${stderr_pipe}" >&2 &
  tee_pid=$!

  set +e
  "${exe_path}" "$@" 2> "${stderr_pipe}"
  first_status=$?
  set -e
  wait "${tee_pid}" || true
  rm -f "${stderr_pipe}" 2>/dev/null || true
  rmdir "${stderr_pipe_dir}" 2>/dev/null || true

  if [[ "${first_status}" -ne 0 ]] && rg -q "Reason: no LC_RPATH's found" "${stderr_log}"; then
    echo "[anica-runner] Detected missing LC_RPATH runtime link; running ./scripts/setup_media_tools.sh (FFmpeg-only) and retrying once..." >&2
    if ANICA_TOOLS_HOME="${tools_home}" "${repo_root}/scripts/setup_media_tools.sh" --mode local-lgpl --yes; then
      ANICA_PATCH_MAIN_BINARY_LINKS=1 setup_repo_media_runtime
      rm -f "${stderr_log}" 2>/dev/null || true
      exec "${exe_path}" "$@"
    fi
    echo "[anica-runner] WARNING: setup_media_tools.sh failed; keeping original launch error." >&2
  fi

  rm -f "${stderr_log}" 2>/dev/null || true
  exit "${first_status}"
}

if [[ "$(uname -s)" == "Darwin" && "${ANICA_AUTO_MEDIA_SETUP_ON_LC_RPATH_MISS:-1}" != "0" ]]; then
  run_main_binary_with_lc_rpath_autorepair "$@"
else
  exec "${exe_path}" "$@"
fi
