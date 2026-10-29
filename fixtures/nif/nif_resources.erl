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
    Table = ets:new(resources, [set]),
    true = ets:insert(Table, [{A}, {B}]),
    Spec = [{{'$1'}, [{is_reference, '$1'}, {'==', {node, '$1'}, node()}], [{{'$1'}}]}],
    Ets = [lists:sort(ets:select(Table, Spec)) =:= [{A}, {B}], ets:lookup(Table, A) =:= [{A}]],
    Lifetimes = lifetimes(),
    %% A and B stay live until here, so no collection before drops them.
    Live = [probe(5, A), probe(6, B)],
    Monitors = monitors(),
    Magic = magic(),
    [Load, Outside, Types, Gets, Same, Text, Hashes, Maps, External, Ets, Lifetimes, Live,
     Monitors, Magic].

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
%% enif_monitor_process of a live process gives 0, and of a process that
%% is not alive 1. The exit of the live process runs the down callback of
%% each monitor that a demonitor did not remove, while a reference keeps
%% its object.
monitors() ->
    {Live, Reference} = spawn_monitor(fun() -> receive stop -> ok end end),
    {LiveResult, Object} = probe(13, Live),
    {Dead, DeadReference} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', DeadReference, process, _, _} -> ok end,
    {DeadResult, _} = probe(13, Dead),
    {First, Second, Same, Differ, Demonitor, Again, Pair} = probe(14, Live),
    Released = probe(15, Live),
    NoDown = probe(16, Live),
    Live ! stop,
    receive {'DOWN', Reference, process, _, _} -> ok end,
    Log = log(),
    %% The kill reaches the victim before the monitor signal on PBeam, which
    %% sends the monitor signal through the inbox of the scheduler.
    {Victim, VictimReference} = spawn_monitor(fun() -> receive stop -> ok end end),
    {VictimResult, VictimObject} = probe(13, Victim),
    exit(Victim, kill),
    receive {'DOWN', VictimReference, process, _, _} -> ok end,
    VictimLog = log(),
    [LiveResult, DeadResult, {First, Second, Same, Differ, Demonitor, Again}, Released, NoDown,
     Log, VictimResult, VictimLog, downs(), probe(17, 0), is_reference(Object),
     is_reference(Pair), is_reference(VictimObject)].

%% A resource term is a magic reference: it sorts with the references of
%% make_ref/0 by its creation, the BIFs of an alias, a monitor, or a timer
%% take it as a reference that is none of them, and binary_to_term/1 of
%% it gives the live resource.
magic() ->
    Before = make_ref(),
    C = probe(2, 40),
    Order = [Before < C, later_ref(C, 1 bsl 19)],
    Sends = [C ! hello, erlang:send(C, hi), exit(C, kill), exit(C, normal)],
    Monitors = [demonitor(C), demonitor(C, [info]), demonitor(C, [flush]), unalias(C)],
    Timers = [erlang:cancel_timer(C), erlang:read_timer(C), erlang:cancel_timer(C, [{info, false}]),
              erlang:cancel_timer(C, [{async, true}]), erlang:read_timer(C, [{async, true}])],
    Messages = receive_all(),
    Decoded = binary_to_term(term_to_binary(C)),
    [Order, Sends, Monitors, Timers, [M =:= {T, C, false} || {T, _, _} = M <- Messages],
     length(Messages), Decoded =:= C, probe(5, Decoded)].

%% Return true when one of the next N references of make_ref/0 is greater
%% than Ref: the magic marker of Ref is below the counter bits of the
%% second reference number (erl_bif_unique.h:102-105).
later_ref(_, 0) -> false;
later_ref(Ref, N) -> make_ref() > Ref orelse later_ref(Ref, N - 1).

receive_all() ->
    receive Message -> [Message | receive_all()] after 0 -> [] end.

%% Return the messages of the down callback, in their order.
downs() ->
    receive {down, Id} -> [Id | downs()] after 0 -> [] end.

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
     [true, true],
     [[], ok, [1011], [10], [20], [30]],
     [1, 2],
     [0, 1, {0, 0, true, true, 0, 1}, 0, -1, [8, 2005, 2006], 0, [2005], [5, 6, 5], {true, 2},
      true, true, true],
     [[true, true], [hello, hi, true, true], [true, false, true, false], [false, false, ok, ok, ok],
      [true, true], 2, true, 40]].

probe(_Kind, _Arg) ->
    erlang:nif_error(not_loaded).
