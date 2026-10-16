-module(file_owner_probe).
-export([main/3]).

%% Open the FIFOs First and Second for write with prim_file in two processes
%% that exit without a close, one after the other, and return their exit
%% reasons. The first exits at once, before it handles the monitor signal of
%% its file resource. The second handles the signal in its receive first.
%% The file resource monitors its owner, and the exit of the owner closes
%% the file (prim_file_nif.c:377-405), so the reader of each FIFO gets the
%% end of file. An open of a FIFO for write waits for its reader, so the
%% open of Second and of Barrier waits until the reader saw the end of
%% First and of Second.
main(First, Second, Barrier) ->
    {FirstPid, FirstRef} = spawn_monitor(fun() -> {ok, _} = prim_file:open(First, [write]) end),
    FirstReason = receive {'DOWN', FirstRef, process, FirstPid, Reason1} -> Reason1 end,
    {SecondPid, SecondRef} =
        spawn_monitor(fun() ->
                              {ok, _} = prim_file:open(Second, [write]),
                              receive after 1 -> ok end
                      end),
    SecondReason = receive {'DOWN', SecondRef, process, SecondPid, Reason2} -> Reason2 end,
    {ok, Fd} = prim_file:open(Barrier, [write]),
    ok = prim_file:close(Fd),
    {FirstReason, SecondReason}.
