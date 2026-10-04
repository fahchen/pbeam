#!/usr/bin/env bash
# Run `cargo test` with the given arguments and print only the results and failures.
set -uo pipefail

# Drop blank lines, `running N tests`, progress dots, and the result lines of test binaries that ran no test.
cargo test --locked --quiet "$@" 2>&1 | awk '
  /^[[:space:]]*$/ || /^running [0-9]+ tests?$/ || /^[.Fi]+( [0-9]+\/[0-9]+)?$/ || /^test result: ok\. 0 passed;/ { next }
  /^test result: / { ran = 1 }
  { print }
  END { if (!ran) print "no test ran" }'
exit "${PIPESTATUS[0]}"
