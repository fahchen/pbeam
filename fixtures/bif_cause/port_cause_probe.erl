-module(port_cause_probe).
-export([main/0, uncaught/0]).

%% Each case gives {Reason, MFA, ErrorInfo, Text} for a BIF of
%% erl_bif_re.c, erl_bif_os.c, erl_bif_port.c, or external.c: the reason,
%% the BIF of the first frame, its error_info map, and the text of the
%% format_error/2 function of the error_info module.
uncaught() ->
    exit(main()).

main() ->
    Cases =
        [fun() -> re:compile("a", [bad]) end,
         fun() -> re:run("a", "a", [bad]) end,
         fun() -> re:run("a", "a", [export]) end,
         fun() -> re:run(1, "a", []) end,
         fun() -> os:set_signal(sighup, bad) end,
         fun() -> os:set_signal(sigint, ignore) end,
         fun() -> erlang:decode_packet(bad, <<>>, []) end,
         fun() -> binary_to_term(term_to_binary(a), [bad]) end,
         fun() -> binary_to_term(term_to_binary(a), bad) end,
         fun() -> binary_to_term(<<1>>, []) end],
    [run(Case) || Case <- Cases].

run(Case) ->
    try Case() of
        Value -> {value, Value}
    catch
        error:Reason:Stack ->
            [{M, F, Args, Info} | _] = Stack,
            ErrorInfo = proplists:get_value(error_info, Info),
            Text = case ErrorInfo of
                       #{module := Module} ->
                           try Module:format_error(Reason, Stack)
                           catch _:_ -> format_crash
                           end;
                       _ -> none
                   end,
            {Reason, {M, F, length(Args)}, ErrorInfo, Text}
    end.
