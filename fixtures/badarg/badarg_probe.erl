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
     reason(catch element(?MODULE:id(0), {a})),
     reason(catch element(1, ?MODULE:id([a]))),
     try lists:keyfind(a, ?MODULE:id(x), []) catch error:R1 -> {error, R1} end,
     try element(?MODULE:id(2), {a}) catch error:R2 -> {error, R2} end}.

id(X) -> X.

apply_module() -> M = ?MODULE:id(1), M:f().
apply_name() -> F = ?MODULE:id(1), {erlang:F()}.

reason({'EXIT', {Reason, Stack}}) when is_list(Stack) -> Reason.
