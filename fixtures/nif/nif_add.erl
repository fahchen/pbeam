-module(nif_add).
-export([load/2, nifs/1, calls/1, trace/1, vetoed/1, reload/2, probes/1, probe/3, add/2,
         other/0, cross_load/0, cross_call/0]).

%% The C library nif_add.c implements add/2. other/0 is a NIF function
%% that the library does not implement, so it runs its stub body.
-nifs([add/2, other/0]).

load(Path, Info) -> erlang:load_nif(Path, Info).

%% The NIFs of the module before and after a load of `Path`
%% (erl_bif_info.c:5769-5805).
nifs(Path) -> {erlang:get_module_info(nif_add, nifs), load(Path, 0), erlang:get_module_info(nif_add, nifs)}.

%% Call add/2 before a load of `Path`, load it, call add/2 and other/0, and
%% load again.
calls(Path) ->
    {try_add(1, 2), load(Path, 0), try_add(1, 2), try_add(-5, 2), try_add(1, a),
     try_add(1 bsl 31, 0), try other() catch Class:Reason -> {Class, Reason} end,
     load(Path, 0)}.

%% The root that loads the library on one scheduler and tells the root of
%% the other scheduler. The test builds the library at this path, relative
%% to the directory of the crate.
cross_load() ->
    ok = load("target/nif_cross/nif_add", 0),
    caller() ! loaded,
    ok.

%% The root that calls the library that the other scheduler loaded.
cross_call() ->
    true = register(nif_caller, self()),
    receive loaded -> try_add(1, 2) end.

caller() ->
    case whereis(nif_caller) of
        undefined ->
            erlang:yield(),
            caller();
        Pid ->
            Pid
    end.

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

%% Load the NIF_ADD_PROBE library `Path` and return the result of each
%% probe call of nif_add.c.
probes(Path) ->
    ok = load(Path, 0),
    %% The compiler keeps the bitstrings out of the literal table.
    Bytes = id(<<1, 2, 3, 4>>),
    <<_:3, Offset:16/bits, _/bits>> = Bytes,
    <<_:8, Part:16/bits, _/bits>> = Bytes,
    [try_probe(Kind, Arg)
     || {Kind, Arg} <- [{0, 7}, {0, -1}, {0, 1 bsl 32 - 1}, {0, 1 bsl 32}, {0, 1 bsl 64}, {0, a},
                        {1, "ab"}, {2, ""}, {2, "abc"}, {2, "abcd"}, {2, "abcde"}, {2, [97 | b]},
                        {2, [97, 256]}, {2, [97, -1]}, {2, [a]}, {2, a}, {2, <<"ab">>}, {3, <<"abc">>},
                        {3, Offset}, {3, Part}, {3, "ab"}, {4, 0}, {a, 0}]].

%% Load the NIF_ADD_PROBE library `Path` and return the result of one
%% probe call.
probe(Path, Kind, Arg) ->
    ok = load(Path, 0),
    try_probe(Kind, Arg).

id(Term) -> Term.

try_probe(Kind, Arg) ->
    try add(Kind, Arg) catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.

try_add(A, B) -> try add(A, B) catch Class:Reason -> {Class, Reason} end.

add(_A, _B) -> erlang:nif_error(undef).

other() -> erlang:nif_error(undef).
