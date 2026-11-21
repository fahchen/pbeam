#!/usr/bin/env bash
set -euo pipefail

# CI runs the full fixture checks separately. The Rust partitions prepare only
# the artifacts they need; local runs still perform the full checks here.
bash scripts/fixture_checks.sh

# Run the OTP gate once, before nextest so that a retry cannot hide an OTP failure.
if [[ -z "${PBEAM_TEST_PARTITION:-}" || "${PBEAM_TEST_PARTITION}" == slice:1/* ]]; then
  otp_args=()
  if [[ "${PBEAM_CI_EVENT:-}" == pull_request ]]; then
    otp_args=(--base "$PBEAM_CI_BASE")
  fi
  mise run otp-check -- "${otp_args[@]}"
fi

frequency_args=()
# Only source-only pull requests use the reduced set. All other events and
# changes run the full set, including changes to fixtures and tool pins.
if [[ "${PBEAM_CI_EVENT:-}" == pull_request ]]; then
  paths=$(git diff --no-renames --name-only "$PBEAM_CI_BASE...$PBEAM_CI_HEAD")
  if [[ -n "$paths" ]] && ! grep -qvE '^src/.*\.rs$' <<<"$paths"; then
    frequency_filter=$(python3 scripts/ci_frequency.py filter)
    frequency_args=(-E "$frequency_filter")
  fi
fi
if [[ ${#frequency_args[@]} -eq 0 ]]; then
  echo 'CI frequency: full test set'
else
  echo 'CI frequency: reduced ERTS-only reference checks'
fi
cargo nextest run --locked --workspace --all-features \
  --status-level slow --final-status-level slow --no-fail-fast \
  "${frequency_args[@]}" \
  ${PBEAM_TEST_PARTITION:+--partition "$PBEAM_TEST_PARTITION"}
# One CI partition checks each inventory against the compiled test list.
if [[ "${PBEAM_TEST_PARTITION:-}" == slice:1/* ]]; then
  (
    list_dir=$(mktemp -d "${TMPDIR:-/tmp}/pbeam-test-classes.XXXXXX")
    trap 'rm -rf "$list_dir"' EXIT
    cargo metadata --locked --no-deps --format-version 1 >"$list_dir/metadata.json"
    cargo nextest list --locked --workspace --all-features --profile ci \
      --message-format json >"$list_dir/tests.json"
    cargo nextest list --locked --workspace --all-features --profile ci \
      --message-format json \
      -E "$(python3 scripts/ci_frequency.py filter)" >"$list_dir/reduced.json"
    python3 scripts/ci_frequency.py verify "$list_dir/tests.json" "$list_dir/reduced.json"
    for scope in feature unit integration; do
      mise exec -- elixir scripts/test_classes.exs --strict --scope "$scope" "$list_dir/metadata.json" "$list_dir/tests.json"
    done
  )
fi
