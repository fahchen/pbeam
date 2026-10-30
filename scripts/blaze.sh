#!/usr/bin/env bash
# Install the blaze release that .github/blaze-version names when this machine has another one, then run the blaze command of the arguments.
# With no argument, run the check of the whole workspace that the CI job runs (docs/blaze.md).
set -euo pipefail

cd "$(dirname "$0")/.."

# The installer puts blaze in ${CARGO_HOME:-~/.cargo}/bin, which a non-interactive shell does not have.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# Line 1 is the release. Line 2 is the SHA-256 of the install.sh of that release tag.
version=$(sed -n 1p .github/blaze-version)
installer_sha256=$(sed -n 2p .github/blaze-version)

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d ' ' -f 1
}

installed=$(blaze --version 2>/dev/null | awk '$1 == "blaze" { print $2 }' || true)
if [[ $installed != "$version" ]]; then
  # The installer of the release tag, checked against the pinned hash before it runs: a tag can move, and the
  # installer of ix.dev changes without a release. The installer checks the release archive against the checksums
  # of the release, puts blaze in ${CARGO_HOME:-~/.cargo}/bin, and downloads the Rust that blaze builds with.
  installer=$(mktemp)
  curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$installer" \
    "https://raw.githubusercontent.com/indexable-inc/blaze/v$version/install.sh"
  actual=$(sha256 "$installer")
  if [[ $actual != "$installer_sha256" ]]; then
    rm -f "$installer"
    echo "blaze.sh: the install.sh of v$version has SHA-256 $actual, not $installer_sha256 of .github/blaze-version" >&2
    exit 1
  fi
  sh "$installer" --version "$version"
  rm -f "$installer"
fi

if (($# == 0)); then
  set -- check --workspace --all-targets
fi
exec blaze "$@"
