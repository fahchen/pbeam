-module(bs_create_bin_writable_probe).
-export([bytes/1, chunks/1, floats/1, sizes/1, shared/1, float_bad/1, float_size/2, append_float/3, bits/1, cause/2, ints/1, reds/1]).

%% A loop that appends one byte: bs_init_writable and an integer segment
%% after a private_append segment.
bytes(N) -> bytes(N, <<>>).

bytes(0, Acc) -> Acc;
bytes(N, Acc) -> bytes(N - 1, <<Acc/binary, N:8>>).

%% A loop that appends a binary after a private_append segment.
chunks(List) -> chunks(List, <<>>).

chunks([], Acc) -> Acc;
chunks([B | T], Acc) -> chunks(T, <<Acc/binary, B/binary>>).

%% A loop that appends a 64-bit float after a private_append segment.
floats(List) -> floats(List, <<>>).

floats([], Acc) -> Acc;
floats([F | T], Acc) -> floats(T, <<Acc/binary, F/float>>).

%% The size of each step of a byte loop, which reads the binary.
sizes(N) -> sizes(N, <<>>, []).

sizes(0, Acc, Sizes) -> {Acc, Sizes};
sizes(N, Acc, Sizes) ->
    Next = <<Acc/binary, N:8>>,
    sizes(N - 1, Next, [byte_size(Next) | Sizes]).

%% A loop whose accumulator is also kept in a list.
shared(N) -> shared(N, <<>>, []).

shared(0, Acc, Seen) -> {Acc, Seen};
shared(N, Acc, Seen) -> shared(N - 1, <<Acc/binary, N:8>>, [Acc | Seen]).

%% A float segment with a term that is not a number.
float_bad(F) -> <<F/float>>.

%% A float segment with a size in a register.
float_size(F, S) -> <<F:S/float>>.

%% An append segment and a float segment with a size in a register.
append_float(A, F, S) -> <<A/binary, F:S/float>>.

%% A loop that appends 4 bits, which gives a bitstring.
bits(N) -> bits(N, <<>>).

bits(0, Acc) -> Acc;
bits(N, Acc) -> bits(N - 1, <<Acc/bitstring, N:4>>).

%% The reason and the error_info cause of a call that fails.
cause(F, Args) ->
    try apply(?MODULE, F, Args)
    catch error:Reason:Stack ->
        [{_, _, _, Info} | _] = Stack,
        {Reason, error_info(Info)}
    end.

error_info([{error_info, #{cause := Cause}} | _]) -> Cause;
error_info([_ | T]) -> error_info(T).

%% A loop that appends each integer of a list as one byte.
ints(List) -> ints(List, <<>>).

ints([], Acc) -> Acc;
ints([X | T], Acc) -> ints(T, <<Acc/binary, X:8>>).

%% The reductions of a loop that appends 1024 bytes N times, between two
%% process_info/2 calls.
reds(N) ->
    {reductions, R0} = process_info(self(), reductions),
    B = kilobytes(N, <<>>),
    {reductions, R1} = process_info(self(), reductions),
    {byte_size(B), R1 - R0}.

kilobytes(0, Acc) -> Acc;
kilobytes(N, Acc) -> kilobytes(N - 1, <<Acc/binary, 0:8192>>).
