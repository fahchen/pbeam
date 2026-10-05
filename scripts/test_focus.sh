#!/usr/bin/env bash
# Run `cargo test` with the given arguments and print only the results and failures.
set -uo pipefail

# Drop blank lines, `running N tests`, progress dots, and the result lines of test binaries that ran no test.
# awk exits 1 when no test binary printed a result line.
cargo test --locked --quiet "$@" 2>&1 | awk '
  /^[[:space:]]*$/ || /^running [0-9]+ tests?$/ || /^[.Fi]+( [0-9]+\/[0-9]+)?$/ || /^test result: ok\. 0 passed;/ { next }
  /^test result: / { ran = 1 }
  { print }
  END { exit !ran }'
statuses=("${PIPESTATUS[@]}")
# A build failure or an unknown target also prints no result line; report `no test ran` only when cargo passed.
if [ "${statuses[0]}" -eq 0 ] && [ "${statuses[1]}" -ne 0 ]; then
  echo "no test ran"
fi
exit "${statuses[0]}"
