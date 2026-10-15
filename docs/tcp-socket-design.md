# TCP socket design for gen_tcp

Status: decision record. Task: [#663](https://github.com/fahchen/pbeam/issues/663). Epic: [#651](https://github.com/fahchen/pbeam/issues/651). Milestone tracker: [#650](https://github.com/fahchen/pbeam/issues/650). The described revision is `4ceb46d0`, the head of `origin/main` at the time of writing. This document changes no runtime behavior.

Each OTP citation is a file and line at the pinned tag `OTP-29.1.1` (`mise.toml`). Print a cited range with `mise run otp-src <path> <start> <end>`. Thousand Island and Bandit citations refer to the versions of `examples/plug_hello/mix.lock`: `thousand_island` 1.5.0 and `bandit` 1.12.5.

## The problem

Bandit listens and accepts through Thousand Island, and Thousand Island uses `gen_tcp` and `inet` (`thousand_island/lib/thousand_island/transports/tcp.ex`). The pinned OTP has two backends for `gen_tcp`. The `inet` backend opens the `tcp_inet` port driver through `prim_inet`. The `socket` backend runs `gen_tcp_socket` on the `prim_socket` NIF. PBeam has neither: it returns a runtime error for `spawn_driver` ports (`docs/erts-differences.md`, row "Ports"), and it loads no NIF library (row "NIF stubs"). This document selects one backend and lists the tasks that implement it. In both options, the pinned OTP Erlang code runs unchanged; only the driver or the NIF is new PBeam code.

## What Thousand Island and Bandit call

| Call | Use |
| --- | --- |
| `gen_tcp:listen/2` | Options `mode: binary`, `active: false`, `backlog: 1024`, `nodelay: true`, `send_timeout: 30000`, `send_timeout_close: true`, `reuseaddr: true`, and the user options; an `inet_backend` option moves to the front (`transports/tcp.ex:44-75`). One listen socket by default (`server_config.ex:35`). |
| `inet:sockname/1` | The listener reads the local address (`listener.ex:72`). |
| `gen_tcp:accept/1` | 100 acceptor processes call it on one listen socket (`server_config.ex:34`, `acceptor.ex:34`). |
| `gen_tcp:controlling_process/2` | The connection supervisor gives each socket to its handler (`connection.ex:65`). |
| `inet:setopts/2` | `active: :once` after each data message (`handler.ex:583-634`). |
| `{tcp, S, Data}`, `{tcp_closed, S}`, `{tcp_error, S, R}` | The handler receives them in `handle_info/2` (`handler.ex:404-428`). |
| `gen_tcp:recv/3`, `gen_tcp:send/2` | Bandit reads the rest of a request and writes the response (5 calls each of `ThousandIsland.Socket.recv` and `send` in `bandit/lib`). |
| `inet:peername/1`, `inet:getstat/1`, `gen_tcp:close/1` | The handler reads the peer and the counts, and closes (`handler.ex:141, 535`). |

Bandit does not call `gen_tcp:shutdown/2` or `sendfile` on the hello path. The run of `GET /` does not use `:inet.getaddr/2` when the options have no host name.

## How OTP selects the backend

`gen_tcp:listen/2` calls `inet:gen_tcp_module/1` (gen_tcp.erl:701-715). A first option `{inet_backend, Flag}` selects the backend. Else the backend is `persistent_term:get({kernel, inet_backend}, inet)` (inet.erl:330, 3289-3300). `inet_db:init/1` puts that persistent term only when the kernel environment `inet_backend` is `inet` or `socket` (inet_db.erl:941-947). Thus without configuration the pinned ERTS uses `inet`. A probe on the pinned ERTS confirms it: `persistent_term:get({kernel, inet_backend}, none)` gives `none`, the listen socket is a port with the name `"tcp_inet"`, and an `{active, once}` socket delivers `{tcp, #Port<0.5>, <<"hi">>}`.

Other OTP code opens the driver directly, independent of the backend. At kernel start, `inet_db:start_link/0` calls `inet_config:init/0` (inet_db.erl:116, 123), which opens a `udp_inet` port with `inet_udp:open/2` to read the host name (inet_config.erl:52-53, 223-231). The distribution module `inet_tcp_dist` uses `inet_tcp` (inet_tcp_dist.erl:53). With the `inet` backend, `inet:gethostname/0` also opens a `udp_inet` port (inet.erl:2436-2457).

## Option A: the tcp_inet port driver

`prim_inet:open/6` opens `{spawn_driver, "tcp_inet"}` with `[binary]` (prim_inet.erl:99-112). Each request is `erlang:port_control(S, Cmd + 16#03f1a300, Args)` with a byte reply: `[1 | Data]` for OK, `[0 | Reason]` for an error, with the error name as bytes (prim_inet.erl:2969-2975; inet_drv.c:869-871). Asynchronous requests reply with `{inet_async, S, Ref, Status}` (prim_inet.erl:488-535, 808-815). A send makes a port monitor, puts `term_to_binary(Mref, [local])` before the data, calls `erlang:port_command/3`, and waits for `{inet_reply, S, Mref}` or `{inet_reply, S, Status, Mref}` (prim_inet.erl:575-616).

Observable behavior to match: the reply bytes of each request (inet_drv.c:10988-11530, 11952-12400); the option codec of `INET_REQ_SETOPTS` and `INET_REQ_GETOPTS` (inet_drv.c:6871, 9071); the queue of waiting acceptors with a monitor and a timer for each (inet_drv.c:12144-12258, 12600, 12732); a new port for each accepted connection, owned by the acceptor (inet_drv.c:11785-11834); the data messages of active mode (inet_drv.c:3987, 13042, 13461); and the output queue (inet_drv.c:12629-12731, 14234). `prim_inet:close/1` with the default `linger` sends `INET_REQ_SUBSCRIBE` for `subs_empty_out_q` and, when output is queued, waits for `{empty_out_q, S}` before it closes the port (prim_inet.erl:175-214, 1296-1299).

Surface for the Thousand Island path: 11 of the 31 requests that `prim_inet` sends (`INET_REQ_OPEN`, `SETOPTS`, `GETOPTS`, `BIND`, `LISTEN`, `NAME`, `PEER`, `ACCEPT`, `GETSTAT`, `SUBSCRIBE`, and `TCP_REQ_RECV`), the port output, active delivery in `{packet, raw}` mode, and about 10 options. Port BIFs that PBeam does not run yet: `erlang:port_control/3`, `erlang:port_info/2` (`connected`), `erlang:port_connect/2`, `erlang:port_set_data/2`, `erlang:port_get_data/1` (inet_db.erl:850-859), `link/1` and `unlink/1` with a port, and `erlang:monitor(port, S)` (inet.erl:4595-4640; prim_inet.erl:580).

## Option B: the prim_socket NIF

`gen_tcp_socket` starts one `gen_statem` server for each socket (gen_tcp_socket.erl:25, 417, 530, 1507-1530) and calls the `socket` module, which calls `prim_socket`. The load of `prim_socket` starts the `socket_registry` system process with `erts_internal:spawn_system_process/3`, loads the NIF, and fills the `supports` tables of protocols, options, and ioctl requests (prim_socket.erl:134-190). The NIF table has 40 entries (prim_socket_nif.c:18696). The Thousand Island path needs about 16 of them: `nif_open`, `nif_bind`, `nif_listen`, `nif_accept`, `nif_recv`, `nif_send`, `nif_sendv`, `nif_setopt`, `nif_getopt`, `nif_sockname`, `nif_peername`, `nif_close`, `nif_finalize_close`, `nif_cancel`, `nif_info`, and `nif_supports`, with the `{'$socket', S, select, Ref}` messages of a select and the monitor of each owner.

Selection without a change to OTP code: a first listen option `{inet_backend, socket}`, which Thousand Island keeps at the front of its transport options (`transports/tcp.ex:68-75`); or the kernel environment `inet_backend` set to `socket` in the release configuration, or `-kernel inet_backend socket` on the command line, which PBeam does not parse (row "Run arguments"). Each form changes the configuration of the example, and the ERTS reference run must use the same configuration. PBeam cannot make `socket` its default: that is a behavior difference from ERTS.

## Comparison

| Criterion | A: `tcp_inet` driver | B: `prim_socket` NIF |
| --- | --- | --- |
| Configuration | None. It is the pinned default. | A listen option `{inet_backend, socket}` in the transport options of the example, or the kernel environment `inet_backend = socket`, in the example and in the ERTS reference run. |
| New PBeam code | The driver requests, the option byte codec, and the accept queue, active delivery, and output queue of `inet_drv.c`. | 16 NIFs, the select messages, owner monitors, NIF load, and `erts_internal:spawn_system_process/3`. The `gen_tcp_socket` server does the buffering and active modes in Erlang. |
| Other runtime work | Driver ports and 8 port BIFs that PBeam does not run yet. | A `gen_statem` server for each socket, and the `socket` module (8185 lines) and `gen_tcp_socket` (3455 lines) on PBeam. |
| Host I/O | The fds of the sockets join the idle wait of the scheduler, as the pipes of the ports do. | The same wait, with select messages in place of driver input. |
| Testability | Each request is a direct `erlang:port_control/3` call on the pinned ERTS with exact bytes, and `gen_tcp` results compare at the API level. | Each NIF is a direct `prim_socket` call on the pinned ERTS with terms, and `gen_tcp` results compare at the API level. |
| Other OTP users | Kernel start (`udp_inet`), `inet:gethostname/0`, `inet_tcp_dist`, `gen_udp`, and `ssl` over `gen_tcp`. | The `socket` and `net` modules. Kernel start and distribution still need the driver. |

## Decision

Select option A, the `tcp_inet` port driver under the pinned `prim_inet`.

- It is the default of the pinned ERTS, so the example and its ERTS reference run need no configuration, and each `gen_tcp` socket is a port as on ERTS.
- PBeam needs the `inet` driver family in all cases: kernel start opens `udp_inet` (inet_config.erl:223-231), and `inet_tcp_dist` uses `inet_tcp`. Option B adds a second socket stack and does not remove that need.
- The Erlang path is short: no server process for each socket, no `socket_registry`, and no NIF load. The PBeam port table, port owner, and idle wait of port pipes (`src/runtime/port.rs`) already give the shape of a driver port.

Tradeoffs: the port control protocol is internal to OTP, so the tasks follow the bytes of the pinned `inet_drv.c` and do not support another OTP version. The driver logic (accept queue, active modes, output queue) is Rust code that follows `inet_drv.c`; the tasks implement only `{packet, raw}` and the options of the measured path, and each other packet type, option, and request returns a clear runtime error with a row in `docs/erts-differences.md`. Option B stays possible later for code that sets `inet_backend` to `socket`; record it in `docs/backlog.md` when a consumer exists.

## Host I/O model

PBeam has one scheduler thread. It waits on host fds only when no process can run (row "Input select"). Each socket is non-blocking. A listen fd and a connected fd with a pending `accept`, `recv`, or active read join the read set of that wait. A connected fd with queued output joins a write set. When the wait ends, the scheduler runs the driver step of each ready fd on the scheduler thread, and the step sends the `inet_async`, `inet_reply`, or `tcp` message. As with port pipes, a ready socket is not seen while a process can run; record this in the row "Input select". A timer of an `accept` or `recv` is a PBeam timer, so the wait ends at its end.

New code stays in safe Rust. `std::net` sets `SO_REUSEADDR` at bind, uses a backlog that the caller cannot set, and has no stable call for `SO_SNDBUF` or `SO_LINGER`, so it cannot give the ERTS option results. Task #665 adds the `socket2` crate for these socket calls and records the decision. Task #667 adds a write set to the existing `select` call of `host::input::wait_readable` and adds no `unsafe` block.

## Capability checks

M6 is postponed. In a run without a policy, each check is skipped (row "Capability checks"), so the example runs with ambient authority. No network capability kind exists yet (`docs/capability-security-design.md:89`). In a run with a policy, the open of a `tcp_inet` port returns a clear runtime error until M6 adds a network capability that checks the listen address and port before the bind, as `docs/capability-security-migration.md:593` requires.

## Implementation tasks

Each task is one PR under #651, in this order. Each task blocks the next, and each blocks #664. Each task compares a compiled fixture on the pinned ERTS and on PBeam, with a client of the Rust test for the connection side.

1. [#665](https://github.com/fahchen/pbeam/issues/665) Open a tcp_inet port and listen on a TCP address: the driver port kind, `erlang:port_control/3`, `OPEN`, `SETOPTS`, `GETOPTS`, `BIND`, `LISTEN`, `NAME`, port data, and the close of a listen port with `SUBSCRIBE` for an empty output queue. Acceptance: `gen_tcp:listen/2`, `inet:sockname/1`, `inet:getopts/2`, and `gen_tcp:close/1` values, and the reply bytes of each request, equal the pinned ERTS.
2. [#666](https://github.com/fahchen/pbeam/issues/666) Accept TCP connections in many processes on one listen socket: `ACCEPT` with the queue, timer, and monitor of each acceptor, the accepted port, the listen fd in the idle wait, and `PEER`. Before `prim_inet:accept/2,3` returns, `accept_opts/3` reads the options `active`, `nodelay`, `keepalive`, `delay_send`, `priority`, `linger`, and `reuseaddr` of the listen socket, and the family options `tos`, `ttl`, `recvtos`, and `recvttl`, and sets them on the accepted socket (prim_inet.erl:502-518; inet_tcp.erl:233-249). This task adds the `GETOPTS` and `SETOPTS` codec of each of these options that #665 does not add. Acceptance: accept results, `peername` values, the options of the accepted socket, and `{error, timeout}` equal the pinned ERTS with several acceptors.
3. [#667](https://github.com/fahchen/pbeam/issues/667) Receive and send on a TCP socket in passive mode: `TCP_REQ_RECV`, the port output with its monitor header and `inet_reply`, the output queue with the write wait, and `GETSTAT`. Acceptance: recv and send results, messages, and counts equal the pinned ERTS.
4. [#668](https://github.com/fahchen/pbeam/issues/668) Deliver TCP data in active once mode and move the socket owner: the `active` option, the `tcp` messages, `erlang:port_info/2`, `erlang:port_connect/2`, and links with a port. Acceptance: messages and values of `controlling_process/2` and `{active, once}` equal the pinned ERTS.
5. [#669](https://github.com/fahchen/pbeam/issues/669) Close a TCP connection from each side: the close of a connected port with queued output (`SUBSCRIBE` and `{empty_out_q, S}`), the exit of the owner, and the peer close. Acceptance: values, messages, and client-side bytes equal the pinned ERTS.

The kernel start path of #449 opens `udp_inet` with `INET_REQ_GETHOSTNAME`. Task #665 gives the driver port foundation; the task that owns kernel start adds the `udp_inet` requests when its run reaches them.
