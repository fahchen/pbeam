-module(bif_cause_probe).
-export([main/0, uncaught/0]).

%% Each case gives {Reason, MFA, ErrorInfo, Text}: the reason, the BIF of
%% the first frame, its error_info map, and the text of
%% erl_erts_errors:format_error/2.
uncaught() ->
    exit(main()).

main() ->
    Dead = spawn(fun() -> ok end),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, process, Dead, _} -> ok end,
    true = register(bif_cause_probe_name, self()),
    Other = spawn(fun() -> receive stop -> ok end end),
    Cases =
        [fun() -> monitor(process, self(), [bad]) end,
         fun() -> monitor(process, self(), bad) end,
         fun() -> monitor(bad, self()) end,
         fun() -> monitor(bad, self(), []) end,
         fun() -> monitor(1, self()) end,
         fun() -> monitor(process, 1) end,
         fun() -> monitor(time_offset, bad) end,
         fun() -> register(undefined, Other) end,
         fun() -> register("name", Other) end,
         fun() -> register(bif_cause_probe_dead, Dead) end,
         fun() -> register(bif_cause_probe_again, self()) end,
         fun() -> register(bif_cause_probe_name, Other) end,
         fun() -> register(bif_cause_probe_bad, bad) end,
         fun() -> erlang:send(self(), m, [bad]) end,
         fun() -> erlang:send(self(), m, [noconnect | bad]) end,
         fun() -> process_flag(bad, true) end,
         fun() -> process_flag({bad, 1}, true) end,
         fun() -> process_flag(save_calls, bad) end,
         fun() -> process_flag(save_calls, 10001) end,
         fun() -> process_flag(trap_exit, bad) end,
         fun() -> process_flag(priority, bad) end,
         fun() -> erlang:halt(0, [bad]) end,
         fun() -> erlang:halt(0, bad) end,
         fun() -> erlang:system_flag(bad, 1) end,
         fun() -> erlang:system_flag(1, 1) end,
         fun() -> erlang:system_flag(backtrace_depth, bad) end,
         fun() -> erlang:system_flag(time_offset, 1) end],
    Results = [run(Case) || Case <- Cases],
    Other ! stop,
    unregister(bif_cause_probe_name),
    Results.

run(Case) ->
    try Case() of
        Value -> {value, Value}
    catch
        error:Reason:Stack ->
            [{M, F, Args, Info} | _] = Stack,
            ErrorInfo = proplists:get_value(error_info, Info),
            Text = case ErrorInfo of
                       #{module := erl_erts_errors} ->
                           try erl_erts_errors:format_error(Reason, Stack)
                           catch _:_ -> format_crash
                           end;
                       _ -> none
                   end,
            {Reason, {M, F, length(Args)}, ErrorInfo, Text}
    end.
