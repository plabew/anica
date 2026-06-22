#!/usr/bin/env bash
# =========================================
# =========================================
# scripts/check_ci_local.sh

set -euo pipefail

# Run from the repository root so paths and Cargo workspace resolution match GitHub Actions.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_step() {
  local label="$1"
  shift
  printf '\n==> %s\n' "$label"
  "$@"
}

run_step "Format check" cargo fmt --check
run_step "Cargo check (workspace)" cargo check --workspace
run_step "Clippy (workspace correctness+suspicious strict, perf warn)" \
  cargo clippy --workspace --all-targets -- -D clippy::correctness -D clippy::suspicious -W clippy::perf
run_step "Clippy (MotionLoom public crate, zero warnings)" \
  cargo clippy -p motionloom --all-targets -- -D warnings
run_step "Tests (workspace)" cargo test --workspace

printf '\nLocal CI checks passed.\n'
