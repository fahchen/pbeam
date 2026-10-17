-module(hash_probe).
-export([main/0, id/1]).

%% `erlang:phash/2` is deprecated in favour of `phash2/2`, but the release
%% imports it, so the probe calls it on purpose.
-compile(nowarn_deprecated_function).

%% The `erlang` md5 and phash BIFs that the `plug_hello` release imports:
%% `md5/1`, `md5_init/0`, `md5_update/2`, `md5_final/1` (erl_bif_chksum.c:455-610,
%% erl_md5.c:76-144), and `phash/2`, `phash2/1,2` (bif.c:5319-5402,
%% erl_term_hashing.c:165-427, 858-1596).
%%
%% Each caught error holds the class, the reason, and the first frame of the
%% stack trace: the frame of the BIF with its arity, which ERTS
%% `save_stacktrace` puts first (beam_common.c:880-1055).
-define(CAUSE(Expr),
        try Expr catch Class:Reason:Stack -> {Class, Reason, first_frame(Stack)} end).

first_frame([{Module, Function, Arguments, _Location} | _Rest]) ->
    [{Module, Function, frame_arity(Arguments)}];
first_frame(_Frames) ->
    [].

frame_arity(Arguments) when is_list(Arguments) -> length(Arguments);
frame_arity(_Arguments) -> 0.

main() ->
    [md5_iodata(), md5_context(), phash_values(), reductions()].

%% The md5 of an empty term, a string, an improper list with a binary tail,
%% a byte run above one block, and a binary above the byte budget of one
%% reduction run. The budget of `erlang:md5/1` is four bytes for each
%% reduction of the caller (erl_bif_chksum.c:465-467), so the binary of
%% 40000 bytes traps at least twice, and the walk continues in the next
%% turn. A term that is not iodata is `badarg`. A bitstring that is not a
%% whole number of bytes is `badarg` too, which a unit test of the BIF
%% checks: the loader of PBeam has no such literal.
md5_iodata() ->
    [binary_to_list(erlang:md5([])),
     binary_to_list(erlang:md5(<<>>)),
     binary_to_list(erlang:md5("abc")),
     binary_to_list(erlang:md5([1, 2 | <<3>>])),
     binary_to_list(erlang:md5([[1], 2, [<<3>>]])),
     binary_to_list(erlang:md5(bytes(300))),
     binary_to_list(erlang:md5(list_to_binary(bytes(40000)))),
     binary_to_list(erlang:md5([[bytes(40000)]])),
     binary_to_list(erlang:md5([list_to_binary(bytes(40000))])),
     binary_to_list(erlang:md5(list_to_binary(bytes(56)))),
     binary_to_list(erlang:md5(list_to_binary(bytes(60)))),
     binary_to_list(erlang:md5(list_to_binary(bytes(63)))),
     ?CAUSE(erlang:md5(id(1))),
     ?CAUSE(erlang:md5([1 | 2])),
     ?CAUSE(erlang:md5([257]))].

%% The state of `md5_init/0` is a binary of 96 bytes (erts_md5_state,
%% erl_md5.h:31-36): four 32-bit words, the length of the whole input, the
%% length of the buffered part, and a buffer of 64 bytes. `md5_update/2`
%% returns a new state and keeps the argument, and `md5_final/1` returns the
%% digest and keeps the argument, so a state gives the same digest again.
%% The final state of a fabricated binary of 96 bytes gives the digest of
%% that state. A state of another size and a term that is not a binary are
%% `badarg`.
md5_context() ->
    C0 = erlang:md5_init(),
    C1 = erlang:md5_update(C0, "abc"),
    Final = erlang:md5_final(C1),
    Fabricated = <<0:768>>,
    [byte_size(C0), binary_to_list(C0), byte_size(C1), C1 =:= C0,
     binary_to_list(Final),
     binary_to_list(erlang:md5_final(C1)),
     binary_to_list(erlang:md5_final(C0)),
     binary_to_list(erlang:md5_final(erlang:md5_update(C1, <<"def">>))),
     byte_size(erlang:md5_update(C0, list_to_binary(bytes(40000)))),
     binary_to_list(erlang:md5_update(C0, list_to_binary(bytes(56)))),
     binary_to_list(erlang:md5_update(C0, list_to_binary(bytes(64)))),
     binary_to_list(erlang:md5_update(C0, bytes(56) ++ [[]] ++ bytes(20))),
     binary_to_list(erlang:md5_update(erlang:md5_update(C0, list_to_binary(bytes(56))),
                                       list_to_binary(bytes(20)))),
     binary_to_list(erlang:md5_update(erlang:md5_update(C0, list_to_binary(bytes(56))),
                                       list_to_binary(bytes(4)))),
     binary_to_list(erlang:md5_final(Fabricated)),
     ?CAUSE(erlang:md5_final(Final)),
     ?CAUSE(erlang:md5_update(Final, <<>>)),
     ?CAUSE(erlang:md5_update(<<0:800>>, <<>>)),
     ?CAUSE(erlang:md5_final(<<0:512>>)),
     ?CAUSE(erlang:md5_final(id(a))),
     ?CAUSE(erlang:md5_update(C0, id(1)))].

%% The old term hash of `phash/2` and the term hash of `phash2/1,2` for the
%% term kinds: a bignum, a float, a tuple, a list, a map of the flatmap and
%% the hashmap form, a binary, an atom, `[]`, and `{}`. The value of
%% `phash(Term, 4294967296)` is the old hash plus one, and `phash2(Term,
%% 4294967296)` is the term hash, so the ranges 1, 2, and 3 divide that
%% value. A range of 0, a range above 2^32, and a range that is not an
%% integer are `badarg`.
phash_values() ->
    [phash(Term) || Term <- terms()] ++
        [?CAUSE(erlang:phash(id(a), Range)) || Range <- ranges()] ++
        [?CAUSE(erlang:phash2(id(a), Range)) || Range <- ranges()].

phash(Term) ->
    Hash = erlang:phash(id(Term), 4294967296),
    [Hash, erlang:phash2(id(Term)), erlang:phash2(id(Term), 4294967296),
     erlang:phash(id(Term), 1), erlang:phash(id(Term), 3),
     erlang:phash2(id(Term), 1), erlang:phash2(id(Term), 3)].

terms() ->
    [576460752303423488, 1 bsl 64, -(1 bsl 59), -(1 bsl 64),
     4294967297, -(1 bsl 59) - 1, 1.5, {a, 1, 2}, [1, 2, 3], one(a, 1),
     list_to_binary([1, 2, 3]), abc, -1, 0, 1, [], {}, list_to_binary([]), one(1, 2),
     {[1, 2, {a, b}], one(1, [2]), list_to_binary([3, 4])},
     bytes(100), list_to_binary(bytes(11)), big_map(40), big_list(),
     erlang:make_fun(hash_probe, id, 1), local_fun(length(bytes(11))),
     local_fun(length(bytes(11))), local_fun(length(bytes(12))),
     local_fun2(length(bytes(11)), length(bytes(12))), nofree()].

ranges() ->
    [0, -1, 4294967297, id(1.0), id(default), id(576460752303423488)].

%% A map of 40 keys, which ERTS stores in the hashmap form, and a list of
%% 100 elements that are not bytes.
%% The terms above include an external fun and local funs of this module,
%% with no free variable and with one free variable, so that the fun cases
%% of both term hashes run.

%% A local fun of this module with one free variable, and one with two. The
%% argument is a call, so that the compiler does not specialize the lambda at
%% the call site and drop the free variable.
local_fun(Free) -> fun(X) -> X + Free end.

local_fun2(First, Second) -> fun(X) -> X + First + Second end.

%% A local fun of this module with no free variable.
nofree() -> fun(X) -> X + 1 end.

%% A map of N keys with the key as the value, built by map updates.
big_map(N) -> big_map(N, #{}).

big_map(0, Map) -> Map;
big_map(N, Map) -> big_map(N - 1, Map#{N => N}).

%% A map of one key, built by a map update rather than a literal.
one(Key, Value) -> #{Key => Value}.

big_list() ->
    [{1, 2}, {3, 4}, {5, 6}, {7, 8}, {9, 10}, {11, 12}, {13, 14}].

%% The reductions of `erlang:md5/1` of 100 bytes and of the two term hashes
%% of a map of 40 keys and a list of 100 bytes. The md5 input stays below the
%% byte budget that the caller has at the call, so that the state of the
%% reduction budget does not change the cost. The md5 walk charges one
%% reduction for each byte (erl_bif_chksum.c:463-489), and a phash2 hash
%% charges one reduction for each 64 iterations (erl_term_hashing.c:860-861,
%% 1565-1570). The costs include the reduction of the calls of the probe.
reductions() ->
    Binary = list_to_binary(bytes(100)),
    Map = big_map(40),
    List = bytes(100),
    [cost(fun() -> erlang:md5(id(Binary)) end),
     cost(fun() -> erlang:phash2(id(Map)) end),
     cost(fun() -> erlang:phash2(id(List)) end)].

cost(Fun) ->
    Before = element(2, process_info(self(), reductions)),
    Fun(),
    element(2, process_info(self(), reductions)) - Before.

%% A list of N bytes, from 0 up, so that a walk reads a byte run.
bytes(N) -> bytes(N, 0, []).

bytes(0, _Byte, Acc) -> Acc;
bytes(N, Byte, Acc) -> bytes(N - 1, (Byte + 1) rem 256, [Byte | Acc]).

%% Return the argument, so that the compiler does not fold a constant into
%% the BIF call.
id(X) -> X.
