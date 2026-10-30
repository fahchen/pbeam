#!/usr/bin/env bash
# Run the tests of the CI hangs in a loop and dump the stacks of a run that takes more than 90 s.
set -u
mkdir -p /tmp/dumps
watch() {
  while :; do
    ps -eo pid=,etimes=,args= | while read -r pid secs args; do
      case "$args" in
        *stops_at_a_part*|*charges_the_port_bifs*|*isolated_kernel_boot*|*/pbeam\ erl*|*starts_kernel_and_stops*) ;;
        *) continue ;;
      esac
      [ "$secs" -gt 90 ] || continue
      [ -e "/tmp/dumps/$pid.txt" ] && continue
      { echo "=== $pid ${secs}s $args"; ps -o pid,ppid,etimes,args --ppid "$pid" ; sudo gdb -p "$pid" -batch -ex 'info threads' -ex 'thread apply all bt' 2>&1 | head -1500; } > "/tmp/dumps/$pid.txt"
      echo "DUMPED $pid ($args)"
    done
    sleep 10
  done
}
watch &
export CARGO_LLVM_COV_TARGET_DIR=target/llvm-cov-target
end=$((SECONDS + 55 * 60))
i=0
while [ $SECONDS -lt $end ]; do
  i=$((i + 1))
  echo "--- round $i"
  mise exec -- cargo llvm-cov nextest --no-report --locked --workspace --all-features --status-level fail --final-status-level slow --no-fail-fast \
    -E 'binary(distribution) | binary(port) | (binary(init_boot) & not test(sends_a_file))' 2>&1 | grep -E 'TIMEOUT|SLOW|Summary|FAIL' | head -20
done
ls /tmp/dumps
for f in /tmp/dumps/*.txt; do [ -e "$f" ] && head -400 "$f"; done
! ls /tmp/dumps/*.txt >/dev/null 2>&1
