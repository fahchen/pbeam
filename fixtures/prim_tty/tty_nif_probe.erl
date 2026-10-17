-module(tty_nif_probe).
-export([isatty/0, create/0, window_size/0, setlocale/0, setupterm/0, terminfo/0, extras/0,
         init/0, monitor/0,
         owner/1, holder/3, select/0, encoding/0, select_monitor/0, selector/1, link/0,
         linker/1, linked_owner/1, root_exit/0, root_free/0, write/0, write_error/0, read/0,
         read_error/0, loop/0, loop_reader/2]).

isatty() ->
    [catch_error(fun() -> prim_tty:isatty(A) end)
     || A <- [stdin, stdout, stderr, foo, 1, make_ref()]].

create() ->
    {ok, _} = prim_tty:tty_create(stdout),
    {ok, _} = prim_tty:tty_create(stderr),
    [catch_error(fun() -> prim_tty:tty_create(A) end) || A <- [stdin, foo, 1, make_ref()]].

window_size() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    {ok, Err} = prim_tty:tty_create(stderr),
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    [catch_error(fun() -> prim_tty:tty_window_size(A) end)
     || A <- [Out, Err, foo, make_ref(), Re]].

setlocale() ->
    [prim_tty:setlocale(A) || A <- [foo, make_ref()]].

setupterm() ->
    [prim_tty:setupterm_nif(), prim_tty:setupterm_nif()].

%% setupterm_nif/0 reads the terminfo entry of TERM first. Each term gives
%% {tigetnum_nif/1, tigetflag_nif/1}: termcap codes as prim_tty gives
%% them, other iodata shapes, and terms that are not iodata.
terminfo() ->
    _ = prim_tty:setupterm_nif(),
    [{catch_error(fun() -> prim_tty:tigetnum_nif(C) end),
      catch_error(fun() -> prim_tty:tigetflag_nif(C) end)}
     || C <- ["co\0", "li\0", "it\0", "xn\0", "am\0", "bs\0", "km\0", "zz\0", "\0", "c\0",
              "cols\0", "co\0x", [<<"c">>, $o | <<0>>], <<"xn", 0>>, foo, 1, [256], [-1],
              [$c | x], make_ref(), 'co\0']].

%% The answers of tinfo_nif/0, tigetstr_nif/1, tputs_nif/2, and
%% wcwidth/1. The digest of each capability list keeps the output short, and
%% the order of each list stays in the digest.
extras() ->
    _ = prim_tty:setlocale(foo),
    _ = prim_tty:setupterm_nif(),
    #{bool := B, num := N, str := S} = prim_tty:tinfo_nif(),
    Caps = [B, N, S],
    [erlang:phash2(erlang:phash2([digest(C) || C <- Caps])),
     erlang:phash2([catch_error(fun() -> prim_tty:tigetstr_nif([C, 0]) end)
                    || C <- ["sgr0", "cup", "op", "zz", "not_a_capability"]]),
     erlang:phash2([catch_error(fun() -> prim_tty:tputs_nif([C, 0], A) end)
                    || {C, A} <- [{"cup", [1, 2]}, {"sgr0", []}, {"%", []}, {"%p1%d", [7]}]]),
     erlang:phash2([catch_error(fun() -> prim_tty:wcwidth(Char) end)
                    || Char <- [0, 16#61, 16#4e2d, 16#301, 16#1f600, -1, foo, 1 bsl 40]])
     | [length(C) || C <- Caps]].

digest(Caps) ->
    [{maps:get(name, C), maps:get(code, C), maps:get(full_name, C)} || C <- Caps].

init() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    {ok, Err} = prim_tty:tty_create(stderr),
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    [catch_error(fun() -> prim_tty:tty_init(T, O) end)
     || {T, O} <- [{Out, foo}, {foo, #{input => raw}}, {make_ref(), #{input => raw}},
                   {Re, #{input => raw}}, {Out, #{}},
                   {Out, #{input => raw}}, {Out, #{input => cooked}},
                   {Out, #{input => foo}}, {Err, #{input => raw}}]].

monitor() ->
    Pid = spawn(?MODULE, owner, [self()]),
    Ref = monitor(process, Pid),
    Result = receive {Pid, R} -> R end,
    Pid ! stop,
    receive {'DOWN', Ref, process, Pid, Reason} -> {Result, Reason} end.

%% A first holder process runs the first tty_init/2 on the first TTY, so
%% the owner has no monitor for it yet. The second and third TTY get the
%% raw mode of the first as their first mode. The owner then has three TTY
%% monitors: the second, the first, and the third. A second holder takes
%% over the third TTY. The holders do not exit.
owner(Parent) ->
    {ok, First} = prim_tty:tty_create(stderr),
    FirstHolder = spawn(?MODULE, holder, [self(), First, raw]),
    Raw = receive {FirstHolder, R1} -> R1 end,
    Size = prim_tty:tty_window_size(First),
    {ok, Second} = prim_tty:tty_create(stderr),
    SecondInit = prim_tty:tty_init(Second, #{input => cooked}),
    FirstInit = prim_tty:tty_init(First, #{input => raw}),
    {ok, Third} = prim_tty:tty_create(stderr),
    ThirdInit = prim_tty:tty_init(Third, #{input => cooked}),
    Holder = spawn(?MODULE, holder, [self(), Third, cooked]),
    HolderInit = receive {Holder, R2} -> R2 end,
    Parent ! {self(), [Raw, Size, SecondInit, FirstInit, ThirdInit, HolderInit]},
    receive stop -> ok end.

holder(Owner, Tty, Input) ->
    Owner ! {self(), prim_tty:tty_init(Tty, #{input => Input})},
    receive stop -> ok end.

select() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    [catch_error(fun() -> prim_tty:tty_select(T, R) end)
     || {T, R} <- [{Out, make_ref()}, {Out, foo}, {Out, undefined},
                   {foo, make_ref()}, {make_ref(), make_ref()}, {Re, make_ref()}]].

encoding() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    [prim_tty:tty_encoding(A) || A <- [Out, foo, 1, make_ref()]].

select_monitor() ->
    Pid = spawn(?MODULE, selector, [self()]),
    Ref = monitor(process, Pid),
    Result = receive {Pid, R} -> R end,
    Pid ! stop,
    receive {'DOWN', Ref, process, Pid, Reason} -> {Result, Reason} end.

%% The first TTY gets the raw mode from a holder. The selector sets the
%% first mode again, then calls tty_init/2 on a second TTY whose first mode
%% is raw, then tty_select/2 on the first TTY. Its monitors then set fd 0
%% to the first mode, raw, and the first mode again.
selector(Parent) ->
    {ok, First} = prim_tty:tty_create(stderr),
    Holder = spawn(?MODULE, holder, [self(), First, raw]),
    Raw = receive {Holder, R} -> R end,
    {ok, Second} = prim_tty:tty_create(stderr),
    FirstInit = prim_tty:tty_init(First, #{input => cooked}),
    SecondInit = prim_tty:tty_init(Second, #{input => cooked}),
    Select = prim_tty:tty_select(First, make_ref()),
    Parent ! {self(), [Raw, FirstInit, SecondInit, Select]},
    receive stop -> ok end.

%% The linker process spawns and links the owner when it gets go, so the
%% monitor is in place before the owner exits. The owner sets the raw mode
%% on a stderr TTY and exits with the reason boom. The linker does not trap
%% exits, so the exit signal ends it with the same reason.
link() ->
    Pid = spawn(?MODULE, linker, [self()]),
    Ref = monitor(process, Pid),
    Pid ! go,
    Result = receive {owner, R} -> R end,
    receive {'DOWN', Ref, process, Pid, Reason} -> {Result, Reason} end.

linker(Parent) ->
    receive go -> ok end,
    spawn_link(?MODULE, linked_owner, [Parent]),
    receive stop -> ok end.

linked_owner(Parent) ->
    {ok, Tty} = prim_tty:tty_create(stderr),
    Parent ! {owner, prim_tty:tty_init(Tty, #{input => raw})},
    exit(boom).

%% PBeam only, as the root process of a run: on ERTS, the -eval process
%% halts the node and runs no TTY monitor. The root process sets the raw
%% mode on a stderr TTY, switches out, and uses all but Left reductions of
%% its new budget before it returns. Its exit bumps 100 reductions
%% (erl_process.c:14229). With 100 or fewer reductions left, the exit
%% switches out before it leaves the process table, and with 150 or fewer,
%% before its links and monitors (erl_process.c:14315-14316, 14475,
%% 14541-14543). The TTY monitor runs in the next turn of the exit.
root_exit() -> root_owner(50).

root_free() -> root_owner(125).

root_owner(Left) ->
    {ok, Tty} = prim_tty:tty_create(stderr),
    ok = prim_tty:tty_init(Tty, #{input => raw}),
    erlang:yield(),
    erlang:bump_reductions(4000 - Left - 2),
    ok.

%% A group has 1024 elements (MAXIOV, prim_tty_nif.c:71-77).
write() ->
    {ok, Err} = prim_tty:tty_create(stderr),
    Group = copies(1023, <<"b">>),
    [catch_error(fun() -> prim_tty:write_nif(Err, I) end)
     || I <- [[], [<<"Hello, ">>, <<>>, <<"world!\n">>], foo, <<"x">>, [<<"x">>, foo],
              [<<"x">>, [<<"y">>]], Group ++ [foo], [<<"c">> | Group] ++ [foo],
              [<<"c">> | Group] ++ [<<"d">>], [<<"e">> | x], [<<"f">> | Group] ++ x,
              [<<"g">>, <<"g">> | Group] ++ x, [<<"h">> | <<"i">>], [<<>>]]].

%% Stderr is a pipe whose reader is closed.
write_error() ->
    {ok, Err} = prim_tty:tty_create(stderr),
    [catch_error(fun() -> prim_tty:write_nif(Err, I) end)
     || I <- [[], [<<"Hello">>], [<<"x">>, foo], [<<>>]]].

%% Stdin is a pipe that holds "hello\nworld\n" and has no writer. The
%% select of tty_select/2 sends one message. Each read_nif/3 that gives
%% {ok, _} selects fd 0 again, and the probe waits for the message of that
%% select before the next read. A read of 0 bytes and a read at the end of
%% file give {error, closed} and do not select fd 0.
read() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    Badargs = [catch_error(fun() -> prim_tty:read_nif(T, make_ref(), N) end)
               || {T, N} <- [{foo, 1}, {make_ref(), 1}, {Re, 1}, {Out, -1}, {Out, foo},
                             {Out, [1]}, {foo, -1}]],
    ok = prim_tty:tty_select(Out, first),
    Reads = [ready(Out, first) | [read(Out, N) || N <- [3, 0, 2000, 1024]]],
    [Badargs, Reads, receive M -> M after 100 -> none end].

read(Out, N) ->
    Ref = make_ref(),
    case prim_tty:read_nif(Out, Ref, N) of
        {ok, _} = Read -> {Read, ready(Out, Ref)};
        Read -> Read
    end.

ready(Out, Ref) ->
    receive {select, Out, Ref, ready_input} -> ready after 1000 -> none end.

%% Stdin is the write end of a pipe, so read fails with EBADF, and
%% read_nif/3 does not select fd 0.
read_error() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    Read = prim_tty:read_nif(Out, make_ref(), 10),
    [Read, receive M -> M after 100 -> none end].

%% Stdin is the non-blocking read end of an empty pipe, and stderr is its
%% write end, so write_nif/2 writes the input of the probe. A read of the
%% empty pipe gives {ok, <<>>} for EAGAIN and selects fd 0 again. A reader
%% process then calls tty_select/2, and the probe takes over the select with
%% read_nif/3, which does not change the reader. When the reader exits, its
%% monitor stops the select (prim_tty_nif.c:1242-1244), so the next input
%% sends no message.
loop() ->
    {ok, Out} = prim_tty:tty_create(stderr),
    Ref = make_ref(),
    Empty = prim_tty:read_nif(Out, Ref, 10),
    ok = prim_tty:write_nif(Out, [<<"x">>]),
    Ready = ready(Out, Ref),
    Read = prim_tty:read_nif(Out, Ref, 10),
    Reader = spawn(?MODULE, loop_reader, [self(), Out]),
    receive {Reader, selected} -> ok end,
    Monitor = monitor(process, Reader),
    Again = prim_tty:read_nif(Out, again, 10),
    Reader ! stop,
    receive {'DOWN', Monitor, process, Reader, normal} -> ok end,
    ok = prim_tty:write_nif(Out, [<<"y">>]),
    Stopped = receive {select, Out, again, ready_input} -> ready after 500 -> none end,
    Last = prim_tty:read_nif(Out, last, 10),
    [Empty, Ready, Read, Again, Stopped, Last].

loop_reader(Parent, Out) ->
    ok = prim_tty:tty_select(Out, reader),
    Parent ! {self(), selected},
    receive stop -> ok end.

copies(0, _) -> [];
copies(N, Binary) -> [Binary | copies(N - 1, Binary)].

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
