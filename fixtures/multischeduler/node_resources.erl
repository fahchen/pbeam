-module(node_resources).
-export([zlib_give/0, zlib_use/0, reference/0]).

%% Two roots on two schedulers share one zlib stream. The root `zlib_give`
%% opens the stream, gives the control to the root `zlib_use`, and checks
%% that it cannot use the stream after that. The root `zlib_use` deflates
%% with the stream and returns the data that `zlib:uncompress/1` gives back.

-define(DATA, <<"node resources node resources node resources">>).

reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {give, zlib_give()} end),
    _ = spawn(fun() -> Self ! {use, zlib_use()} end),
    Give = receive {give, A} -> A after 60000 -> timeout end,
    Use = receive {use, B} -> B after 60000 -> timeout end,
    {Give, Use}.

zlib_give() ->
    Z = zlib:open(),
    ok = zlib:deflateInit(Z),
    Pid = wait_for(zlib_user, 100000),
    ok = zlib:set_controlling_process(Z, Pid),
    Rejected =
        try zlib:deflate(Z, ?DATA) of
            _ -> used
        catch
            error:not_on_controlling_process -> not_on_controlling_process;
            Class:Reason -> {Class, Reason}
        end,
    Pid ! {zlib, self(), Z},
    receive done -> Rejected after 60000 -> timeout end.

zlib_use() ->
    true = register(zlib_user, self()),
    receive
        {zlib, From, Z} ->
            Deflated = iolist_to_binary(zlib:deflate(Z, ?DATA, finish)),
            ok = zlib:deflateEnd(Z),
            ok = zlib:close(Z),
            From ! done,
            zlib:uncompress(Deflated) =:= ?DATA
    after 60000 -> timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid -> Pid
    end.
