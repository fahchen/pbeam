-module(init_stop_probe).
-export([main/0]).

%% Monitor init and call init:stop/0, which sends {stop, stop} to init
%% (init.erl:536). init halts with status 0 (init.erl:924-945) before the
%% probe gives 'stopped' (init exits in 5 s) or 'running'. The 5 s limit
%% lets a slow host, such as CI with coverage, complete the stop of the
%% kernel processes.
main() ->
    Ref = monitor(process, whereis(init)),
    ok = init:stop(),
    receive
        {'DOWN', Ref, process, _, _} -> stopped
    after 5000 -> running
    end.
