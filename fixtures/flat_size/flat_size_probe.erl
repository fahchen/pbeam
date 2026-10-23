-module(flat_size_probe).
-export([sizes/0, show/0, id/1]).

%% Return erts_debug:flat_size/1 of a table of terms of each kind.
sizes() ->
    X = id(7),
    Big = binary:copy(<<"a">>, id(100)),
    <<_:8, Rest/binary>> = Big,
    <<_:8, Small:10/binary, _/binary>> = Big,
    Hash = maps:from_list([{I, I} || I <- lists:seq(1, id(33))]),
    Hash2 = maps:from_list([{I, [I]} || I <- lists:seq(1, id(200))]),
    %% A and B have equal ERTS hashes, so the map has a collision node.
    A = [246, 157, 203, 77, 19, 254, 19, 93],
    B = [161, 92, 65, 50, 231, 24, 153, 137],
    Collision = Hash#{A => a, B => b},
    Ref = make_ref(),
    Alias = monitor(process, self(), [{alias, demonitor}]),
    Tab = ets:new(t, []),
    Tuple = {a, X},
    Terms = [1, 1 bsl 59 - 1, 1 bsl 59, -(1 bsl 59), -(1 bsl 59) - 1,
             1 bsl 63, 1 bsl 64, -(1 bsl 64), 1 bsl 200, 1.5, a, self(),
             Ref, Alias, Tab, {}, Tuple, {Tuple, Tuple}, [a], [a | b],
             [1, 2, 3], [Tuple, Tuple], #{}, #{a => 1}, #{a => Tuple, b => [X]},
             Hash, Hash2, Collision, <<>>, <<1, 2, 3>>, <<1:3>>, <<1, 2, 3, 4:5>>,
             binary:copy(<<"a">>, id(64)), binary:copy(<<"a">>, id(65)),
             Big, Rest, Small, element(2, split_binary(Big, 99)),
             fun() -> ok end, fun() -> X end, fun() -> {X, Tuple} end,
             fun lists:map/2, [fun() -> X end | Hash]],
    [erts_debug:flat_size(Term) || Term <- id(Terms)].

%% Exit with the result of sizes/0, so the host reads it.
show() ->
    exit(sizes()).

id(Term) ->
    Term.
