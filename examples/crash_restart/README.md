# Crash and Restart

This example runs `CrashRestart.main/0` of `crash_restart.ex`. The process sets `trap_exit`, so an exit signal from a linked process comes to it as a message. It links a worker with `spawn_link/1`. The worker raises a `RuntimeError`. The process receives `{:EXIT, pid, {%RuntimeError{}, stacktrace}}`, prints the message of the error, and starts a new worker. After the fourth crash, it stops.

ERTS also logs an error report for each crash. The report has a time stamp, and the `logger` processes write it, so its text and its position in the output change on each run. The example sets the primary log level `none` with `:logger.set_primary_config/2`, so the output is the same on each run.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example crash_restart
```

Add `--erts` to run the same entry on the pinned ERTS with `erl -noshell`:

```sh
mise run example crash_restart --erts
```

Each command prints these lines and exits with status 0:

```text
start worker 1
exit: worker 1 crashed
restart
start worker 2
exit: worker 2 crashed
restart
start worker 3
exit: worker 3 crashed
restart
start worker 4
exit: worker 4 crashed
stop after 4 crashes
```
