#!/usr/bin/env bash
# Install the blaze release that .github/blaze-version names when this machine has another one, then run the blaze command of the arguments.
# With no argument, run the check of the whole workspace that the CI job runs (docs/blaze.md).
set -euo pipefail

# The installer puts blaze in ${CARGO_HOME:-~/.cargo}/bin, which a non-interactive shell does not have.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

version=$(cat .github/blaze-version)
installed=$(blaze --version 2>/dev/null | awk '$1 == "blaze" { print $2 }' || true)
if [[ $installed != "$version" ]]; then
  # The installer puts blaze in ${CARGO_HOME:-~/.cargo}/bin and downloads the Rust that blaze builds with.
  curl -fsSL https://ix.dev/blaze | sh -s -- --version "$version"
fi

if (($# == 0)); then
  set -- check --workspace --all-targets
fi
exec blaze "$@"
