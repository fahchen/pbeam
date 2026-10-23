-module(net_probe).
-export([main/0, text/0, debug_on/0]).

%% The cases of `nif_info/0`, `nif_command/1`, and `nif_gethostname/0` of
%% the `net` NIF library (`prim_net_nif.c:848-1021`). `main/0` gives one
%% pair for each case: the result of the call, or the class and the reason
%% of its exception.
main() ->
    [{info, call(fun() -> lists:sort(maps:to_list(prim_net:info())) end)},
     {debug_off, call(fun() -> prim_net:debug(false) end)},
     {info_debug, call(fun() -> maps:get(debug, prim_net:info()) end)},
     {debug_bad, call(fun() -> prim_net:debug(1) end)},
     {hostname, call(fun() -> element(1, net:gethostname()) end)}].

%% The external term format of `main/0` and of `net:gethostname/0`.
text() ->
    term_to_binary([main(), net:gethostname()]).

%% Turn on the debug trace of the library.
debug_on() ->
    prim_net:debug(true).

call(Fun) ->
    try Fun() of
        Value -> {value, Value}
    catch
        Class:Reason -> {Class, Reason}
    end.
