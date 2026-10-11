-module(from_keys_probe).
-export([results/1, improper_after_trap/0]).
-export([measure/2]).

%% The results and the reductions of maps:from_keys/2 (erl_map.c:279-311,
%% 629-642) for each input shape. Each case runs in a new process after a
%% yield, so the BIF starts with the budget of a new turn. The result is
%% the map size, whether the map equals the map of maps:from_list/1 with
%% the same pairs, and whether it is a flatmap (erts_debug:flat_size/1 is
%% not used: PBeam has no heap words).
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that no garbage collection occurs. PBeam
%% has no process heap and gets [].

-define(REDS, element(2, process_info(self(), reductions))).

%% The hashmap build sorts the hashes with erts_qsort, which picks each
%% pivot from addresses that change between ERTS runs (utils.c:3522-3526).
%% A list of more than 32 keys has copies of one key, or of two keys, so the
%% cost does not change with the pivot, as the maps:from_list/1 cases of
%% bif_cost_probe have.
keys(empty) -> [];
keys(flat) -> [b, a, b, 1];
keys(flat_limit) -> seq(1, 32);
keys(dups) -> copies(33, 1);
keys(more_dups) -> copies(400, 1);
keys(trap) -> copies(200000, 1);
keys(two_keys) -> copies(9, 1) ++ copies(73, 2);
keys(two_keys_again) -> copies(73, 1) ++ copies(9, 2);
keys(improper) -> copies(400, 1) ++ b;
keys(not_list) -> a.

cases() ->
    [empty, flat, flat_limit, dups, more_dups, trap, two_keys, two_keys_again, improper,
     not_list].

copies(0, _) -> [];
copies(Count, Term) -> [Term | copies(Count - 1, Term)].

%% A list of 200000 keys with an improper tail: the walk yields before it
%% reaches the tail. ERTS then traps again forever (utils.c:3757-3773).
improper_after_trap() -> maps:from_keys(copies(200000, 1) ++ b, v).

seq(First, Last) when First > Last -> [];
seq(First, Last) -> [First | seq(First + 1, Last)].

results(Opts) ->
    [begin
         Pid = spawn_opt(?MODULE, measure, [self(), Case], Opts),
         receive {Pid, Result, Reductions} -> {Case, Result, Reductions} end
     end || Case <- cases()].

measure(Collector, Case) ->
    Keys = keys(Case),
    Expected = expected(Keys),
    erlang:yield(),
    Before = ?REDS,
    Result = try maps:from_keys(Keys, v) catch Class:Reason -> {Class, Reason} end,
    After = ?REDS,
    Collector ! {self(), summary(Result, Expected), After - Before}.

expected(Keys) ->
    try maps:from_list([{Key, v} || Key <- Keys]) catch _:_ -> none end.

summary(Map, Expected) when is_map(Map) -> {map_size(Map), Map =:= Expected};
summary(Error, _) -> Error.
