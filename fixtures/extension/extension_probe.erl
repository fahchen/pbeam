-module(extension_probe).
-export([load/1, add/2, invalid/0, unbound/1, atom_name/1, dirty_atom_name/1]).

%% The functions of the extension library of the test. The library binds
%% `add/2`, `make_atom/1`, and `dirty_atom/1`; `unbound/1` has no binding,
%% so it runs its stub body.
-nifs([add/2, unbound/1, make_atom/1, dirty_atom/1]).

%% The load callback of the library loads it only for the load info `ok`.
load(Info) -> erlang:load_nif("extension_probe", Info).

add(_A, _B) -> erlang:nif_error(undef).

invalid() ->
    try add(1, a) catch Class:Reason -> {Class, Reason} end.

unbound(_X) -> erlang:nif_error(undef).

%% The binding makes the atom of the binary name on the scheduler thread.
atom_name(Name) -> atom_to_binary(make_atom(Name), utf8).

make_atom(_Name) -> erlang:nif_error(undef).

%% The binding makes the atom of the binary name on a dirty CPU thread.
dirty_atom_name(Name) -> atom_to_binary(dirty_atom(Name), utf8).

dirty_atom(_Name) -> erlang:nif_error(undef).
