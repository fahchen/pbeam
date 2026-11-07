-module(dist_fun_creator_probe).
-export([run/0]).

run() ->
    Fun = fun(X) -> X + 1 end,
    Encoded = term_to_binary(Fun),
    Creator = <<88, 119, 13, "nonode@nohost", 0:96>>,
    [Head, Tail] = binary:split(Encoded, Creator),
    Remote = <<88, 119, 13, "remote@nohost", 1:32, 0:32, 1:32>>,
    Cases = [Creator, Remote, <<97, 1>>],
    Results = [try binary_to_term(<<Head/binary, C/binary, Tail/binary>>) of
                   Decoded -> {Decoded =:= Fun, term_to_binary(Decoded) =:= Encoded, Decoded(41)}
               catch error:Reason -> {error, Reason}
               end || C <- Cases],
    io:format("~w~n", [Results]).
