%% The module of the NIF library fixtures/nif/nif_atoms.c: probe/2 and
%% dirty_probe/2 use the atoms that the load callback keeps in static
%% variables.
-module(nif_atoms).
-export([results/1, mismatches/1]).

-nifs([probe/2, dirty_probe/2]).

%% Load the library `Path` and return the result of each case, in the
%% order of expected/0.
results(Path) ->
    ok = erlang:load_nif(Path, 0),
    Calls = [probe(0, x), probe(1, 7), probe(2, [tail]), probe(3, x)],
    Sent = [probe(4, x), flush()],
    Kept = [probe(5, x), probe(6, x), probe(6, x)],
    Same = probe(7, x),
    Dirty = [dirty_probe(0, 5), dirty_probe(1, x), flush()],
    [Calls, Sent, Kept, Same, Dirty].

%% Load the library `Path` and return each case whose result is not the
%% result of the pinned ERTS, as {Index, Result, Expected}.
mismatches(Path) ->
    Pairs = lists:zip(results(Path), expected()),
    [{Index, Result, Expected}
     || {Index, {Result, Expected}} <- lists:enumerate(Pairs), Result =/= Expected].

%% Return the messages in the mailbox, in their order.
flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.

probe(_Kind, _Arg) -> erlang:nif_error(not_loaded).

dirty_probe(_Kind, _Arg) -> erlang:nif_error(not_loaded).

%% The results of the pinned ERTS (OTP 29.1.1).
expected() ->
    [[pbeam_nif_atoms_loaded, {ok, pbeam_nif_atoms_loaded, 7},
      [ok, pbeam_nif_atoms_loaded, tail], 23],
     [1, [{pbeam_nif_atoms_loaded, ok}]],
     [ok, pbeam_nif_atoms_kept, pbeam_nif_atoms_kept],
     1,
     [{pbeam_nif_atoms_loaded, 5}, 1, [{pbeam_nif_atoms_loaded, dirty}]]].
