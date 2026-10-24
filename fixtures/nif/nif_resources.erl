%% The module of the NIF library fixtures/nif/nif_resources.c: probe/2 runs
%% the resource enif calls of its first argument.
-module(nif_resources).
-export([results/1, mismatches/1, call/3]).

-nifs([probe/2]).
-compile({nowarn_deprecated_function, [{erlang, phash, 2}]}).

%% Load the library `Path` and return the result of each case, in the
%% order of expected/0.
results(Path) ->
    ok = erlang:load_nif(Path, 0),
    Load = probe(0, 0),
    Outside = probe(1, 0),
    A = probe(2, 1),
    B = probe(3, 2),
    D = probe(4, 0),
    Types = [is_reference(A), reference_guard(B), is_reference(D), node(A) =:= node(),
             A =:= A, A =/= B, A < B, lists:sort([D, B, A]) =:= [A, B, D],
             erts_debug:flat_size(A), erts_internal:term_type(A)],
    Gets = [try_probe(5, A), try_probe(6, A), try_probe(6, B), try_probe(5, D),
            try_probe(5, make_ref()), try_probe(5, <<"ab">>), try_probe(5, 1)],
    {M1, M2} = probe(10, A),
    Same = [M1 =:= A, M2 == A, probe(5, M2), [M1] -- [A]],
    Text = [lists:prefix("#Ref<0.", ref_to_list(A)),
            ref_to_list(M1) =:= ref_to_list(A)],
    Hashes = [is_integer(erlang:phash2(A)), is_integer(erlang:phash(A, 1 bsl 32)),
              erlang:phash2(M1) =:= erlang:phash2(A), erlang:phash2(A) =/= erlang:phash2(B)],
    Big = maps:from_list([{A, a} | [{I, I} || I <- lists:seq(1, 40)]]),
    Small = #{B => b},
    Maps = [maps:get(A, Big), maps:get(B, Small), maps:get(M2, Big)],
    External = [binary_to_term(term_to_binary(A)) =:= A,
                binary_to_term(term_to_binary(A)) =:= B],
    Lifetimes = lifetimes(),
    %% A and B stay live until here, so no collection before drops them.
    Live = [probe(5, A), probe(6, B)],
    [Load, Outside, Types, Gets, Same, Text, Hashes, Maps, External, Lifetimes, Live].

%% Load the library `Path` and return each case whose result is not the
%% result of the pinned ERTS, as {Index, Result, Expected}.
mismatches(Path) ->
    Pairs = lists:zip(results(Path), expected()),
    [{Index, Result, Expected}
     || {Index, {Result, Expected}} <- lists:enumerate(Pairs), Result =/= Expected].

%% Load the library `Path` and return the result of one probe call.
call(Path, Kind, Arg) ->
    ok = erlang:load_nif(Path, 0),
    probe(Kind, Arg).

%% The destructor calls: a process that ends drops its terms, the
%% collection of the garbage of this process drops a term that no
%% variable holds, and a C reference keeps an object until its release.
lifetimes() ->
    Parent = self(),
    Empty = log(),
    {_, Monitor} = spawn_monitor(fun() ->
                                         Kept = probe(2, 10),
                                         _ = probe(3, 11),
                                         Parent ! {kept, probe(7, Kept)}
                                 end),
    Kept = receive {kept, Ok} -> Ok end,
    receive {'DOWN', Monitor, process, _, normal} -> ok end,
    AfterExit = log(),
    ok = probe(8, 0),
    AfterRelease = log(),
    _ = probe(2, 20),
    AfterCollection = log(),
    ok = probe(11, 30),
    NoTerm = log(),
    [Empty, Kept, AfterExit, AfterRelease, AfterCollection, NoTerm].

%% Collect the garbage, wait for the scheduled destructors, and return
%% the log of the destructor calls.
log() ->
    erlang:garbage_collect(),
    receive after 50 -> ok end,
    probe(9, 0).

try_probe(Kind, Arg) ->
    try probe(Kind, Arg)
    catch Class:Reason -> {Class, Reason}
    end.

reference_guard(Term) when is_reference(Term) -> true;
reference_guard(_) -> false.

%% The results of the pinned ERTS (OTP 29.1.1).
expected() ->
    [{true, 1, 1, true, 2, true},
     {true, 1},
     [true, true, true, true, true, true, true, true, 3, reference],
     [1, {error, badarg}, 2, {error, badarg}, {error, badarg}, {error, badarg}, {error, badarg}],
     [true, true, 1, []],
     [true, true],
     [true, true, true, true],
     [a, b, a],
     [true, false],
     [[], ok, [1011], [10], [20], [30]],
     [1, 2]].

probe(_Kind, _Arg) ->
    erlang:nif_error(not_loaded).
