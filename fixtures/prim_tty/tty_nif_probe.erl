-module(tty_nif_probe).
-export([isatty/0, create/0, window_size/0, setlocale/0, setupterm/0, init/0, monitor/0,
         owner/1, holder/3, select/0, encoding/0, select_monitor/0, selector/1, link/0,
         linker/1, linked_owner/1, write/0, write_error/0]).

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

copies(0, _) -> [];
copies(N, Binary) -> [Binary | copies(N - 1, Binary)].

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
