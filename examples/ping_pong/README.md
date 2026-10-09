# Ping Pong

This example runs `PingPong.main/0` of `ping_pong.ex`. The root process spawns two processes with `spawn/1`. The ping process sends `{:ping, from}` to the pong process, and the pong process answers with `{:pong, from}`, three times. The ping process records the order of the round trips and sends it with the count to the root process. The root process prints the order and then the count.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example ping_pong
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`:

```sh
mise run example ping_pong --erts
```

Each run prints these lines and exits with status 0:

```text
ping 1
pong 1
ping 2
pong 2
ping 3
pong 3
count: 3
```
