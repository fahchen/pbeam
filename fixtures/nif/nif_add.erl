-module(nif_add).
-export([load/2, calls/1, trace/1, vetoed/1, reload/2, missing/0, add/2, other/0]).

%% The C library nif_add.c implements add/2. other/0 is a NIF function
%% that the library does not implement, so it runs its stub body.
-nifs([add/2, other/0]).

load(Path, Info) -> erlang:load_nif(Path, Info).

%% Call add/2 before a load of `Path`, load it, call add/2 and other/0, and
%% load again.
calls(Path) ->
    {try_add(1, 2), load(Path, 0), try_add(1, 2), try_add(-5, 2), try_add(1, a),
     try_add(1 bsl 31, 0), try other() catch Class:Reason -> {Class, Reason} end,
     load(Path, 0)}.

%% Load `Path` and return the reason and the first stack frame of add(1, a).
trace(Path) ->
    ok = load(Path, 0),
    try add(1, a) catch error:Reason:Stack -> {Reason, hd(Stack)} end.

%% Load `Path` with the load info 7, then with 0, and call add/2.
vetoed(Path) -> {load(Path, 7), load(Path, 0), try_add(1, 2)}.

%% Load `Path`, load `Binary` as the new code of the module, and load `Path`
%% from the new code, which is an upgrade.
reload(Path, Binary) ->
    ok = load(Path, 0),
    true = erlang:delete_module(nif_add),
    {module, nif_add} = erlang:load_module(nif_add, Binary),
    nif_add:load(Path, 0).

%% Load a library that does not exist.
missing() -> load("/nonexistent/nif_add", 0).

try_add(A, B) -> try add(A, B) catch Class:Reason -> {Class, Reason} end.

add(_A, _B) -> erlang:nif_error(undef).

other() -> erlang:nif_error(undef).
