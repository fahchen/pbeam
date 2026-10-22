-module(extension_probe).
-export([load/1, calls/1, loaded/0, reload/1, old_load/1, with_load/2, add/2, invalid/0, unbound/1, atom_name/1,
         dirty_atom_name/1]).

%% The functions of the extension library of the test. The library binds
%% `add/2`, `make_atom/1`, and `dirty_atom/1`; `unbound/1` has no binding,
%% so it runs its stub body.
-nifs([add/2, unbound/1, make_atom/1, dirty_atom/1]).

%% The load callback of the library loads it only for the load info `ok`.
load(Info) -> erlang:load_nif("extension_probe", Info).

%% Call `add/2` before a load with `Info`, after it, and load again.
calls(Info) -> {try_add(), load(Info), try_add(), load(Info)}.

loaded() -> calls(ok).

%% Load the library, delete the module, load `Binary` as its new code, and call
%% the new code, whose load is an upgrade.
reload(Binary) ->
    First = calls(ok),
    true = erlang:delete_module(extension_probe),
    {module, extension_probe} = erlang:load_module(extension_probe, Binary),
    {First, extension_probe:calls(ok)}.

%% Delete the module, load `Binary` as its new code, call `erlang:load_nif/2`
%% from the old code, and call the new code.
old_load(Binary) ->
    true = erlang:delete_module(extension_probe),
    {module, extension_probe} = erlang:load_module(extension_probe, Binary),
    {load(ok), extension_probe:calls(ok)}.

%% Load the library and apply `Name` to `Arguments`.
with_load(Name, Arguments) ->
    ok = load(ok),
    apply(?MODULE, Name, Arguments).

try_add() ->
    try add(1, 2) catch Class:Reason -> {Class, Reason} end.

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
