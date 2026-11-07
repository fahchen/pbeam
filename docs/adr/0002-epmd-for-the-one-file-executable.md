# ADR 0002: Find nodes without an epmd in the one-file executable

- Status: Accepted
- Date: 2026-11-06
- Deciders: owner, acting through the standing capability-program authorization
- Related: #1403, #1416, #1419, #1390

## Context

`pbeam erl` starts `epmd -daemon` from `PATH` before the boot when a node has a name. It reads `-name`, `-sname`, `-proto_dist`, `-start_epmd`, and `-no_epmd` as `erlexec` does. Without an `epmd` in `PATH`, the run stops with `cannot start epmd -daemon: <Reason>` (`erlexec.c:1131-1132`, `1327-1372`; row "Distribution" of `docs/erts-differences.md`).

A one-file executable has no sibling `epmd` binary. A host without OTP has no `epmd`. PR #1539 merged as `5c8d29a3` for #1419 added an SSH tunnel between a PBeam node and a pinned-ERTS peer through a local port. It needs no daemon on the host.

The pinned ERTS supports an `-epmd_module` that replaces `erl_epmd`. Its external export list gives `start_link/0`, `register_node/2,3`, `address_please/3`, `port_please/2,3`, `listen_port_please/2`, and `names/0,1` (`lib/kernel/src/erl_epmd.erl:57-60`). Redbug CLI runs such a module as `Server.Epmd` (`server/lib/server/epmd.ex` at redbug-cli). It installs the module with `-start_epmd false -epmd_module Elixir.Server.Epmd`, never registers in a real epmd, and returns a pinned `{ok, Ip, Port, Version}` from `address_please/3`. The VM uses that four-tuple for a direct dial, so no `port_please` round and no epmd daemon are needed. Its measured path works over an SSH tunnel.

## Decision

- `pbeam erl` keeps the `epmd` from `PATH`. This matches `erlexec` and stays unchanged.
- A one-file executable uses the pinned OTP `-epmd_module` with a fixed name-to-endpoint map and `-start_epmd false`. It starts no epmd daemon.
- The module follows the Redbug CLI shape: `register_node/2,3` returns a creation without a daemon, `address_please/3` returns `{ok, Ip, Port, Version}`, and an unpinned name falls back to `erl_epmd`.
- PBeam does not add the module or its configuration now. No one-file executable needs distribution yet, so there is no consumer. The work waits in `docs/backlog.md` with its trigger.
- Row "Distribution" of `docs/erts-differences.md` points to this ADR.

## Consequences

- When the module is built, a one-file executable will reach a node by name on a host with no OTP installed. No such module exists yet.
- Two nodes must agree on the endpoint map. There is no name discovery by broadcast.
- A node that others must find by name needs a fixed published port.
- The SSH tunnel of PR #1539 keeps its pinned endpoint and needs no epmd change.
- No new `unsafe`, no new daemon, and no new protocol implementation.

## Alternatives

- **Require an installed `epmd`.** Rejected. A host without OTP has no `epmd`, so this breaks the one-file executable.
- **Put an `epmd` inside the executable.** This needs a Rust daemon or a copy of the OTP Erlang code, plus a daemon lifecycle and a second protocol implementation. Rejected. There is no consumer, and the repository rule keeps the pinned OTP module code.
- **Use only `-no_epmd` and a fixed port.** Rejected. `-no_epmd` needs `-proto_dist` and does not give the name-to-endpoint lookup that the VM calls.

## Evidence

- `server/lib/server/epmd.ex` at redbug-cli shows the connect-only module and its `-start_epmd false -epmd_module` flags.
- `lib/kernel/src/erl_epmd.erl:57-60` is the export list of the module at `OTP-29.1.1`.
- `erlexec.c:1131-1132` and `1327-1372` show the daemon start of `pbeam erl`.
- PR #1539 merged as `5c8d29a3` proved an SSH tunnel between PBeam and a pinned-ERTS peer with `{packet, line}` framing.
- Row "Distribution" of `docs/erts-differences.md` records the current `PATH` behavior.
