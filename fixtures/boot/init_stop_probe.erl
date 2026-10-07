-module(init_stop_probe).
-export([main/0]).

%% Monitor init and call init:stop/0, which sends {stop, stop} to init
%% (init.erl:536). Give 'stopped' when init exits in 10 ms, and 'running'
%% when it does not.
main() ->
    Ref = monitor(process, whereis(init)),
    ok = init:stop(),
    receive
        {'DOWN', Ref, process, _, _} -> stopped
    after 10 -> running
    end.
