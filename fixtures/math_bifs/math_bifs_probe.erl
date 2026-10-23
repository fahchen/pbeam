-module(math_bifs_probe).
-export([show/0, show_reductions/0]).

%% Return the term unchanged. The calls through id/1 keep the compiler from
%% folding the math calls at compile time.
id(Term) -> Term.

-define(REDS, element(2, process_info(self(), reductions))).

%% The arguments of each math BIF of one argument: zero of each sign,
%% small integers, bignums, normal floats, large floats, a subnormal float,
%% the smallest float, and a term that is not a number.
args_1() ->
    [0, 0.0, -0.0, 1, -1, 2, 1 bsl 80, -(1 bsl 80), 0.5, -0.5, 0.7, 0.9999999,
     1.5, 3.141592653589793, -2.5, 10.0, 20.0, 710.0, 1.0e22, 1.0e300,
     -1.0e300, 1.0e-310, 5.0e-324, a].

%% The argument pairs of each math BIF of two arguments, with zero divisors
%% and a term that is not a number in each place.
args_2() ->
    [{1, 1}, {1, -1}, {-1, 1}, {-1, -1}, {0, 0}, {0.0, -0.0}, {-0.0, -0.0},
     {-0.0, 1}, {5, 3}, {-5, 3}, {5.5, -2}, {1.0e300, 3.0}, {1 bsl 80, 7},
     {1, 0}, {1.0, 0.0}, {0, 1}, {1.0e-310, 1.0e300}, {5.0e-324, 2},
     {a, 1}, {1, a}].

unary() ->
    [{acos, fun math:acos/1}, {acosh, fun math:acosh/1},
     {asin, fun math:asin/1}, {asinh, fun math:asinh/1},
     {atan, fun math:atan/1}, {atanh, fun math:atanh/1},
     {ceil, fun math:ceil/1}, {cos, fun math:cos/1},
     {cosh, fun math:cosh/1}, {erf, fun math:erf/1},
     {erfc, fun math:erfc/1}, {floor, fun math:floor/1},
     {log2, fun math:log2/1}, {sin, fun math:sin/1},
     {sinh, fun math:sinh/1}, {tan, fun math:tan/1},
     {tanh, fun math:tanh/1}].

binary() ->
    [{atan2, fun math:atan2/2}, {fmod, fun math:fmod/2}].

%% Each result as the 64 bits of the float, so that the text compares the
%% results bit for bit, or as the class and the reason of the error.
result(Fun) ->
    try Fun() of
        Float -> <<Bits:64/signed>> = <<Float:64/float>>, Bits
    catch
        Class:Reason -> {Class, Reason}
    end.

results() ->
    [{Name, [result(fun() -> F(id(A)) end) || A <- args_1()]}
     || {Name, F} <- unary()] ++
    [{Name, [result(fun() -> F(id(A), id(B)) end) || {A, B} <- args_2()]}
     || {Name, F} <- binary()].

show() ->
    iolist_to_binary(text(results())).

%% The reductions of one call of each BIF through a fun and a loop, as the
%% mean of 1000 calls: one reduction each for the loop, the fun, id/1, and
%% the BIF. The division drops the reductions of process_info/2 and of the
%% list.
show_reductions() ->
    iolist_to_binary(text([{Name, reductions(fun(X) -> F(X) end, arg(Name))}
                           || {Name, F} <- unary()] ++
                          [{Name, reductions(fun(X) -> F(X, X) end, 0.5)}
                           || {Name, F} <- binary()])).

arg(acosh) -> 1.5;
arg(_) -> 0.5.

reductions(Fun, Arg) ->
    Before = ?REDS,
    loop(1000, Fun, Arg),
    (?REDS - Before) div 1000.

loop(0, _Fun, _Arg) -> ok;
loop(N, Fun, Arg) -> Fun(id(Arg)), loop(N - 1, Fun, Arg).

text(Term) when is_integer(Term) -> integer_to_binary(Term);
text(Term) when is_atom(Term) -> atom_to_binary(Term, utf8);
text(Term) when is_tuple(Term) -> [${, join(tuple_to_list(Term)), $}];
text(Term) when is_list(Term) -> [$[, join(Term), $]].

join([]) -> [];
join([Term]) -> [text(Term)];
join([Term | Terms]) -> [text(Term), $, | join(Terms)].
