#!/usr/bin/env bash
set -euo pipefail

(cd fixtures/hello && mix test --cover --warnings-as-errors)

# Only the exact empty bootstrap is exempt; any Rust source addition enables coverage.
rust_sources=$(git ls-files --cached --others --exclude-standard -- '*.rs')
if [[ "$rust_sources" == 'src/lib.rs' ]] &&
  [[ "$(cat src/lib.rs)" == '//! Portable BEAM runtime. Implementation begins in subsequent task PRs.' ]]; then
  cargo test --locked --workspace --all-features
  echo 'Line coverage: N/A (empty bootstrap; no executable Rust code).'
else
  # nextest runs each test in its own process, in parallel across the test binaries.
  # The slow status level prints only slow and failed tests, not a line for each passed test.
  # --no-fail-fast runs all tests after a failure, so the output shows each failure.
  # By default, cargo-llvm-cov runs `cargo clean -p pbeam-runtime`, which removes the incremental state of the crate.
  # --no-clean keeps that state, so rustc reuses the unchanged code of a restored CI cache or of the last local run.
  # The profile files of the last run must go, or their counts would cover lines that no test runs now.
  cargo llvm-cov clean --profraw-only
  rm -f target/llvm-cov-target/*.profdata
  # A change of the profile or the features gives the test executables new file names, and the old executables stay.
  # cargo-llvm-cov reports each executable whose name matches a workspace target, so an old executable would add uncovered lines.
  # Each executable of the workspace has no extension; dependency files such as proc-macro libraries have one and stay.
  # rustc links the executables again from the incremental state.
  deps=target/llvm-cov-target/debug/deps
  if [[ -d "$deps" ]]; then
    find "$deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -delete
  fi
  cargo llvm-cov nextest --locked --workspace --all-features --no-clean \
    --status-level slow --final-status-level slow --no-fail-fast \
    --cobertura --output-path target/coverage.xml
  mise exec -- elixir scripts/check_coverage.exs target/coverage.xml
fi
