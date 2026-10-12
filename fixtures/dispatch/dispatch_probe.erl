%% The interpreter dispatch probe of `docs/interpreter-dispatch-plan.md`.
%% Each loop is a tail call, and its hot clause holds three instructions:
%% `test`, `gc_bif '-'`, and the call. The name of the function says which
%% instruction the call body adds to the floor loop.
%%
%% `floor_ext_loop/1` makes a module-qualified call (`call_ext_only`). The
%% plan says that the floor loop "includes one external call", so this loop
%% is the floor loop of the plan. `floor_loop/1` makes a local `call_only`
%% call, as the plan and the task also name. The benchmark measures both,
%% because the two cost different work on PBeam.
%%
%% `tests/integration/dispatch_baseline.rs` disassembles each loop and fails
%% when the compiled shape changes, so the measured work is the work that
%% the benchmark claims to measure.
-module(dispatch_probe).

-export([floor_ext_loop/1, floor_loop/1, plus_loop/2, tuple_size_loop/3,
         element_loop/3, hd_loop/3, map_get_loop/4, map_1000/0, tuple3/0,
         list3/0, key/0]).

%% The floor loop with a module-qualified tail call: `test`, `gc_bif '-'`,
%% and `call_ext_only`. The loop holds one external call, as the plan says.
floor_ext_loop(0) -> 0;
floor_ext_loop(N) -> ?MODULE:floor_ext_loop(N - 1).

%% The floor loop with a local tail call: `test`, `gc_bif '-'`, and
%% `call_only`.
floor_loop(0) -> 0;
floor_loop(N) -> floor_loop(N - 1).

%% The floor loop plus one `gc_bif '+'`.
plus_loop(0, Acc) -> Acc;
plus_loop(N, Acc) -> plus_loop(N - 1, Acc + 1).

%% The floor loop plus the `bif` of `tuple_size/1`.
tuple_size_loop(0, Acc, _Tuple) -> Acc;
tuple_size_loop(N, _Acc, Tuple) -> tuple_size_loop(N - 1, tuple_size(Tuple), Tuple).

%% The floor loop plus the `bif` of `element/2`. ERTS names this BIF
%% `get_tuple_element`.
element_loop(0, Acc, _Tuple) -> Acc;
element_loop(N, _Acc, Tuple) -> element_loop(N - 1, element(2, Tuple), Tuple).

%% The floor loop plus the `bif` of `hd/1`. ERTS names this BIF `get_hd`.
hd_loop(0, Acc, _List) -> Acc;
hd_loop(N, _Acc, List) -> hd_loop(N - 1, hd(List), List).

%% The floor loop plus the `bif` of `map_get/2`. The map has 1000 keys, so
%% the HAMT walk of the BIF is part of the measured cost.
map_get_loop(0, Acc, _Map, _Key) -> Acc;
map_get_loop(N, _Acc, Map, Key) -> map_get_loop(N - 1, maps:get(Key, Map), Map, Key).

%% A map with the integer keys 1 to 1000. The benchmark makes it once and
%% passes it to each `map_get_loop/4` run.
map_1000() -> map_1000(1, #{}).

map_1000(1001, Map) -> Map;
map_1000(Key, Map) -> map_1000(Key + 1, Map#{Key => Key}).

%% The tuple of the `tuple_size_loop/3` and `element_loop/3` runs.
tuple3() -> {a, b, c}.

%% The list of the `hd_loop/3` run.
list3() -> [a, b, c].

%% The key of the `map_get_loop/4` run.
key() -> 500.
