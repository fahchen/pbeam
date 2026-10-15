#!/usr/bin/env bash
# Run `mise run check` and print only its result. The full output stays in target/check.log.
set -uo pipefail

log=target/check.log
mkdir -p target
mise run check >"$log" 2>&1
status=$?

# Remove ANSI colours and the worktree path.
clean() {
  awk -v root="$(pwd -P)/" '{
    gsub(/\033(\[[0-9;]*[A-Za-z]|\([A-Z0-9])/, "")
    while ((i = index($0, root)) > 0) $0 = substr($0, 1, i - 1) substr($0, i + length(root))
    print
  }'
}

# Under host load, the pinned ERTS process `erl_child_setup` can stop with error 32 (EPIPE) in this pty test (issue #642).
flake=restores_the_terminal_before_the_result_of_terra_counter_as_interactive_erts
flake_message='erl_child_setup: failed with error 32'

# Succeed only when the test task is the only failed task, the nextest final status list after `Summary` holds no failure other than one FAIL of the flaky test, and the test output names the known message.
# The list also holds SLOW lines, which are not failures.
only_known_flake() {
  [[ "$(clean <"$log" | sed -n 's/^\[\([^]]*\)\] ERROR task failed$/\1/p')" == test ]] || return 1
  clean <"$log" | awk -v name="$flake" -v message="$flake_message" '
    index($0, "[test] ") != 1 { next }
    { line = substr($0, 8) }
    index(line, message) { named = 1 }
    line ~ /^ +Summary \[/ { final = 1; next }
    final && line ~ /^ +[A-Z]/ && line !~ /^ +SLOW / {
      failures++
      n = split(line, field, " ")
      flaky = field[1] == "FAIL" && field[n - 1] == "pbeam-runtime::example" && field[n] == name
    }
    END { exit !(named && failures == 1 && flaky) }'
}

# Run the flaky test once more, alone, with the arguments of scripts/test.sh, and check the coverage again.
# --no-clean keeps the profile files of the full run, so the report merges the full run and this run.
retry_flake() {
  echo "check: retried $flake after \`$flake_message\`" | tee -a "$log"
  CARGO_LLVM_COV_TARGET_DIR=target/llvm-cov-target cargo llvm-cov nextest --locked --workspace --all-features --no-clean \
    --status-level slow --final-status-level slow --no-fail-fast \
    --cobertura --output-path target/coverage.xml -E "test(=$flake)" 2>&1 | sed 's/^/[retry] /' >>"$log" &&
    mise exec -- elixir scripts/check_coverage.exs target/coverage.xml 2>&1 | sed 's/^/[retry] /' >>"$log"
}

if ((status != 0)) && only_known_flake; then
  retry_flake
  status=$?
  # The failure report below prints the output of each task that has this line.
  ((status == 0)) || echo '[retry] ERROR task failed' >>"$log"
fi

if ((status == 0)); then
  echo 'check: pass'
  clean <"$log" | grep -o 'Application line coverage: .*'
  exit 0
fi

echo "check: FAIL (exit $status; full log: $log)"
# mise prints `[task] $ <command>` before each step and `[task] ERROR task failed` after a failed task.
# Drop the nextest PASS and START lines (a status level above slow shows them) and the libtest progress lines, which nextest indents in the output of a failed test.
for task in $(clean <"$log" | sed -n 's/^\[\([^]]*\)\] ERROR task failed$/\1/p'); do
  clean <"$log" | awk -v prefix="[$task] " '
    index($0, prefix) == 1 {
      line = substr($0, length(prefix) + 1)
      if (line ~ /^\$ /) n = 0
      step[++n] = line
    }
    END { for (i = 1; i <= n; i++) print step[i] }' |
    grep -Ev '^[[:space:]]*$|^[[:space:]]*(Compiling|Checking|Running) |^[[:space:]]*(PASS|START) |^[[:space:]]*running [0-9]+ tests?$|^[[:space:]]*test .* \.\.\. ok$|^[[:space:]]*test result: ok\.|^ERROR task failed$' |
    head -n 60
done

if grep -q 'application line coverage must be 100%' "$log"; then
  echo 'Uncovered lines:'
  cargo llvm-cov report --show-missing-lines 2>&1 | clean | sed '1,/^Uncovered Lines:$/d'
fi
exit "$status"
