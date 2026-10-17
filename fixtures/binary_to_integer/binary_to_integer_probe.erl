-module(binary_to_integer_probe).
-export([main/0, bits/0, id/1]).

%% `erts_internal:binary_to_integer/2` (`big.c:4162-4256`): the base must
%% be a small integer from 2 to 36, and the binary must hold a whole number
%% of bytes. The text reads one optional `-` or `+`, then the digits of the
%% base. An empty binary, a sign without a digit, a digit outside the base,
%% a term that is not a binary, and a base outside 2 to 36 raise `badarg`.
%% Leading zeros do not count toward the digit limit, and a text above
%% `digits_per_small_lookup` digits gives the atom `big` before ERTS reads
%% a digit. `id/1` hides the arguments from the compiler.
main() ->
    [result(<<"3FF">>, 16),
     result(<<"10">>, 2),
     result(<<"-12">>, 10),
     result(<<"+33">>, 10),
     result(<<"-0">>, 10),
     result(<<"Z">>, 36),
     result(<<"z">>, 36),
     result(<<"0000000000000000000000000000000001">>, 10),
     result(<<"0000000000000000000000000000000000">>, 10),
     result(<<"12345678901234567">>, 10),
     result(<<"100000000000000000">>, 10),
     result(<<"123456789012345678">>, 10),
     result(<<"1g">>, 10),
     result(<<"1g">>, 16),
     result(<<"">>, 10),
     result(<<"-">>, 10),
     result(<<"+">>, 10),
     result(<<"2">>, 2),
     result(<<"1a">>, 10),
     result(<<255>>, 10),
     result(<<$1, 0>>, 10),
     result(<<"1">>, 1),
     result(<<"1">>, 37),
     result(<<"1">>, -1),
     result(<<"1">>, 10.0),
     result(<<"1">>, 123456789012345678),
     result(<<"1">>, 1234567890123456789),
     result(<<"1">>, <<1, 0>>),
     result(1, 10),
     result([], 10)].

%% The BIF for a bit string that is not a whole number of bytes gives the
%% atom `badarg`. The argument of the call is such a bit string, which PBeam
%% cannot build yet (row "Bitstring terms" of `docs/erts-differences.md`), so
%% this probe gives the value alone.
bits() ->
    [erts_internal:binary_to_integer(?MODULE:id(<<1:1>>), 10)].

id(X) -> X.

%% Give the value of the BIF, or the reason and the stack trace of the
%% exception.
result(Binary, Base) ->
    try erts_internal:binary_to_integer(?MODULE:id(Binary), ?MODULE:id(Base)) of
        Value -> {ok, Value}
    catch
        error:Reason:Stacktrace -> {error, Reason, frames(Stacktrace)}
    end.

%% The frame of the BIF and the frames of this module below it: the
%% `erl -eval` caller adds its own frames after them, and a PBeam run has
%% none.
frames([Bif | Rest]) -> [Bif | frames_of_module(Rest)];
frames(_) -> [].

frames_of_module([Frame = {?MODULE, _, _, _} | Rest]) -> [Frame | frames_of_module(Rest)];
frames_of_module(_) -> [].
