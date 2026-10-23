-module(chksum_probe).
-export([main/0, id/1]).

%% The `erlang` checksum BIFs `adler32/1,2`, `adler32_combine/3`,
%% `crc32/1,2`, and `crc32_combine/3` (erl_bif_chksum.c:53-451).
%%
%% Each caught error holds the class, the reason, and the BIF of the first
%% frame of the stack trace with its arity.
-define(CAUSE(Expr),
        try Expr catch Class:Reason:Stack -> {Class, Reason, frame(Stack)} end).

frame([{M, F, A, _} | _]) -> {M, F, length(A)}.

main() ->
    [sums(), combines(), errors(), reductions()].

%% Sums of iodata: nil, an empty binary, byte runs, deep lists, binary
%% tails, a sub-binary, and terms above the walk budget of 40000 bytes
%% (CHUNK_PER_SCHEDULE, erl_bif_chksum.c:286-287), which trap to the
%% two-argument BIF. An initial adler32 sum whose halves are not below
%% 65521 gives a value that depends on the length of the first part (zlib
%% adler32_z): parts of 0, 1, 3, and 6000 bytes.
sums() ->
    Big = binary:copy(<<"xyz">>, id(50000)),
    <<_:3/binary, Sub:10/binary, _/binary>> = id(list_to_binary(bytes(20))),
    [erlang:adler32(id([])), erlang:crc32(id([])),
     erlang:adler32(id(<<"abc">>)), erlang:crc32(id(<<"abc">>)),
     erlang:adler32(id([<<"ab">>, [$c | <<"d">>]])),
     erlang:crc32(id([[], [1 | <<2>>], [[[3]]], <<>>])),
     erlang:adler32(id(Sub)), erlang:crc32(id(Sub)),
     erlang:adler32(id(16#FFFFFFFF), id(<<>>)),
     erlang:adler32(id(16#FFFFFFFF), id([])),
     erlang:adler32(id(16#FFFFFFFF), id([1])),
     erlang:adler32(id(16#FFFFFFFF), id([1, 2, 3])),
     erlang:adler32(id(16#FFF0FFF8), id(binary:copy(<<255>>, 6000))),
     erlang:crc32(id(16#FFFFFFFF), id(bytes(300))),
     erlang:adler32(Big), erlang:crc32(Big),
     erlang:adler32([Big | bytes(256)]), erlang:crc32([Big | bytes(256)]),
     erlang:adler32(bytes(90000)), erlang:crc32([[bytes(45000)], Big])].

%% A length of 0 gives the first sum for adler32 (erl_bif_chksum.c:443-444)
%% and the zlib value for crc32.
combines() ->
    [erlang:adler32_combine(id(16#FFFFFFFF), id(16#FFFFFFFF), id(7)),
     erlang:adler32_combine(id(1), id(2), id(0)),
     erlang:adler32_combine(id(16#12345678), id(16#9abcdef0), id(16#FFFFFFFF)),
     erlang:crc32_combine(id(1), id(2), id(0)),
     erlang:crc32_combine(id(16#12345678), id(16#9abcdef0), id(100000)),
     erlang:crc32_combine(id(erlang:crc32(<<"ab">>)), id(erlang:crc32(<<"cd">>)), id(2))
         =:= erlang:crc32(<<"abcd">>)].

%% Each argument must be iodata or a 32-bit word (erl_bif_chksum.c:319,
%% 348-360). A bitstring that is not a whole number of bytes is `badarg`
%% (erl_bif_chksum.c:75-81), also after the trap of a long term.
errors() ->
    Big = binary:copy(<<"xyz">>, id(50000)),
    [?CAUSE(erlang:crc32(id(16#100000000), id([]))),
     ?CAUSE(erlang:adler32(id(-1), id([]))),
     ?CAUSE(erlang:adler32(id(1.0), id([]))),
     ?CAUSE(erlang:crc32(id(1 bsl 70), id([]))),
     ?CAUSE(erlang:crc32(id(<<(id(1)):7>>))),
     ?CAUSE(erlang:adler32(id([1 | 2]))),
     ?CAUSE(erlang:adler32(id([256]))),
     ?CAUSE(erlang:crc32(id(atom))),
     ?CAUSE(erlang:crc32([Big, <<(id(1)):7>>])),
     ?CAUSE(erlang:adler32_combine(id(-1), id(0), id(0))),
     ?CAUSE(erlang:adler32_combine(id(0), id(16#100000000), id(0))),
     ?CAUSE(erlang:crc32_combine(id(0), id(0), id(16#100000000)))].

%% One reduction for each 10 bytes read (erl_bif_chksum.c:300).
reductions() ->
    Binary = list_to_binary(bytes(1000)),
    [cost(fun() -> erlang:crc32(id(Binary)) end),
     cost(fun() -> erlang:adler32(id(bytes(1000))) end)].

cost(Fun) ->
    Before = element(2, process_info(self(), reductions)),
    Fun(),
    element(2, process_info(self(), reductions)) - Before.

%% A list of N bytes, so that a walk reads a byte run.
bytes(N) -> bytes(N, 0, []).

bytes(0, _Byte, Acc) -> Acc;
bytes(N, Byte, Acc) -> bytes(N - 1, (Byte + 1) rem 256, [Byte | Acc]).

%% Return the argument, so that the compiler does not fold a constant into
%% the BIF call.
id(Term) -> Term.
