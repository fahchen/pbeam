-module(effect_probe).
-export([
    spawn/0,
    spawn_link/0,
    spawn_opt/0,
    spawn_opt_link/0,
    spawn_opt_monitor/0,
    done/0
]).

%% One call of each effect entry that checks a capability. Each function
%% gives `denied` for error:capability_denied.

spawn() -> attempt(fun() -> erlang:spawn(?MODULE, done, []) end).

spawn_link() -> attempt(fun() -> erlang:spawn_link(?MODULE, done, []) end).

spawn_opt() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], []) end).

spawn_opt_link() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], [link]) end).

spawn_opt_monitor() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], [monitor]) end).

attempt(Call) ->
    try Call() of
        _ -> ok
    catch
        error:capability_denied -> denied;
        Class:Reason -> {Class, Reason}
    end.

done() -> ok.
