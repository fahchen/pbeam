-module(at_exit_probe).
-export([main/0]).

%% Start the elixir and logger applications, as elixir -e does, and register two
%% at_exit callbacks. The second callback runs first, because
%% System.at_exit/1 puts each callback at the head of the list
%% (system.ex:474-477). Each callback gets the status 0. Then log a
%% warning just before the return: the default handler is async, so the
%% event is in its queue at the return, and only the flush of the logger
%% writes it.
main() ->
    {ok, _} = application:ensure_all_started([elixir, logger]),
    ok = 'Elixir.System':at_exit(fun(Status) -> io:format(standard_error, "first ~p~n", [Status]) end),
    ok = 'Elixir.System':at_exit(fun(Status) -> io:format(standard_error, "second ~p~n", [Status]) end),
    logger:warning("last event").
