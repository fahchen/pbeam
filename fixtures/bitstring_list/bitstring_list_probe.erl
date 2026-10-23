-module(bitstring_list_probe).
-export([small/0, long/0, show/1, id/1]).

%% Run erlang:bitstring_to_list/1 and erlang:list_to_bitstring/1 on
%% binaries, bitstrings, iolists with bitstring elements and tails, and bad
%% arguments. A raise gives the first frame of its stack trace, so the
%% result shows the error_info of the BIF.
small() ->
    B2L = [<<>>, <<1, 2, 3>>, <<1:3>>, <<1, 2, 3:3>>, <<255, 7:7>>,
           element(2, split_binary(<<1, 2, 3:5>>, 1)), [], a, [<<1>>], 1],
    L2B = [[], [1, 2], [<<1:3>>], [<<1:3>>, <<2:5>>, 1 | <<3:2>>],
           [1 | <<1:1>>], [[<<1:1>>], [], [2, <<7:3>>]], [<<1, 2>>],
           [<<>>, <<1:7>>, <<>>], <<1>>, <<1:1>>, a, [256], [-1], [a],
           [1 | a], [<<1:1>> | 2], [[1 | <<1:4>>] | <<3:4>>]],
    {[run(fun erlang:bitstring_to_list/1, Term) || Term <- id(B2L)],
     [run(fun erlang:list_to_bitstring/1, Term) || Term <- id(L2B)]}.

%% Run the BIFs on terms that use more than one turn of reductions, so that
%% the walks trap. Each result is compared with the result of a short walk.
long() ->
    Bits = [<<N:(N rem 8 + 1)>> || N <- lists:seq(1, 20000)],
    Deep = [[<<1:1>>, <<0:9000>>, 5 | <<2:3>>] || _ <- lists:seq(1, 300)],
    Big = <<0:(300000 * 8), 5:3>>,
    L2B = erlang:list_to_bitstring(Bits),
    D2B = erlang:list_to_bitstring(Deep),
    B2L = erlang:bitstring_to_list(Big),
    {bit_size(L2B), L2B =:= << <<B/bitstring>> || B <- Bits >>,
     bit_size(D2B), D2B =:= << <<(erlang:list_to_bitstring(D))/bitstring>> || D <- Deep >>,
     length(B2L), lists:last(B2L), lists:sum(lists:droplast(B2L)),
     try erlang:list_to_bitstring(Bits ++ [a]) catch error:Reason -> Reason end,
     %% The copy of the tail takes 62500 steps, more than one turn, so it
     %% stops in the tail at a bit offset that is not a whole byte of the
     %% result.
     erlang:list_to_bitstring(id([<<1:1>> | <<0:(8 * 2000000)>>])) =:= <<1:1, 0:(8 * 2000000)>>}.

run(Fun, Term) ->
    try Fun(Term)
    catch error:Reason:Stack -> {error, Reason, hd(Stack)}
    end.

%% Exit with the result of a probe function, so that the exit reason
%% prints it.
show(Name) -> exit(id(?MODULE:Name())).

id(Term) -> Term.
