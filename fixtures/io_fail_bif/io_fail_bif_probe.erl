-module(io_fail_bif_probe).
-export([unicode_io/0, done/0, to_list/0, big/0, trap/0, id/1]).

%% net_kernel:dflag_unicode_io/1, as io:bc_req/3 (io.erl:1734-1735) runs it
%% for a pid of an io:format/3 call.
unicode_io() ->
    Dead = spawn(?MODULE, done, []),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, process, Dead, _} -> ok end,
    [try net_kernel:dflag_unicode_io(T) catch error:R -> {error, R} end
     || T <- [self(), Dead, foo, make_ref()]].

done() -> ok.

%% unicode:characters_to_list/2, as standard_error runs it for each
%% put_chars request (standard_error.erl:266-287): the binary path, the
%% list path with bytes of a sequence that continue in the next binary, and
%% the shape check of utf8_need (erl_unicode.c:342-505, 1122-1169).
to_list() ->
    %% 1 bsl 60 is a bignum, which PBeam does not load from the literal
    %% table, so the call makes the cases with it at run time.
    Bignum = id(1 bsl 60),
    [to_list(Chardata, Encoding) || {Chardata, Encoding} <-
        [{<<"Hello">>, unicode}, {<<97,255,98>>, unicode},
         {<<97,195>>, unicode}, {<<97,237,160>>, unicode},
         {<<97,224,128>>, unicode},
         {[<<97>>,[98,<<195>>]], unicode}, {[97,55296,98], unicode},
         {[97,-1], unicode}, {[97|foo], unicode}, {foo, unicode},
         {[a], unicode},
         {<<200>>, latin1}, {[256], latin1}, {[<<200>>,300], latin1},
         {<<195,169>>, latin1},
         {[<<195>>,<<169>>], unicode}, {[<<195>>,[<<169>>]], unicode},
         {[<<195>>,97], unicode}, {[<<97>>,<<195>>], unicode},
         {[<<97>>|<<195>>], unicode}, {[<<97,255>>,98], unicode},
         {[[97,[98,1114112,99]],100], unicode},
         {[Bignum], unicode}, {[Bignum|foo], unicode},
         {[<<240,159>>,<<152>>], unicode}, {[<<240,159>>|<<152>>], unicode},
         {[<<240,159>>,<<65>>], unicode}, {[<<120>>,<<224,160>>], unicode},
         {[<<224>>,<<97>>], unicode},
         {[], unicode}, {<<>>, latin1}, {[[],<<>>|<<"x">>], utf8},
         {[97,<<195,169>>], utf8},
         {[<<195>>,<<65,66>>], unicode}, {<<97,192>>, unicode},
         {[[97|<<255>>],1], unicode}, {[[-1]|<<"x">>], unicode},
         {<<240,159,152,128,97>>, unicode}, {<<128>>, unicode}]].

%% 100000 copies of $a in a binary. The compiler folds each binary of
%% the fixture to a literal, because PBeam does not run bs_create_bin.
-define(TENFOLD(B), <<B/binary, B/binary, B/binary, B/binary, B/binary,
                      B/binary, B/binary, B/binary, B/binary, B/binary>>).
-define(BIG_BINARY, ?TENFOLD(?TENFOLD(?TENFOLD(?TENFOLD(<<"aaaaaaaaaa">>))))).

%% The inputs of 100000 characters, which ERTS converts in more than one
%% call (allowed_iterations, erl_unicode.c:166-174). Each result is
%% summarized as {Tag, length(Converted), Rest}.
big() ->
    Big = tenfold(tenfold(tenfold(tenfold(tenfold("a"))))),
    %% The call keeps one literal of BigBinary in the fixture.
    BigBinary = id(?BIG_BINARY),
    [begin {Tag, Converted, Rest} = unicode:characters_to_list(Chardata, unicode),
           {Tag, length(Converted), Rest} end || Chardata <-
        [Big ++ [[-1,98]], [[Big,[<<"b">>,[<<"c",255>>,1]],2],3],
         <<?BIG_BINARY/binary,255,1>>, [BigBinary,<<195>>,7],
         [[[Big,-1,2],3],4], [BigBinary,<<240,159>>]]].

%% Ten copies of a list. A list cell of PBeam copies its tail, so the
%% input of 100000 characters is made with ++ in few steps.
tenfold(L) -> L ++ L ++ L ++ L ++ L ++ L ++ L ++ L ++ L ++ L.

%% Another encoding traps to unicode:characters_to_list_int/2
%% (erl_unicode.c:1135-1143).
trap() ->
    unicode:characters_to_list("abc", utf16).

id(Term) -> Term.

to_list(Chardata, Encoding) ->
    try unicode:characters_to_list(Chardata, Encoding)
    catch Class:Reason -> {Class, Reason}
    end.
