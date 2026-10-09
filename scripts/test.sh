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
  cargo llvm-cov nextest --locked --workspace --all-features \
    --status-level slow --final-status-level slow --no-fail-fast \
    --cobertura --output-path target/coverage.xml
  mise exec -- elixir scripts/check_coverage.exs target/coverage.xml
fi
