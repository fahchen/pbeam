-module(bif_trap_stack_probe).
-export([uncaught/0]).

%% Each case calls a BIF on a long list that ends in a bad element or tail,
%% so the BIF traps and then raises. The result gives the first frame, with
%% each list argument as {list, Length, Tail}, and the next frame. The
%% `lists` BIFs keep the rest of the list, whose length depends on the
%% reductions of the time slice, so their frame gives {list, rest, Tail}
%% for a list that is shorter than the input.
uncaught() ->
    L = lists:seq(1, 200000),
    Erlang = id(erlang),
    exit([case_of(fun() -> lists:member(x, L ++ a) end),
          case_of(fun() -> lists:reverse(L ++ a, []) end),
          case_of(fun() -> Erlang:length(L ++ a) end),
          case_of(fun() -> list_to_binary(L ++ [a]) end),
          case_of(fun() -> iolist_size(L ++ [a]) end),
          case_of(fun() -> erlang:iolist_to_iovec(L ++ [a]) end),
          case_of(fun() -> L -- (L ++ a) end),
          case_of(fun() -> (L ++ a) -- [1] end),
          case_of(fun() -> tail_apply(Erlang, L ++ [a]) end)]).

tail_apply(Module, List) ->
    Module:iolist_size(List).

case_of(Fun) ->
    try Fun() of
        Value -> {value, Value}
    catch
        Class:Reason:Stack ->
            [{M, F, A, Location}, Next | _] = Stack,
            {Class, Reason, {M, F, [rest(M, summary(X)) || X <- A], Location}, Next}
    end.

rest(lists, {list, N, T}) when N > 0, N < 200000 -> {list, rest, T};
rest(_, Summary) -> Summary.

summary(X) when is_list(X) -> summary(X, 0);
summary(X) -> X.

summary([_ | T], N) -> summary(T, N + 1);
summary(T, N) -> {list, N, T}.

id(X) -> X.
