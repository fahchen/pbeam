#!/usr/bin/env escript
%% Evaluate Erlang expressions as `erl -eval` does and print the value or the exception.
%% Usage: escript scripts/erts_eval.escript <code path or ""> <expressions>
%% Evaluate in a process that traps exits, as `erl -eval` does: init runs erl_eval:exprs/2 in the boot process (OTP-29.1.1 erts/preloaded/src/init.erl:1552-1566),
%% and that process sets trap_exit to true (init.erl:1195-1196). escript sets it to false before main/1 (lib/stdlib/src/escript.erl:419), so main/1 sets it back.
%% Exit signals then arrive as {'EXIT', From, Reason} messages. Only exit(self(), kill) stops the runtime, as with `erl -eval`.

main([CodePath, Expr]) ->
    process_flag(trap_exit, true),
    add_path(CodePath),
    Exprs = parse(Expr),
    try erl_eval:exprs(Exprs, erl_eval:new_bindings()) of
        {value, Value, _Bindings} -> io:format("~tp~n", [Value])
    catch
        Class:Reason -> io:format("exception ~p: ~tp~n", [Class, Reason])
    end;
main(_) ->
    fail("usage: erts_eval.escript <code path or \"\"> <expressions>~n", []).

add_path("") ->
    ok;
add_path(CodePath) ->
    case code:add_patha(CodePath) of
        true -> ok;
        {error, Reason} -> fail("code path ~ts: ~p~n", [CodePath, Reason])
    end.

%% Scan and parse as init does for `-eval`: add the final dot when it is missing (erts/preloaded/src/init.erl:1555-1560).
parse(Expr) ->
    case erl_scan:string(Expr) of
        {ok, Tokens, _End} ->
            Dotted =
                case lists:reverse(Tokens) of
                    [{dot, _} | _] -> Tokens;
                    Reversed -> lists:reverse([{dot, erl_anno:new(1)} | Reversed])
                end,
            case erl_parse:parse_exprs(Dotted) of
                {ok, Exprs} -> Exprs;
                {error, Error} -> fail_error(Error)
            end;
        {error, Error, _End} ->
            fail_error(Error)
    end.

fail_error({Location, Module, Descriptor}) ->
    fail("~p: ~ts~n", [Location, Module:format_error(Descriptor)]).

fail(Format, Args) ->
    io:format(standard_error, Format, Args),
    halt(1).
