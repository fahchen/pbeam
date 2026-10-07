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

if ((status == 0)); then
  echo 'check: pass'
  clean <"$log" | grep -o 'Application line coverage: .*'
  exit 0
fi

echo "check: FAIL (exit $status; full log: $log)"
# mise prints `[task] $ <command>` before each step and `[task] ERROR task failed` after a failed task.
for task in $(clean <"$log" | sed -n 's/^\[\([^]]*\)\] ERROR task failed$/\1/p'); do
  clean <"$log" | awk -v prefix="[$task] " '
    index($0, prefix) == 1 {
      line = substr($0, length(prefix) + 1)
      if (line ~ /^\$ /) n = 0
      step[++n] = line
    }
    END { for (i = 1; i <= n; i++) print step[i] }' |
    grep -Ev '^[[:space:]]*$|^[[:space:]]*(Compiling|Checking|Running) |^running [0-9]+ tests?$|^test .* \.\.\. ok$|^test result: ok\.|^ERROR task failed$' |
    head -n 60
done

if grep -q 'application line coverage must be 100%' "$log"; then
  echo 'Uncovered lines:'
  cargo llvm-cov report --show-missing-lines 2>&1 | clean | sed '1,/^Uncovered Lines:$/d'
fi
exit "$status"
