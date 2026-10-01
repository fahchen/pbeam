#!/usr/bin/env bash
set -euo pipefail

if [[ -n "$(git ls-files --cached --others --exclude-standard -- '*.ex' '*.exs')" ]]; then
  echo 'Add Elixir lint, tests, and 100% coverage gates before introducing Elixir source.' >&2
  exit 1
fi

# Only the exact empty bootstrap is exempt; any Rust source addition enables coverage.
rust_sources=$(git ls-files --cached --others --exclude-standard -- '*.rs')
if [[ "$rust_sources" == 'src/lib.rs' ]] &&
  [[ "$(cat src/lib.rs)" == '//! Portable BEAM runtime. Implementation begins in subsequent task PRs.' ]]; then
  cargo test --locked --workspace --all-features
  echo 'Line coverage: N/A (empty bootstrap; no executable Rust code).'
else
  cargo llvm-cov --locked --workspace --all-features --fail-under-lines 100
fi
