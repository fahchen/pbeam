-module(io_client_probe).
-export([main/0, list/0, server/2, gone/0, group_leader_badarg/0]).
main() -> run(<<"Hello, world!\n">>).
list() -> run([<<"Hello, world!">>, 10]).
run(Chars) ->
    Old = group_leader(),
    Server = spawn(?MODULE, server, [self(), Old]),
    group_leader(Server, self()),
    self() ! noise,
    Result = io:put_chars(standard_io, Chars),
    group_leader(Old, self()),
    Request = receive {request, R, Inherited} -> {R, Inherited} end,
    Kept = receive noise -> noise_ok end,
    Leak = receive {'DOWN', _, _, _, _} -> leaked after 0 -> none end,
    {Result, Request, Kept, Leak, flush()}.
server(Caller, Old) ->
    receive
        {io_request, From, ReplyAs, Request} when is_reference(ReplyAs) ->
            Caller ! {request, Request, same(group_leader(), Old)},
            From ! {io_reply, ReplyAs, ok}
    end.
gone() -> ok.
flush() ->
    {Pid, Ref} = spawn_monitor(?MODULE, gone, []),
    receive {'DOWN', Ref, process, Pid, normal} -> ok end,
    Ref2 = monitor(process, Pid),
    receive {'DOWN', Ref2, process, Pid, noproc} -> ok end,
    Ref3 = monitor(process, Pid),
    Demon = demonitor(Ref3, [flush]),
    {Demon, receive {'DOWN', Ref3, _, _, _} -> down after 0 -> no_down end}.
group_leader_badarg() ->
    {Pid, Ref} = spawn_monitor(?MODULE, gone, []),
    receive {'DOWN', Ref, process, Pid, normal} -> ok end,
    {erts_internal:group_leader(self(), Pid), erts_internal:group_leader(not_a_pid, self())}.
same(X, X) -> same;
same(_, _) -> different.
