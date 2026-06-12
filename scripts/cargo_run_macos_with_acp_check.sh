#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[anica-runner] macOS development runs are supported on Apple Silicon only." >&2
  exit 1
fi

exec "${script_dir}/cargo_run_with_acp_check.sh" "$@"
