-module(magic_ref_probe).
-export([main/0, uncaught/0]).

uncaught() ->
    exit(main()).

%% Make one reference of each magic kind between two ordinary references,
%% and give for each: is_reference/1, the magic marker bit 17 of N1 in the
%% printed form and in term_to_binary/2, and the term order against each
%% other reference. ERTS sets the bit for a magic reference
%% (ERTS_REF1_MAGIC_MARKER_BIT__, erl_bif_unique.h:102-105), and erts_cmp
%% compares the numbers N2, N1, N0 (utils.c:2283-2286).
main() ->
    R0 = make_ref(),
    T = ets:new(x, []),
    A = atomics:new(1, []),
    {atomics, C} = counters:new(1, []),
    {bm, B} = binary:compile_pattern(<<"ab">>),
    {ok, {re_pattern, _, _, _, P}} = re:compile("a"),
    Z = prim_buffer:new(),
    R1 = make_ref(),
    Refs = [R0, T, A, C, B, P, Z, R1],
    Cause = try ets:lookup(A, k) catch error:badarg:S -> cause(S) end,
    {[{is_reference(X), printed_bit(X), encoded_bit(X)} || X <- Refs],
     [[order(X, Y) || Y <- Refs] || X <- Refs],
     Cause,
     round_trip(T, A)}.

%% dec_term gives the live magic reference of a magic binary
%% (erts_magic_ref_lookup_bin, external.c:4845-4851).
round_trip(T, A) ->
    ok = atomics:put(A, 1, 7),
    [DT, DA] = [binary_to_term(term_to_binary(X)) || X <- [T, A]],
    LT = binary_to_term(term_to_binary(T, [local])),
    {DT =:= T, ets:lookup(DT, 1), LT =:= T, ets:lookup(LT, 1),
     DA =:= A, atomics:get(DA, 1), dead(T)}.

%% The decoded identifier of a live table stays a table identifier after
%% the table is deleted (tid2tab, erl_db.c:280-306).
dead(T) ->
    D = binary_to_term(term_to_binary(T)),
    B = term_to_binary(T),
    true = ets:delete(T),
    L = binary_to_term(B),
    [{L =:= T, ets:info(X, size), try ets:lookup(X, 1) catch error:badarg:S -> cause(S) end} || X <- [D, L]].

order(X, Y) when X < Y -> lt;
order(X, Y) when X > Y -> gt;
order(_, _) -> eq.

printed_bit(X) ->
    [_, _, _, N1, _] = string:lexemes(erlang:ref_to_list(X), "<.>"),
    (list_to_integer(N1) bsr 17) band 1.

encoded_bit(X) ->
    <<_:14/binary, _:32, N1:32, _:32>> = term_to_binary(X, [local]),
    (N1 bsr 17) band 1.

cause([{_, _, _, Info} | _]) ->
    maps:get(cause, proplists:get_value(error_info, Info)).
