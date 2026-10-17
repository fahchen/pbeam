-module(zlib_probe).
-export([exact/0, filtered_large/0, roundtrips/0, flushes/0, safe_chunks/0,
         eos/0, errors/0, controller/0, controller_child/2, uncompress_hex/1,
         unsupported_set_controller/0, unsupported_deflate_dictionary/0,
         unsupported_deflate_params/0, unsupported_inflate_dictionary/0,
         unsupported_inflate_get_dictionary/0]).

%% The data of the chunking cases. It is larger than the 8 KiB input chunk
%% and the 16 KiB output chunk of the pinned `zlib.erl` (`zlib.erl:141-144`).
data() ->
    binary:copy(<<"The quick brown fox jumps over the lazy dog. ">>, 1000).

%% The data of the exact byte cases.
small() ->
    <<"hello hello hello world">>.

hex(Bin) ->
    hex(Bin, <<>>).

hex(<<>>, Acc) ->
    Acc;
hex(<<Byte, Rest/binary>>, Acc) ->
    hex(Rest, <<Acc/binary, (hex_digit(Byte bsr 4)), (hex_digit(Byte band 15))>>).

hex_digit(N) when N < 10 -> $0 + N;
hex_digit(N) -> $a + N - 10.

unhex(<<>>) ->
    <<>>;
unhex(<<Hi, Lo, Rest/binary>>) ->
    <<((digit(Hi) bsl 4) bor digit(Lo)), (unhex(Rest))/binary>>.

digit(N) when N >= $0, N =< $9 -> N - $0;
digit(N) when N >= $a, N =< $f -> N - $a + 10.

%% One deflate stream of `Data` with the arguments of `deflateInit/6`.
stream(Level, WindowBits, Strategy, Data) ->
    Z = zlib:open(),
    try
        ok = zlib:deflateInit(Z, Level, deflated, WindowBits, 8, Strategy),
        B = iolist_to_binary(zlib:deflate(Z, Data, finish)),
        ok = zlib:deflateEnd(Z),
        B
    after
        zlib:close(Z)
    end.

%% The compressed bytes of each level and strategy of `small/0`.
exact() ->
    D = small(),
    [{list_to_binary("level " ++ integer_to_list(L)), hex(stream(L, 15, default, D))}
     || L <- lists:seq(0, 9)] ++
    [{list_to_binary("strategy " ++ atom_to_list(S)), hex(stream(6, 15, S, D))}
     || S <- [default, filtered, huffman_only, rle]] ++
    [{list_to_binary("window " ++ integer_to_list(W)), hex(stream(6, W, default, D))}
     || W <- [15, -15, 31]].

%% The compressed bytes of level 6 and the filtered strategy for `data/0`.
%% This is the one level and strategy pair where the bytes of the pinned
%% zlib 1.3.2 and `zlib-rs` differ.
filtered_large() ->
    hex(stream(6, 15, filtered, data())).

%% The public one-shot functions.
roundtrips() ->
    D = data(),
    [D =:= zlib:uncompress(zlib:compress(D)),
     D =:= zlib:gunzip(zlib:gzip(D)),
     D =:= zlib:unzip(zlib:zip(D))].

%% The bytes of a deflate stream with each flush mode, and of a stream
%% after `deflateReset/1`.
flushes() ->
    D = small(),
    Z = zlib:open(),
    ok = zlib:deflateInit(Z, default),
    P1 = iolist_to_binary(zlib:deflate(Z, D, none)),
    P2 = iolist_to_binary(zlib:deflate(Z, <<>>, sync)),
    P3 = iolist_to_binary(zlib:deflate(Z, D, full)),
    P4 = iolist_to_binary(zlib:deflate(Z, <<>>, finish)),
    ok = zlib:deflateReset(Z),
    P5 = iolist_to_binary(zlib:deflate(Z, D, finish)),
    ok = zlib:deflateEnd(Z),
    zlib:close(Z),
    [hex(iolist_to_binary([P1, P2, P3, P4])), hex(P5)].

%% The results and the output size of each `safeInflate/2` call of a
%% compressed `data/0`.
safe_chunks() ->
    C = zlib:compress(data()),
    Z = zlib:open(),
    ok = zlib:inflateInit(Z),
    Out = safe_loop(Z, C, []),
    ok = zlib:inflateEnd(Z),
    zlib:close(Z),
    Out.

safe_loop(Z, Input, Acc) ->
    case zlib:safeInflate(Z, Input) of
        {continue, Out} ->
            safe_loop(Z, [], [{<<"continue">>, iolist_size(Out)} | Acc]);
        {finished, Out} ->
            lists:reverse([{<<"finished">>, iolist_size(Out)} | Acc])
    end.

%% Two concatenated zlib streams with each end-of-stream behavior.
eos() ->
    A = zlib:compress(<<"first">>),
    B = zlib:compress(<<"second">>),
    Data = <<A/binary, B/binary>>,
    {hex(eos_run(15, reset, Data)),
     hex(eos_run(15, cut, Data)),
     eos_error(15, error, Data)}.

eos_run(Bits, Behavior, Data) ->
    Z = zlib:open(),
    ok = zlib:inflateInit(Z, Bits, Behavior),
    Out = iolist_to_binary(zlib:inflate(Z, Data)),
    ok = zlib:inflateEnd(Z),
    zlib:close(Z),
    Out.

eos_error(Bits, Behavior, Data) ->
    Z = zlib:open(),
    ok = zlib:inflateInit(Z, Bits, Behavior),
    R = catch_error(fun() -> zlib:inflate(Z, Data) end),
    zlib:close(Z),
    atom_to_binary(R, utf8).

%% The errors of the argument checks and of the stream states.
errors() ->
    Z = zlib:open(),
    R1 = catch_error(fun() -> zlib:deflate(Z, <<>>) end),
    R2 = catch_error(fun() -> zlib:deflateInit(Z, 10) end),
    R3 = catch_error(fun() -> zlib:deflateInit(Z, default, deflated, 7, 8, default) end),
    R4 = catch_error(fun() -> zlib:deflateInit(Z, default, deflated, 15, 0, default) end),
    R5 = catch_error(fun() -> zlib:deflateInit(Z, default, raw, 15, 8, default) end),
    R6 = catch_error(fun() -> zlib:deflateInit(Z, default, deflated, 15, 8, bogus) end),
    R7 = catch_error(fun() -> zlib:deflateInit(Z, default, deflated, 17, 8, default) end),
    ok = zlib:deflateInit(Z, default),
    R8 = catch_error(fun() -> zlib:deflateInit(Z, default) end),
    _ = zlib:deflate(Z, <<"x">>, none),
    R9 = catch_error(fun() -> zlib:deflateEnd(Z) end),
    ok = zlib:close(Z),
    R10 = catch_error(fun() -> zlib:close(Z) end),
    I = zlib:open(),
    R11 = catch_error(fun() -> zlib:inflate(I, <<>>) end),
    ok = zlib:inflateInit(I, 15),
    R12 = catch_error(fun() -> zlib:inflateInit(I, 15) end),
    R13 = catch_error(fun() -> zlib:inflateEnd(I) end),
    ok = zlib:close(I),
    [atom_to_binary(R, utf8)
     || R <- [R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12, R13]].

%% The error of a call from a process that does not control the stream.
controller() ->
    Z = zlib:open(),
    Parent = self(),
    spawn(?MODULE, controller_child, [Parent, Z]),
    receive
        {remote, R} -> R
    end.

controller_child(Parent, Z) ->
    Parent ! {remote, atom_to_binary(catch_error(fun() -> zlib:close(Z) end), utf8)}.

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.

%% Uncompress the bytes of a hex string.
uncompress_hex(Hex) ->
    zlib:uncompress(unhex(Hex)).

%% The calls of the library functions that PBeam does not bind.
unsupported_set_controller() ->
    Z = zlib:open(),
    zlib:set_controlling_process(Z, self()).

unsupported_deflate_dictionary() ->
    Z = zlib:open(),
    ok = zlib:deflateInit(Z, default),
    zlib:deflateSetDictionary(Z, <<>>).

unsupported_deflate_params() ->
    Z = zlib:open(),
    ok = zlib:deflateInit(Z, default),
    zlib:deflateParams(Z, 1, default).

unsupported_inflate_dictionary() ->
    Z = zlib:open(),
    ok = zlib:inflateInit(Z),
    zlib:inflateSetDictionary(Z, <<>>).

unsupported_inflate_get_dictionary() ->
    Z = zlib:open(),
    ok = zlib:inflateInit(Z),
    zlib:inflateGetDictionary(Z).
