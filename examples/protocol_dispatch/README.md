# Protocol Dispatch

This example runs `ProtocolDispatch.main/0` of `protocol_dispatch.ex`. The module defines the struct `ProtocolDispatch.Point` and implements the protocols `String.Chars` and `Inspect` for it. `main/0` prints whether the two protocols are consolidated, calls each protocol with a point, and then calls `String.Chars` with an atom. The atom goes to the implementation `String.Chars.Atom` of Elixir.

The task consolidates the protocols of Elixir, as `mix run` does (see the [Hello World example](../hello_world/README.md)). A consolidated protocol finds the implementation for a value with a pattern match. An unconsolidated protocol calls `Code.ensure_compiled/1`, which calls `code:ensure_loaded/1`. The `pbeam` binary has no code server, so this call stops a run with `unresolved external MFA code:ensure_loaded/1`.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example protocol_dispatch
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`:

```sh
mise run example protocol_dispatch --erts
```

Each run prints these lines and exits with status 0:

```text
String.Chars consolidated: true
Inspect consolidated: true
to_string of a point: (1, 2)
inspect of a point: #Point<1, 2>
to_string of an atom: ok
```
