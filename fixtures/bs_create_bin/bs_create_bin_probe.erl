-module(bs_create_bin_probe).
-export([prefix/1, around/1, guard/1, catch_bad/1, cost/1, utf16/1,
         binary_sized/2]).

%% "count: " <> X in Elixir: a string segment and a binary segment of
%% size all.
prefix(X) -> <<"count: ", X/binary>>.

%% "worker " <> X <> " crashed" in Elixir: string, binary, string.
around(X) -> <<"worker ", X/binary, " crashed">>.

%% bs_create_bin in a guard has a fail label.
guard(X) when <<"a", X/binary>> =:= <<"ab">> -> ab;
guard(_) -> other.

%% The exception of a binary segment with a term that is not a binary.
catch_bad(X) ->
    try prefix(X)
    catch Class:Reason:Stack -> {Class, Reason, Stack}
    end.

%% The reductions of one bs_create_bin with a binary segment of X.
cost(X) ->
    R0 = element(2, process_info(self(), reductions)),
    B = <<"x", X/binary>>,
    R1 = element(2, process_info(self(), reductions)),
    {R1 - R0, B}.

%% A utf16 segment after a string segment.
utf16(X) -> <<"b", X/utf16>>.

%% A binary segment of a size in a register, which PBeam does not run yet:
%% the loader keeps it as an unsupported segment (row "Binary construction").
binary_sized(X, Size) -> <<X:Size/binary>>.
