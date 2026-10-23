-module(timer_cause_probe).
-export([main/0, uncaught/0]).

%% Each case gives {Reason, MFA, ErrorInfo, Text} for a timer BIF of
%% erl_hl_timer.c: the reason, the BIF of the first frame, its error_info
%% map, and the text of erl_erts_errors:format_error/2.
uncaught() ->
    exit(main()).

main() ->
    Cases =
        [fun() -> erlang:send_after(bad, self(), m) end,
         fun() -> erlang:send_after(-1, self(), m) end,
         fun() -> erlang:send_after(10, self(), m, [bad]) end,
         fun() -> erlang:send_after(10, self(), m, bad) end,
         fun() -> erlang:send_after(bad, self(), m, []) end,
         fun() -> erlang:send_after(-1, self(), m, [{abs, true}]) end,
         fun() -> erlang:start_timer(bad, self(), m) end,
         fun() -> erlang:start_timer(-1, self(), m) end,
         fun() -> erlang:start_timer(10, self(), m, [{abs, bad}]) end,
         fun() -> erlang:start_timer(bad, self(), m, [{abs, false}]) end,
         fun() -> erlang:cancel_timer(bad) end,
         fun() -> erlang:cancel_timer(make_ref(), [bad]) end,
         fun() -> erlang:read_timer(bad) end],
    [run(Case) || Case <- Cases].

run(Case) ->
    try Case() of
        Value when is_reference(Value) -> reference;
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
