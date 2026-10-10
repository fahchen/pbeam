#!/usr/bin/env bash
# Print the directory of a Terra source tree at the pinned commit of docs/terra-target.md, after a check of the commit.
# Usage: bash scripts/terra.sh
# PBEAM_TERRA_SRC selects a local Terra git repository. The script does not change it. It stops when HEAD is not the pinned commit, or when lib/ or examples/counter_app.exs has changed, untracked, or ignored files.
# Without PBEAM_TERRA_SRC, the script downloads the pinned commit from GitHub into target/terra/<commit>/ once, and then uses that copy with the same check.
# The script only downloads and reads files. It runs no Terra code.
set -euo pipefail

commit=30dea5667d7e4ae5777aa3df77acb36c4aac6967
url=https://github.com/uminocelo/terra.git

fail() {
  echo "terra: $1" >&2
  exit 2
}

if [[ -n ${PBEAM_TERRA_SRC:-} ]]; then
  src=$PBEAM_TERRA_SRC
  [[ $(git -C "$src" rev-parse --show-toplevel 2>/dev/null) -ef $src ]] || fail "PBEAM_TERRA_SRC $src is not the top directory of a git repository"
else
  src=target/terra/$commit
  if [[ ! -d $src ]]; then
    # Download into a directory of this run, then rename it, so that the copy in target/terra/<commit>/ is never a part of a download. When a parallel run made the copy first, use that copy.
    # The rename is one rename(2) call through file:rename/2 of erl. It fails when target/terra/<commit>/ is a directory that is not empty. A check before mv is not sufficient: when a parallel run makes the copy between the check and mv, mv moves the download into that copy.
    mkdir -p target/terra
    work=$(mktemp -d target/terra/download.XXXXXX)
    trap 'rm -rf "$work"' EXIT
    echo "terra: download $url at $commit into $src" >&2
    git -C "$work" init --quiet
    git -C "$work" fetch --quiet --depth 1 "$url" "$commit"
    git -C "$work" -c advice.detachedHead=false checkout --quiet --detach FETCH_HEAD
    erl -noshell -eval 'halt(case apply(file, rename, init:get_plain_arguments()) of ok -> 0; _ -> 1 end).' -extra "$work" "$src" || rm -rf "$work"
    trap - EXIT
  fi
fi

head=$(git -C "$src" rev-parse --verify --quiet 'HEAD^{commit}') || fail "$src has no commit"
[[ $head == "$commit" ]] || fail "$src is at $head, not at the pinned commit $commit"
changes=$(git -C "$src" status --porcelain --ignored --untracked-files=all -- lib examples/counter_app.exs)
[[ -z $changes ]] || fail "$src has changes in the Terra sources:"$'\n'"$changes"
echo "$src"
