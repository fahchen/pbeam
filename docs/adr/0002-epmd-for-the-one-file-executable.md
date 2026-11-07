# ADR 0002: Find nodes without an epmd in the one-file executable

- Status: Accepted
- Date: 2026-11-06
- Deciders: owner, acting through the standing capability-program authorization
- Related: #1403, #1416, #1419, #1390

## Context

`pbeam erl` starts an epmd before the boot for a named node. The default command is `epmd -daemon` from `PATH`. `-epmd Program` replaces that command. `-start_epmd false` and `-no_epmd` disable the start. `start_epmd` in `src/bin/pbeam.rs:672` starts the program, and the run stops with `cannot start epmd -daemon: <Reason>` when the command cannot start. A nonzero exit status is ignored. The pinned ERTS launcher uses `$BINDIR/epmd` (`erlexec.c:1131-1132`, `1327-1372`).

A one-file executable has no sibling epmd binary. A host without OTP can lack epmd. The pinned ERTS supports an `-epmd_module` that replaces `erl_epmd`. Selected external exports are `start_link/0`, `register_node/2,3`, `address_please/3`, `port_please/2,3`, `listen_port_please/2`, and `names/0,1`. The same export list also names `start/0`, `stop/0`, and `open/0,1,2` (`lib/kernel/src/erl_epmd.erl:57-60` at `OTP-29.1.1`).

Redbug CLI runs a connect-only module as `Server.Epmd` (`server/lib/server/epmd.ex` at redbug-cli). The release installs it with `-start_epmd false -epmd_module Elixir.Server.Epmd` (`server/rel/env.sh.eex:13`). The module does not register in a real epmd. Its `address_please/3` returns `{ok, Ip, Port, Version}` for a pinned name, and it falls back to `erl_epmd` for an unpinned name. The module documents an SSH tunnel as a use of a pinned endpoint.

## Decision

- `pbeam erl` keeps its current epmd start and its overrides. This ADR does not change that path.
- A one-file executable uses the pinned OTP `-epmd_module` with a fixed name-to-endpoint map and `-start_epmd false`. A pinned name needs no epmd daemon.
- The module follows the Redbug CLI shape for its callbacks, but it does not copy the creation value. `register_node/2,3` must return a creation that changes for each incarnation of the same node name. Redbug CLI returns a random number from 1 through 3, and PBeam converts a creation below 4 to 4, so a restarted node can get the same creation. `address_please/3` returns `{ok, Ip, Port, Version}` for a pinned name and falls back to `erl_epmd` otherwise.
- PBeam does not add the module or its configuration now. No one-file executable needs distribution yet, so there is no consumer. The work waits in `docs/backlog.md` with its trigger.
- Row "Distribution" of `docs/erts-differences.md` points to this ADR.

## Consequences

- The intended outcome is that a one-file executable reaches a node by name on a host that lacks epmd. No test does this today. Verification needs a distribution connection between two nodes on a host with no epmd.
- Only the node that starts a connection needs the target endpoint in its map. The peer does not need the same map. There is no name discovery by broadcast.
- An unpinned name falls back to `erl_epmd`. Without an epmd, the connection does not start. The failure can come from host resolution or from the name lookup, which then gives `noport`.
- A node that others must find by name needs a fixed published port.
- The future verification must check that identifiers of two incarnations of the same node name stay distinct. A fixed or reused creation makes them indistinguishable.
- No new `unsafe`, no new daemon, and no new protocol implementation.

## Alternatives

- **Require an installed epmd.** Rejected. A host without OTP can lack epmd, so this does not serve the one-file executable.
- **Bundle the OTP epmd with the executable.** The OTP epmd is a C program. Bundling it means a second binary or embedded C code, plus its daemon lifecycle and its build. Rejected. There is no consumer.
- **Use `erl_epmd` with `-erl_epmd_port`.** This gives one fixed port for every node name. Rejected for the one-file executable, because it cannot give a different endpoint per node name.
- **Use only `-no_epmd` and a fixed port.** Rejected. `-no_epmd` needs `-proto_dist` and does not give the name-to-endpoint lookup that the VM calls.

## Evidence

- `src/bin/pbeam.rs:672` starts the epmd of `pbeam erl`; `erlexec.c:1131-1132` and `1327-1372` show the launcher of the pinned ERTS.
- `lib/kernel/src/erl_epmd.erl:57-60` is the external export list of OTP `erl_epmd` at `OTP-29.1.1`.
- `server/lib/server/epmd.ex` and `server/rel/env.sh.eex:13` at redbug-cli show the connect-only module and its installation flags.
- PR #1539 merged as `5c8d29a3` for #1419 proves a TCP echo over an SSH tunnel between PBeam and a pinned-ERTS node with `{packet, line}` framing. Its test starts an epmd, so it gives no evidence about a host without one, and it does not test distribution through the tunnel.
- Row "Distribution" of `docs/erts-differences.md` records the current epmd behavior.
