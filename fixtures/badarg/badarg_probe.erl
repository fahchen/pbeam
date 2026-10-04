-module(badarg_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, id/1]).

main() ->
    {reason(catch ?MODULE:id(a) ++ [1]),
     reason(catch lists:keyfind(a, ?MODULE:id(0), [])),
     reason(catch lists:keyfind(a, 1, ?MODULE:id(a))),
     reason(catch erlang:function_exported(?MODULE:id(1), self, 0)),
     reason(catch erlang:function_exported(erlang, self, ?MODULE:id(x))),
     reason(catch apply_module()),
     reason(catch apply_name()),
     try lists:keyfind(a, ?MODULE:id(x), []) catch error:R -> {error, R} end}.

id(X) -> X.

apply_module() -> M = ?MODULE:id(1), M:f().
apply_name() -> F = ?MODULE:id(1), {erlang:F()}.

reason({'EXIT', {Reason, Stack}}) when is_list(Stack) -> Reason.
