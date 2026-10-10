#!/usr/bin/env bash
# Read the changed paths of a pull request, one path on each line, and print the mise task that CI must run.
# Print `lint` when each path is a Markdown file that no test or build step reads. Else print `check`.
set -euo pipefail

# The Markdown files that a test or build step reads. Add a file here when code starts to read it.
# tests/runtime/interpreter/native.rs reads docs/effect-inventory.md with include_str!.
code_docs=(docs/effect-inventory.md)

paths=$(cat)
# A path that is not Markdown, a Markdown file that code reads, or an empty list needs the full check.
if grep -qv '\.md$' <<<"$paths" || grep -qxF -f <(printf '%s\n' "${code_docs[@]}") <<<"$paths"; then
  echo check
else
  echo lint
fi
