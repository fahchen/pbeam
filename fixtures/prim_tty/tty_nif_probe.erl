-module(tty_nif_probe).
-export([isatty/0, create/0, window_size/0, setlocale/0, setupterm/0, init/0, monitor/0,
         owner/1, holder/2, write/0]).

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
     || {T, O} <- [{Out, foo}, {foo, #{input => raw}}, {Re, #{input => raw}}, {Out, #{}},
                   {Out, #{input => raw}}, {Out, #{input => cooked}},
                   {Out, #{input => foo}}, {Err, #{input => raw}}]].

monitor() ->
    Pid = spawn(?MODULE, owner, [self()]),
    Ref = monitor(process, Pid),
    Result = receive {Pid, R} -> R end,
    Pid ! stop,
    receive {'DOWN', Ref, process, Pid, Reason} -> {Result, Reason} end.

%% The second and third TTY get the raw mode of the first as their first
%% mode. The owner then has four TTY monitors: the first, the second, the
%% first again, and the third. A holder process takes over the third TTY and
%% does not exit.
owner(Parent) ->
    {ok, First} = prim_tty:tty_create(stderr),
    Raw = prim_tty:tty_init(First, #{input => raw}),
    Size = prim_tty:tty_window_size(First),
    {ok, Second} = prim_tty:tty_create(stderr),
    SecondInit = prim_tty:tty_init(Second, #{input => cooked}),
    FirstInit = prim_tty:tty_init(First, #{input => raw}),
    {ok, Third} = prim_tty:tty_create(stderr),
    ThirdInit = prim_tty:tty_init(Third, #{input => cooked}),
    Holder = spawn(?MODULE, holder, [self(), Third]),
    HolderInit = receive {Holder, R} -> R end,
    Parent ! {self(), [Raw, Size, SecondInit, FirstInit, ThirdInit, HolderInit]},
    receive stop -> ok end.

holder(Owner, Tty) ->
    Owner ! {self(), prim_tty:tty_init(Tty, #{input => cooked})},
    receive stop -> ok end.

write() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    prim_tty:write_nif(Out, []).

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
