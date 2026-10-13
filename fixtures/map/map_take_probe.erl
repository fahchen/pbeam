-module(map_take_probe).
-export([main/0, aliases/0, aliasing/0, retained/0, keys/0, guarded/0,
         caught/0, badmap/1, exact/2, assoc/3, exact_self/1, assoc_self/2,
         exact_self_key/2, shared_value/1, id/1]).

%% A same-register `put_map_exact`: the source and the destination of the
%% instruction are one register, and the value of the pair is an operand.
exact(Map, Value) when is_map(Map) -> Map#{1 := Value}.

%% A same-register `put_map_assoc` with a key and a value that the caller
%% gives. A key that the map does not hold.
assoc(Map, Key, Value) when is_map(Map) -> Map#{Key => Value}.

%% A `put_map_exact` whose value is the source map. The value operand names
%% the register that the change also writes, so ERTS must read it before the
%% change and the new map holds the old map.
exact_self(Map) when is_map(Map) -> Map#{1 := Map}.

%% A `put_map_assoc` whose value is the source map, with a key from a caller
%% register.
assoc_self(Map, Key) when is_map(Map) -> Map#{Key => Map}.

%% A `put_map_exact` whose key and value are caller registers, and whose
%% value is the source map.
exact_self_key(Map, Key) when is_map(Map) -> Map#{Key := Map}.

%% A `put_map_exact` whose value is a map that the source holds.
shared_value(Map) when is_map(Map) ->
    Inner = maps:get(i, Map),
    Map#{1 := Inner}.

%% Each helper changes one same-register copy and returns the result and
%% each old root. The key and the value of the pair are terms of the
%% registers of the caller, so every operand read completes before the
%% change. Every alias keeps the pinned ERTS value.
aliases() ->
    Map = ?MODULE:id(#{1 => 1, 2 => 2}),
    Key = ?MODULE:id(7),
    Value = ?MODULE:id(9),
    Exact = ?MODULE:exact(Map, Value),
    Alias = ?MODULE:id(#{1 => 1, 2 => 2, 7 => 7}),
    AliasResult = ?MODULE:assoc(Alias, Key, Value),
    Other = Map#{1 => 3},
    [Exact, Map, Key, Value, Alias, AliasResult, Other].

%% The alias cases: the value of the pair is the source map, the key and the
%% value are caller registers, and the value is a map that the source holds.
%% Each result keeps the pinned ERTS value.
aliasing() ->
    ExactSelf = ?MODULE:exact_self(?MODULE:id(#{1 => 0, 2 => 2})),
    Key = ?MODULE:id(7),
    AssocSelf = ?MODULE:assoc_self(?MODULE:id(#{2 => 2}), Key),
    ExactSelfKey = ?MODULE:exact_self_key(?MODULE:id(#{1 => 0, 2 => 2}), ?MODULE:id(1)),
    Inner = ?MODULE:id(#{x => 1}),
    SharedValue = ?MODULE:shared_value(?MODULE:id(#{1 => 0, i => Inner, 2 => 2})),
    [ExactSelf, AssocSelf, ExactSelfKey, SharedValue].

%% An update that shares the form and the descendants with an old root. The
%% old root keeps its pairs, and the shared inner term is not written.
retained() ->
    Old = ?MODULE:id(#{1 => #{1 => 1}}),
    Same = ?MODULE:id(Old),
    Updated = Same#{2 => 2},
    Inner = ?MODULE:id(#{1 => 1}),
    Outer = ?MODULE:id(#{1 => Inner}),
    Copy = ?MODULE:id(Outer),
    Changed = Copy#{1 := Inner#{2 => 2}},
    [Old, Same, Updated, Inner, Outer, Changed,
     maps:get(1, Old), maps:get(1, Changed)].

%% An invalid key. `put_map_exact` raises `error:{badkey, Key}`, and
%% `put_map_assoc` adds or replaces the pair. A term that is not a map
%% raises `error:{badmap, Term}`.
keys() ->
    Map = ?MODULE:id(#{1 => 1}),
    NoKey = try Map#{2 := 2} catch error:Reason1 -> Reason1 end,
    Exact = Map#{1 := 2},
    Added = Map#{2 => 2},
    Repeated = Map#{1 => 3},
    NotMap = ?MODULE:id(not_a_map),
    BadMap = try NotMap#{1 := 2} catch error:Reason2 -> Reason2 end,
    [NoKey, Exact, Added, Repeated, Map, BadMap].

%% A guard uses `put_map_exact` with a fail label. A missing key fails the
%% guard, and the source register keeps its value.
guarded() ->
    Map = ?MODULE:id(#{1 => 1}),
    Present = case Map of
        M when is_map(M), M#{1 := 2} =/= #{} -> present;
        _ -> absent
    end,
    Missing = case Map of
        N when is_map(N), N#{2 := 2} =/= #{} -> present;
        _ -> absent
    end,
    [Present, Missing, Map].

%% A same-register `put_map_exact` of a term that is not a map. The
%% instruction raises `error:{badmap, Term}`, and a live catch tag gives the
%% process another frame to run.
badmap(Term) ->
    try Term#{1 := 2} of
        _ -> no_error
    catch
        error:Reason -> Reason
    end.

%% A caught exception keeps the source register and each alias. The catch
%% gives the exception class and reason.
caught() ->
    Map = ?MODULE:id(#{1 => 1}),
    Alias = ?MODULE:id(Map),
    Result = try Map#{2 := 2} of
                 _ -> no_error
             catch
                 Class:Reason -> {Class, Reason, Map, Alias}
             end,
    [Result, Map, Alias].

main() ->
    [aliases(), aliasing(), retained(), keys(), guarded(), caught()].

id(Term) -> Term.
