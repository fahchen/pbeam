#!/usr/bin/env bash
set -euo pipefail

# Only the exact empty bootstrap is exempt; any Rust source addition enables coverage.
rust_sources=$(git ls-files --cached --others --exclude-standard -- '*.rs')
if [[ "$rust_sources" == 'src/lib.rs' ]] &&
  [[ "$(cat src/lib.rs)" == '//! Portable BEAM runtime. Implementation begins in subsequent task PRs.' ]]; then
  bash scripts/elixir_checks.sh
  cargo test --locked --workspace --all-features
  echo 'Line coverage: N/A (empty bootstrap; no executable Rust code).'
else
  # nextest runs each test in its own process, in parallel across the test binaries.
  # The slow status level prints only slow and failed tests, not a line for each passed test.
  # --no-fail-fast runs all tests after a failure, so the output shows each failure.
  # By default, cargo-llvm-cov runs `cargo clean -p pbeam-runtime`, which removes the incremental state of the crate.
  # --no-clean keeps that state, so rustc reuses the unchanged code of the last local run.
  # The profile files of the last run must go, or their counts would cover lines that no test runs now.
  # CARGO_TARGET_DIR or a cargo config can move the coverage directory.
  # The pin makes cargo-llvm-cov use the directory that the cleanup below removes files from.
  cov_dir=target/llvm-cov-target
  export CARGO_LLVM_COV_TARGET_DIR=$cov_dir
  cargo llvm-cov clean --profraw-only
  rm -f "$cov_dir"/*.profdata
  # A change of the profile or the features gives the test executables new file names, and the old executables stay.
  # cargo-llvm-cov reports each executable whose name matches a workspace target, so an old executable would add uncovered lines.
  # Each executable of the workspace has no extension; dependency files such as proc-macro libraries have one and stay.
  # rustc links the executables again from the incremental state.
  deps=$cov_dir/debug/deps
  if [[ -d "$deps" ]]; then
    find "$deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -delete
  fi
  # CI sets PBEAM_TEST_PARTITION, such as slice:1/4, to run one part of the tests in each job. The report then covers only that part, and the last CI job checks the reports of all parts together.
  # The Elixir checks run while cargo builds the tests, because they use other cores for most of the build. The filter none() builds and lists the tests but runs none of them. The Rust tests read the Elixir outputs, so they start only after the Elixir checks pass.
  elixir_log=target/elixir-checks.log
  mkdir -p target
  bash scripts/elixir_checks.sh >"$elixir_log" 2>&1 &
  elixir_pid=$!
  cargo llvm-cov nextest --locked --workspace --all-features --no-clean --no-report \
    -E 'none()' --no-tests=pass
  elixir_status=0
  wait "$elixir_pid" || elixir_status=$?
  cat "$elixir_log"
  if ((elixir_status != 0)); then
    exit "$elixir_status"
  fi
  cargo llvm-cov nextest --locked --workspace --all-features --no-clean \
    --status-level slow --final-status-level slow --no-fail-fast \
    ${PBEAM_TEST_PARTITION:+--partition "$PBEAM_TEST_PARTITION"} \
    --cobertura --output-path target/coverage.xml
  if [[ -z "${PBEAM_TEST_PARTITION:-}" ]]; then
    mise exec -- elixir scripts/check_coverage.exs target/coverage.xml
  fi
fi
