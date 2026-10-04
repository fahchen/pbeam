-module(user_drv_probe).
-export([main/0, iovec/0, start_driver/3, state/3, writer/2]).

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
