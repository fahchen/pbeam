%% The interpreter dispatch position probe of
%% `docs/interpreter-dispatch-plan.md`. Each call runs
%% `function_at_label`, which scans the instruction vector from index 0 and
%% compares the module name of each `func_info`. Thus the cost of one
%% iteration of the floor loop grows with the functions before the loop.
%%
%% This module places `floor_loop/1` after 20 guest functions, so that the
%% loop runs the scan that the plan's quoted baseline runs. The guest
%% functions exist only to set that position. The benchmark measures the
%% same `test`, `gc_bif '-'`, and `call_only` loop as
%% `fixtures/dispatch/dispatch_probe.erl`.
-module(dispatch_position_probe).

-export([guest_1/0, guest_2/0, guest_3/0, guest_4/0, guest_5/0, guest_6/0, guest_7/0, guest_8/0, guest_9/0, guest_10/0, guest_11/0, guest_12/0, guest_13/0, guest_14/0, guest_15/0, guest_16/0, guest_17/0, guest_18/0, guest_19/0, guest_20/0, floor_loop/1]).

guest_1() -> 1.
guest_2() -> 2.
guest_3() -> 3.
guest_4() -> 4.
guest_5() -> 5.
guest_6() -> 6.
guest_7() -> 7.
guest_8() -> 8.
guest_9() -> 9.
guest_10() -> 10.
guest_11() -> 11.
guest_12() -> 12.
guest_13() -> 13.
guest_14() -> 14.
guest_15() -> 15.
guest_16() -> 16.
guest_17() -> 17.
guest_18() -> 18.
guest_19() -> 19.
guest_20() -> 20.

%% The floor loop with a local tail call: `test`, `gc_bif '-'`, and
%% `call_only`.
floor_loop(0) -> 0;
floor_loop(N) -> floor_loop(N - 1).
