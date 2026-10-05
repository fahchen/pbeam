-module(user_drv_probe).
-export([main/0, iovec/0, iolists/7, ne/0, ne/3, start_driver/3, state/3, writer/2]).

main() ->
    WriterRef = make_ref(),
    Writer = spawn(?MODULE, writer, [self(), WriterRef]),
    Driver = spawn(?MODULE, start_driver, [self(), Writer, WriterRef]),
    From = {self(), make_ref()},
    Driver ! {self(), {put_chars_sync, unicode, <<"Hello, world!\n">>, From}},
    Written = receive {written, Chars} -> Chars end,
    Reply = receive {reply, From, R} -> R end,
    {Written, Reply}.

start_driver(Group, Writer, WriterRef) ->
    put('$ancestors', [self(), Group]),
    put(current_group, Group),
    process_flag(trap_exit, true),
    gen_statem:enter_loop(user_drv, [], server, state(Group, Writer, WriterRef)).

state(Group, Writer, WriterRef) ->
    Tty = {state, make_ref(), undefined, {Writer, WriterRef}, #{input => disabled, output => raw, ofd => stdout},
           true, true, [], [], [], [], undefined, 1, 0, <<>>, 80, 24, false, undefined},
    {state, Tty, WriterRef, undefined, disabled, false, undefined, Group, Group,
     {gr, 1, 0, Group, [{group, 0, Group, {}}]}, {false, queue:new()}}.

writer(Probe, WriterRef) ->
    receive
        {write, From, Chars} ->
            Probe ! {written, Chars},
            From ! {WriterRef, ok},
            ?MODULE:writer(Probe, WriterRef)
    end.

iovec() ->
    [try erlang:iolist_to_iovec(X) catch error:R -> R end || X <- [<<>>, [], foo]].

%% ERTS copies bytes and each binary of at most 512 bytes into one merged
%% binary, and keeps each larger binary as its own element (erl_iolist.c).
%% The caller gives the binaries and the deep list, as PBeam has no
%% bs_create_bin and no gc_bif2. The caller also gives 256 and -1.
iolists(B64, B512, B513, Huge, Deep, Above, Below) ->
    Inputs = [[[]], [<<>>], [1, 2], [<<"a">>, [<<"b">>, 3]], [[[[1]]]], [[], <<>>, [[]]],
              [[[1, [2, [3, <<4>>]], 5]], 6], [B64], [1, B64], [B512], [1, B512], [B512, B512],
              [B513], [1, B513], [1, B513, 2], [B513, B513], [B513, 1, 2, B513, <<>>],
              [<<>>, B513], [[B513]], [Huge], [1, Huge, <<"ab">>], Deep,
              [Above], [Below], [a], [1, a], [1, [2, Above]], [B513, foo], B64, B513],
    [try erlang:iolist_to_iovec(X) catch error:R -> R end || X <- Inputs].

ne() ->
    [?MODULE:ne(A, B, C) || {A, B, C} <- [{a, a, b}, {a, b, a}, {a, b, c}]].

%% The guard compiles to is_ne_exact on A and B, as user_drv:server/3 does.
ne(A, B, C) when A =:= B orelse A =:= C -> match;
ne(_, _, _) -> no_match.
