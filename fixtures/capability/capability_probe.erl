-module(capability_probe).
-export([
    main/0,
    modules/0,
    undefined/0,
    inherit/0,
    reduce/0,
    empty/0,
    undelegable/0,
    widen/0,
    malformed/0,
    done/0,
    report/1
]).

%% Each spawn BIF, with and without a link or a monitor.
main() ->
    [
        attempt(fun() -> spawn(?MODULE, done, []) end),
        attempt(fun() -> spawn_link(?MODULE, done, []) end),
        attempt(fun() -> spawn_monitor(?MODULE, done, []) end),
        attempt(fun() -> spawn_opt(?MODULE, done, [], [link]) end),
        attempt(fun() -> spawn_opt(?MODULE, done, [], [monitor]) end)
    ].

%% A spawn of an MFA of this module, and a spawn of a fun, which
%% erlang:spawn/1 starts as erlang:apply/2.
modules() ->
    [
        attempt(fun() -> spawn(?MODULE, done, []) end),
        attempt(fun() -> spawn(fun done/0) end)
    ].

%% A spawn of a function that this module does not export.
undefined() ->
    [attempt(fun() -> spawn(?MODULE, missing, []) end)].

%% main/0 in a child with the default spawn options.
inherit() ->
    in_child([], main).

%% main/0 in a child that gets only spawn and monitor.
reduce() ->
    in_child([{capabilities, #{process => #{operations => [spawn, monitor]}}}], main).

%% main/0 in a child that gets no capability.
empty() ->
    in_child([{capabilities, #{}}], main).

%% main/0 in a grandchild of a child whose process capability is not
%% delegable.
undelegable() ->
    in_child([{capabilities, #{process => #{delegable => false}}}], inherit).

%% Requests for more than the delegable capabilities of the caller.
widen() ->
    [
        with([{capabilities, #{process => #{operations => [spawn]}}}]),
        with([{capabilities, #{process => #{operations => [spawn, link]}}}]),
        with([{capabilities, #{process => #{modules => [capability_probe]}}}]),
        with([{capabilities, #{process => #{modules => any}}}]),
        with([{capabilities, #{process => #{modules => [capability_probe, lists]}}}]),
        with([{capabilities, #{terminal => #{}}}])
    ].

%% Option values that are not valid, then one valid value of each kind.
malformed() ->
    [
        with([{capabilities, all}]),
        with([{capabilities, #{1 => #{}}}]),
        with([{capabilities, #{bogus => #{}}}]),
        with([{capabilities, #{process => all}}]),
        with([{capabilities, #{process => #{1 => []}}}]),
        with([{capabilities, #{process => #{operations => spawn}}}]),
        with([{capabilities, #{process => #{operations => [1]}}}]),
        with([{capabilities, #{process => #{operations => [write]}}}]),
        with([{capabilities, #{process => #{delegable => yes}}}]),
        with([{capabilities, #{process => #{names => any}}}]),
        with([{capabilities, #{process => #{modules => 3}}}]),
        with([{capabilities, #{process => #{modules => ["lists"]}}}]),
        with([{capabilities, #{terminal => #{modules => any}}}]),
        with([{capabilities, #{persistent_term => #{names => any}}}]),
        with([{capabilities, #{persistent_term => #{operations => [halt]}}}]),
        with([{capabilities, #{env => #{names => [[-1]]}}}]),
        with([{capabilities, #{env => #{names => [[a]]}}}]),
        with([
            {capabilities, #{
                process => #{operations => [spawn, link, monitor, exit_other], modules => any, delegable => true},
                terminal => #{operations => [write, read, raw_mode, window_size]},
                port => #{operations => [spawn, command, close], executables => ["/bin/echo"]},
                env => #{operations => [read, write], names => ["HOME", ""]},
                signal => #{operations => [handle, ignore], signals => [sighup]},
                persistent_term => #{operations => [read, write]}
            }}
        ])
    ].

%% Spawn done/0 with Options.
with(Options) ->
    attempt(fun() -> spawn_opt(?MODULE, done, [], Options) end).

%% Run Function in a child that spawn_opt/4 starts with Options and a
%% monitor, and return its result. The child gives the result as its exit
%% reason, so a child without the process capability can give it.
in_child(Options, Function) ->
    try spawn_opt(?MODULE, report, [Function], [monitor | Options]) of
        {_, Monitor} ->
            receive
                {'DOWN', Monitor, process, _, {result, Result}} -> Result
            end
    catch
        error:capability_denied -> denied;
        Class:Reason -> {Class, Reason}
    end.

report(Function) ->
    exit({result, ?MODULE:Function()}).

attempt(Spawn) ->
    try Spawn() of
        Pid when is_pid(Pid) -> ok;
        {Pid, Reference} when is_pid(Pid), is_reference(Reference) -> ok
    catch
        error:capability_denied -> denied;
        Class:Reason -> {Class, Reason}
    end.

done() -> ok.
