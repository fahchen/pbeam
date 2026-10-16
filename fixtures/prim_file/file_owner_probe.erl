-module(file_owner_probe).
-export([main/1]).

%% Open the file Path with prim_file in two processes that exit without a
%% close, and return their exit reasons. The first exits at once, before it
%% handles the monitor signal of its file resource. The second handles the
%% signal in its receive first. The file resource monitors its owner, and
%% the exit of the owner closes the file (prim_file_nif.c:377-405).
main(Path) ->
    {First, FirstRef} = spawn_monitor(fun() -> {ok, _} = prim_file:open(Path, [write]) end),
    {Second, SecondRef} =
        spawn_monitor(fun() ->
                              {ok, _} = prim_file:open(Path, [write]),
                              receive after 1 -> ok end
                      end),
    receive
        {'DOWN', FirstRef, process, First, FirstReason} ->
            receive
                {'DOWN', SecondRef, process, Second, SecondReason} ->
                    {FirstReason, SecondReason}
            end
    end.
