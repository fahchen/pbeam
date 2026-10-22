-module(line_table_probe).
-export([uncaught/0, stack/1, current/0, maybe_raise/1]).
-compile(nowarn_deprecated_catch).

%% The fixture keeps its line table. keep/1 drops the frames of the erl
%% -eval caller, so the frames of ERTS and PBeam can be compared.
uncaught() ->
    exit(main()).

main() ->
    [stack(error), stack(throw), stack(exit), stack(error3),
     stack(badmatch), stack(case_clause), stack(if_clause),
     stack(function_clause), stack(bif), stack(next_line),
     stack(element), stack(fun_tail), stack(apply_tail),
     catch_stack(error), catch_stack(throw), catch_stack(badmatch),
     line_table_bare:call(fun ?MODULE:stack/1, error),
     current(), line_table_bare:call(fun ?MODULE:current/0)].

stack(Kind) ->
    try raise(Kind) of
        Value -> {value, Value}
    catch
        Class:Reason:Stack -> {Class, Reason, keep(Stack)}
    end.

catch_stack(Kind) ->
    case catch raise(Kind) of
        {'EXIT', {Reason, Stack}} -> {'EXIT', Reason, keep(Stack)};
        Value -> {value, Value}
    end.

raise(error) -> erlang:error(e);
raise(throw) -> throw(t);
raise(exit) -> exit(x);
raise(error3) -> erlang:error(e, [a], [{error_info, #{module => m}}]);
raise(badmatch) -> {ok, _} = id(error), done;
raise(case_clause) -> case id(c) of a -> done end;
raise(if_clause) -> X = id(i), if X =:= a -> done end;
raise(function_clause) -> clause(id(b));
raise(bif) -> [atom_to_list(id(1))];
raise(element) -> [element(id(5), {a})];
raise(fun_tail) -> F = id(fun erlang:error/1), F(f);
raise(apply_tail) -> apply(id(erlang), error, [a]);
raise(next_line) ->
    X = [?MODULE:maybe_raise(id(yes))],
    [id(X)].

maybe_raise(yes) -> erlang:error(n);
maybe_raise(no) -> ok.

clause(a) ->
    done.

current() ->
    {current_stacktrace, Stack} = process_info(self(), current_stacktrace),
    [keep(Stack)].

id(X) -> X.

keep(Stack) ->
    [Frame || {Module, _, _, _} = Frame <- Stack,
              Module =/= erl_eval, Module =/= shell, Module =/= init].
