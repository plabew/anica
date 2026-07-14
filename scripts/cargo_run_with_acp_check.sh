#!/usr/bin/env bash
set -euo pipefail

# Cargo runner receives: <compiled-binary> [args...]
# It prepares repo-local FFmpeg and keeps the ACP helper current before launch.
if [[ $# -lt 1 ]]; then
  exit 1
fi

exe_path="$1"
shift || true
ANICA_RESOLVED_RUNTIME_ROOT=""

resolve_runtime_tool_root() {
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

setup_repo_media_runtime() {
  local script_dir repo_root os arch platform runtime_root manifest_path ffmpeg_version
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
  manifest_path="${repo_root}/tools/media_tools_manifest.json"
  ffmpeg_version=""
  if command -v jq >/dev/null 2>&1 && [[ -f "${manifest_path}" ]]; then
    ffmpeg_version="$(jq -r ".common.ffmpeg.version // empty" "${manifest_path}" 2>/dev/null || true)"
  fi

  runtime_root="${repo_root}/tools/runtime/current/${os}"
  [[ -d "${runtime_root}" ]] || return 0

  local ffmpeg_exe="ffmpeg" ffprobe_exe="ffprobe"
  if [[ "${os}" == "windows" ]]; then
    ffmpeg_exe="ffmpeg.exe"
    ffprobe_exe="ffprobe.exe"
  fi

  local ffmpeg_root ffmpeg_bin ffprobe_bin ffmpeg_lib
  ffmpeg_root="$(resolve_runtime_tool_root "${runtime_root}/ffmpeg" "${ffmpeg_exe}" "${ffmpeg_version}" || true)"
  [[ -n "${ffmpeg_root}" ]] || return 0

  ANICA_RESOLVED_RUNTIME_ROOT="${runtime_root}"
  export ANICA_MEDIA_RUNTIME_STRICT="${ANICA_MEDIA_RUNTIME_STRICT:-1}"
  export ANICA_ALLOW_SYSTEM_MEDIA="${ANICA_ALLOW_SYSTEM_MEDIA:-0}"
  export ANICA_TOOLS_HOME="${runtime_root}"

  ffmpeg_bin="${ffmpeg_root}/bin/${ffmpeg_exe}"
  ffprobe_bin="${ffmpeg_root}/bin/${ffprobe_exe}"
  ffmpeg_lib="${ffmpeg_root}/lib"
  if [[ -x "${ffmpeg_bin}" ]]; then
    export ANICA_FFMPEG_PATH="${ffmpeg_bin}"
    export PATH="$(dirname "${ffmpeg_bin}"):${PATH}"
    if [[ -x "${ffprobe_bin}" ]]; then
      export ANICA_FFPROBE_PATH="${ffprobe_bin}"
      export PATH="$(dirname "${ffprobe_bin}"):${PATH}"
    fi
    echo "[anica-runner] Using vendored ffmpeg runtime: ${ffmpeg_bin}" >&2
  fi
  if [[ "${os}" == "linux" && -d "${ffmpeg_lib}" ]]; then
    export LD_LIBRARY_PATH="${ffmpeg_lib}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
  elif [[ "${os}" == "macos" && -d "${ffmpeg_lib}" ]]; then
    export DYLD_FALLBACK_LIBRARY_PATH="${ffmpeg_lib}${DYLD_FALLBACK_LIBRARY_PATH:+:${DYLD_FALLBACK_LIBRARY_PATH}}"
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

  if [[ "${first_status}" -ne 0 ]] && grep -q "Reason: no LC_RPATH's found" "${stderr_log}"; then
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
