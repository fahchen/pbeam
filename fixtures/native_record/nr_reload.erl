-module(nr_reload).
-export([main/0]).

main() ->
    io:format("~w~n", [results()]).

%% Code load, delete and reload change the node table of native record
%% definitions that a remote record #nr_def:pub{} reads.
results() ->
    NotLoaded = try_pub(),
    {module, nr_def} = code:ensure_loaded(nr_def),
    Old = #nr_def:pub{a = 1},
    Loaded = try_pub(),
    true = code:delete(nr_def),
    Deleted = try_pub(),
    {ok, V2} = file:read_file("fixtures/native_record/v2/nr_def.beam"),
    {module, nr_def} = erlang:load_module(nr_def, V2),
    Reloaded = [try_pub(), (#nr_def:pub{})#nr_def:pub.c],
    [NotLoaded, Loaded, Deleted, Reloaded, Old#nr_def:pub.a].

try_pub() ->
    try #nr_def:pub{a = id(1)} of
        R -> R#nr_def:pub.b
    catch
        error:E -> E
    end.

id(X) -> X.
