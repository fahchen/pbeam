-module(zstd_probe).
-export([main/0, owner_child/3, unsupported_dictionary/0,
         unsupported_get_parameter/0, unsupported_default_strategy/0]).

%% Write each case as its label and the hex text of the external term
%% format of its result. The feature test compares the output with the
%% output of the pinned ERTS.
main() ->
    [io:format("~s ~s~n", [Label, binary:encode_hex(term_to_binary(attempt(Fun)))])
     || {Label, Fun} <- cases()],
    ok.

attempt(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

small() ->
    <<"hello hello hello world">>.

%% 45000 bytes that compress well.
text() ->
    binary:copy(<<"The quick brown fox jumps over the lazy dog. ">>, 1000).

%% 140000 bytes that do not compress. They are larger than the output
%% buffers of the NIFs, `ZSTD_CStreamOutSize()` and `ZSTD_DStreamOutSize()`.
noise() ->
    noise(140000, 1, []).

noise(0, _, Acc) ->
    list_to_binary(Acc);
noise(N, X, Acc) ->
    Next = (X * 1664525 + 1013904223) band 16#FFFFFFFF,
    noise(N - 1, Next, [(Next bsr 24) | Acc]).

flat(IoData) ->
    iolist_to_binary(IoData).

frame() ->
    flat(zstd:compress(text())).

with_compress(Fun) ->
    {ok, Ctx} = zstd:context(compress),
    Fun(Ctx).

with_decompress(Fun) ->
    {ok, Ctx} = zstd:context(decompress),
    Fun(Ctx).

cases() ->
    compress_cases() ++ parameter_cases() ++ context_cases() ++ decompress_cases().

compress_cases() ->
    [{<<"compress small">>, fun() -> zstd:compress(small()) end},
     {<<"compress empty">>, fun() -> zstd:compress(<<>>) end},
     {<<"compress iolist">>, fun() -> zstd:compress(["hel", <<"lo">>, $!]) end},
     {<<"compress text">>, fun() -> zstd:compress(text()) end},
     {<<"compress noise">>, fun() -> zstd:compress(noise()) end},
     {<<"levels">>,
      fun() ->
              [flat(zstd:compress(text(), #{compressionLevel => L}))
               || L <- [-5, 0, 1, 3, 9, 19, 22, 100]]
      end},
     {<<"strategies">>,
      fun() ->
              [flat(zstd:compress(text(), #{strategy => S}))
               || S <- [default, fast, dfast, greedy, lazy, lazy2, btlazy2, btopt,
                        btultra, btultra2]]
      end},
     {<<"flags">>,
      fun() ->
              [flat(zstd:compress(small(), #{K => V}))
               || K <- [checksumFlag, contentSizeFlag, dictIDFlag, enableLongDistanceMatching],
                  V <- [true, false]]
      end},
     {<<"integers">>,
      fun() ->
              [flat(zstd:compress(text(), #{K => V}))
               || {K, V} <- [{windowLog, 10}, {hashLog, 6}, {chainLog, 6}, {searchLog, 2},
                             {minMatch, 4}, {targetLength, 16}, {targetCBlockSize, 1340},
                             {ldmHashLog, 10}, {ldmMinMatch, 16}, {ldmBucketSizeLog, 2},
                             {ldmHashRateLog, 4}]]
      end},
     {<<"long distance">>,
      fun() ->
              zstd:compress(noise(), #{enableLongDistanceMatching => true, ldmHashLog => 12,
                                       compressionLevel => 5})
      end},
     {<<"bandit context">>,
      fun() ->
              {ok, Ctx} = zstd:context(compress, #{}),
              A = zstd:compress(<<"Hello world from PBeam!">>, Ctx),
              B = zstd:compress(small(), Ctx),
              {A, B, zstd:close(Ctx)}
      end},
     {<<"stream flush finish">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        R1 = zstd:stream(Ctx, <<"hello ">>),
                        R2 = zstd:flush(Ctx),
                        R3 = zstd:stream(Ctx, ["wor", <<"ld">>]),
                        R4 = zstd:finish(Ctx, <<"!">>),
                        R5 = zstd:finish(Ctx, []),
                        {R1, R2, R3, R4, R5}
                end)
      end},
     {<<"stream noise">>,
      fun() ->
              with_compress(fun(Ctx) -> {zstd:stream(Ctx, noise()), zstd:finish(Ctx, <<>>)} end)
      end},
     {<<"flush noise">>,
      fun() ->
              with_compress(fun(Ctx) -> {zstd:stream(Ctx, noise()), zstd:flush(Ctx)} end)
      end},
     {<<"finish noise">>, fun() -> with_compress(fun(Ctx) -> zstd:finish(Ctx, noise()) end) end},
     {<<"reset">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        _ = zstd:stream(Ctx, <<"abc">>),
                        {zstd:reset(Ctx), zstd:finish(Ctx, <<"def">>)}
                end)
      end},
     {<<"stream badarg reset">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        _ = zstd:stream(Ctx, <<"abc">>),
                        R = attempt(fun() -> zstd:stream(Ctx, foo) end),
                        {R, zstd:finish(Ctx, <<"def">>)}
                end)
      end},
     {<<"stream atom">>, fun() -> with_compress(fun(Ctx) -> zstd:stream(Ctx, foo) end) end},
     {<<"stream bad iolist">>, fun() -> with_compress(fun(Ctx) -> zstd:stream(Ctx, [foo]) end) end}].

parameter_cases() ->
    [{<<"bad key">>, fun() -> zstd:context(compress, #{bogus => 1}) end},
     {<<"key not atom">>,
      fun() -> with_compress(fun(Ctx) -> zstd:set_parameter(Ctx, "windowLog", 10) end) end},
     {<<"bad level">>, fun() -> zstd:context(compress, #{compressionLevel => foo}) end},
     {<<"big level">>, fun() -> zstd:context(compress, #{compressionLevel => 1 bsl 40}) end},
     {<<"bad strategy">>, fun() -> zstd:context(compress, #{strategy => 1}) end},
     {<<"bad flag">>, fun() -> zstd:context(compress, #{checksumFlag => 1}) end},
     {<<"bad integer">>, fun() -> zstd:context(compress, #{windowLog => 1.0}) end},
     {<<"window out of bound">>, fun() -> zstd:context(compress, #{windowLog => 5}) end},
     {<<"window negative">>, fun() -> zstd:context(compress, #{windowLog => -1}) end},
     {<<"stage wrong">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        _ = zstd:stream(Ctx, <<"a">>),
                        zstd:set_parameter(Ctx, windowLog, 12)
                end)
      end},
     {<<"level mid stream">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        R1 = zstd:stream(Ctx, text()),
                        R2 = zstd:set_parameter(Ctx, compressionLevel, 19),
                        R3 = zstd:set_parameter(Ctx, strategy, default),
                        {R1, R2, R3, zstd:finish(Ctx, text())}
                end)
      end},
     {<<"pledged">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        {zstd:set_parameter(Ctx, pledgedSrcSize, 3), zstd:finish(Ctx, <<"abc">>)}
                end)
      end},
     {<<"pledged wrong">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        ok = zstd:set_parameter(Ctx, pledgedSrcSize, 5),
                        {continue, _} = zstd:stream(Ctx, <<"abc">>),
                        R = attempt(fun() -> zstd:finish(Ctx, <<>>) end),
                        {R, zstd:finish(Ctx, <<"x">>)}
                end)
      end},
     {<<"pledged max">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        {zstd:set_parameter(Ctx, pledgedSrcSize, (1 bsl 64) - 1),
                         zstd:finish(Ctx, small())}
                end)
      end},
     {<<"pledged big">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        R1 = zstd:set_parameter(Ctx, pledgedSrcSize, 1 bsl 62),
                        R2 = zstd:stream(Ctx, small()),
                        {R1, R2, attempt(fun() -> zstd:finish(Ctx, <<>>) end)}
                end)
      end},
     {<<"pledged over">>,
      fun() -> with_compress(fun(Ctx) -> zstd:set_parameter(Ctx, pledgedSrcSize, 1 bsl 64) end) end},
     {<<"pledged negative">>,
      fun() -> with_compress(fun(Ctx) -> zstd:set_parameter(Ctx, pledgedSrcSize, -1) end) end},
     {<<"pledged atom">>,
      fun() -> with_compress(fun(Ctx) -> zstd:set_parameter(Ctx, pledgedSrcSize, foo) end) end},
     {<<"pledged mid stream">>,
      fun() ->
              with_compress(
                fun(Ctx) ->
                        _ = zstd:stream(Ctx, <<"a">>),
                        zstd:set_parameter(Ctx, pledgedSrcSize, 1)
                end)
      end},
     {<<"window log max">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        [attempt(fun() -> zstd:set_parameter(Ctx, K, V) end)
                         || {K, V} <- [{windowLogMax, 20}, {windowLogMax, foo}, {bogus, 1},
                                       {bogus, foo}, {"windowLogMax", 1}, {windowLogMax, 5},
                                       {windowLogMax, 1 bsl 40}]]
                end)
      end},
     {<<"window too large">>,
      fun() ->
              Frame = flat(zstd:compress(noise(), #{windowLog => 20})),
              zstd:decompress(Frame, #{windowLogMax => 10})
      end},
     {<<"invalid context">>, fun() -> zstd:set_parameter(foo, windowLog, 10) end}].

context_cases() ->
    [{<<"closed">>,
      fun() ->
              {ok, C} = zstd:context(compress),
              {ok, D} = zstd:context(decompress),
              [zstd:close(C), zstd:close(D)
               | [attempt(F)
                  || F <- [fun() -> zstd:close(C) end, fun() -> zstd:close(D) end,
                           fun() -> zstd:reset(C) end, fun() -> zstd:reset(D) end,
                           fun() -> zstd:stream(C, <<"a">>) end,
                           fun() -> zstd:stream(D, <<"a">>) end,
                           fun() -> zstd:set_parameter(C, windowLog, 10) end,
                           fun() -> zstd:set_parameter(D, windowLogMax, 10) end,
                           fun() -> zstd:set_parameter(C, pledgedSrcSize, 1) end]]]
      end},
     {<<"other references">>,
      fun() ->
              {ok, {compress, C}} = zstd:context(compress),
              {ok, {decompress, D}} = zstd:context(decompress),
              [attempt(F)
               || F <- [fun() -> zstd:stream({compress, foo}, <<"a">>) end,
                        fun() -> zstd:close({decompress, foo}) end,
                        fun() -> zstd:stream({compress, make_ref()}, <<"a">>) end,
                        fun() -> zstd:close({decompress, make_ref()}) end,
                        fun() -> zstd:stream({decompress, C}, <<"a">>) end,
                        fun() -> zstd:stream({compress, D}, <<"a">>) end,
                        fun() -> zstd:reset({decompress, C}) end,
                        fun() -> zstd:close({compress, D}) end,
                        fun() -> zstd:set_parameter({decompress, C}, windowLogMax, 10) end,
                        fun() -> zstd:set_parameter({compress, D}, windowLog, 10) end,
                        fun() -> zstd:set_parameter({compress, D}, pledgedSrcSize, 1) end]]
      end},
     {<<"owner">>,
      fun() ->
              {ok, C} = zstd:context(compress),
              {ok, D} = zstd:context(decompress),
              spawn(?MODULE, owner_child, [self(), C, D]),
              receive {owner, Results} -> Results end
      end}].

owner_child(Parent, C, D) ->
    Results = [attempt(F)
               || F <- [fun() -> zstd:stream(C, <<"a">>) end,
                        fun() -> zstd:stream(D, <<"a">>) end,
                        fun() -> zstd:set_parameter(C, bogus, 1) end,
                        fun() -> zstd:set_parameter(D, windowLogMax, foo) end,
                        fun() -> zstd:set_parameter(C, pledgedSrcSize, foo) end,
                        fun() -> zstd:reset(C) end, fun() -> zstd:reset(D) end,
                        fun() -> zstd:close(C) end, fun() -> zstd:close(D) end]],
    Parent ! {owner, Results}.

decompress_cases() ->
    [{<<"decompress small">>, fun() -> zstd:decompress(zstd:compress(small())) end},
     {<<"decompress text">>, fun() -> zstd:decompress(frame()) end},
     {<<"decompress noise">>, fun() -> zstd:decompress(zstd:compress(noise())) end},
     {<<"decompress unknown size">>,
      fun() ->
              {done, Frame} = with_compress(fun(Ctx) -> zstd:finish(Ctx, ["a", noise()]) end),
              zstd:decompress(Frame)
      end},
     {<<"decompress empty frame">>, fun() -> zstd:decompress(zstd:compress(<<>>)) end},
     {<<"decompress frame list">>, fun() -> zstd:decompress([frame(), frame()]) end},
     {<<"decompress frames">>,
      fun() -> zstd:decompress(<<(frame())/binary, (flat(zstd:compress(small())))/binary>>) end},
     {<<"decompress garbage">>, fun() -> zstd:decompress(<<"garbage">>) end},
     {<<"decompress truncated">>,
      fun() -> F = frame(), zstd:decompress(binary:part(F, 0, byte_size(F) - 3)) end},
     {<<"decompress checksum">>,
      fun() ->
              F = flat(zstd:compress(small(), #{checksumFlag => true})),
              Size = byte_size(F) - 1,
              <<Head:Size/binary, Last>> = F,
              zstd:decompress(<<Head/binary, (Last bxor 1)>>)
      end},
     {<<"decompress empty input">>, fun() -> zstd:decompress(<<>>) end},
     {<<"decompress stream">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        F = frame(),
                        <<A:10/binary, B/binary>> = F,
                        R1 = zstd:stream(Ctx, A),
                        R2 = zstd:finish(Ctx, B),
                        R3 = zstd:finish(Ctx, F),
                        R4 = zstd:finish(Ctx, <<>>),
                        {R1, R2, R3, R4}
                end)
      end},
     {<<"decompress reuse">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        F = flat(zstd:compress(small())),
                        {zstd:decompress(F, Ctx), zstd:decompress(F, Ctx)}
                end)
      end},
     {<<"decompress badarg reset">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        <<A:10/binary, B/binary>> = frame(),
                        R1 = zstd:stream(Ctx, A),
                        R2 = attempt(fun() -> zstd:stream(Ctx, foo) end),
                        R3 = attempt(fun() -> zstd:stream(Ctx, B) end),
                        {R1, R2, R3}
                end)
      end},
     {<<"decompress first badarg">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        R = attempt(fun() -> zstd:stream(Ctx, foo) end),
                        {R, zstd:decompress(frame(), Ctx)}
                end)
      end},
     {<<"decompress atom">>, fun() -> with_decompress(fun(Ctx) -> zstd:stream(Ctx, foo) end) end},
     {<<"decompress reset">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        <<A:10/binary, B/binary>> = frame(),
                        R1 = zstd:stream(Ctx, A),
                        R2 = zstd:reset(Ctx),
                        {R1, R2, attempt(fun() -> zstd:finish(Ctx, B) end)}
                end)
      end},
     {<<"decompress flush">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        try zstd:flush(Ctx)
                        catch error:{badarg, {invalid_context, Ctx}} -> invalid_context
                        end
                end)
      end},
     {<<"decompress set mid stream">>,
      fun() ->
              with_decompress(
                fun(Ctx) ->
                        <<A:10/binary, _/binary>> = frame(),
                        _ = zstd:stream(Ctx, A),
                        zstd:set_parameter(Ctx, windowLogMax, 20)
                end)
      end}].

%% The calls that PBeam does not support.
unsupported_dictionary() ->
    zstd:context(compress, #{dictionary => <<>>}).

unsupported_get_parameter() ->
    with_compress(fun(Ctx) -> zstd:get_parameter(Ctx, compressionLevel) end).

unsupported_default_strategy() ->
    with_compress(
      fun(Ctx) ->
              ok = zstd:set_parameter(Ctx, strategy, fast),
              zstd:set_parameter(Ctx, strategy, default)
      end).
