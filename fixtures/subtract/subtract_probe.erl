-module(subtract_probe).
-export([results/1]).
-export([measure/2]).

%% The results and the reductions of erlang:'--'/2 for each stage of
%% subtract_continue (erl_bif_lists.c:931-1000), with and without a trap,
%% and for each badarg. Each case runs in a new process after a yield, so
%% the BIF starts with the budget of a new turn.
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that no garbage collection occurs. PBeam
%% has no process heap and gets [].

-define(REDS, element(2, process_info(self(), reductions))).

%% The left and the right list of each case. seq/2 and copies/2 build the
%% lists at run time, so the compiler does not fold a subtraction.
args(nil_nil) -> {[], []};
args(nil_list) -> {[], seq(1, 3)};
args(list_nil) -> {seq(1, 3), []};
args(naive_left) -> {[1, 2, 1, 3 | seq(4, 5)], [1, 3, 9, 1, 1]};
args(naive_right) -> {seq(1, 20), [3, 5, 3, 25]};
args(naive_right_used) -> {seq(1, 20), [2, 4]};
args(set) -> {seq(1, 40) ++ seq(1, 40), seq(10, 30) ++ [99]};
args(set_used) -> {seq(1, 40), seq(10, 30) ++ seq(10, 30)};
args(set_terms) -> {[b, {1}, "a", 2, b, <<"x">> | seq(1, 20)], [b, "a", {1}, <<"x">> | seq(1, 20)]};
args(length_left_trap) -> {copies(200000, 7), [8]};
args(naive_left_trap) -> {seq(1, 10), copies(20000, 1)};
args(length_right_trap) -> {seq(1, 20), copies(200000, 1)};
args(naive_right_trap) -> {copies(200000, 7), seq(1, 10)};
args(set_build_trap) -> {seq(1, 20), seq(1, 100000)};
args(set_finish_trap) -> {copies(200000, 7), seq(1, 20)};
args(not_list_left) -> {a, []};
args(not_list_right) -> {[], a};
args(improper_left) -> {[1, 2 | a], [1]};
args(improper_right) -> {[], [1 | a]};
args(improper_naive_left) -> {[1, 2], [1, 3 | a]};
args(improper_after_trap) -> {[1, 2], copies(20000, 1) ++ a};
args(improper_right_long) -> {seq(1, 20), seq(1, 20) ++ a}.

cases() ->
    [nil_nil, nil_list, list_nil, naive_left, naive_right, naive_right_used, set, set_used,
     set_terms, length_left_trap, naive_left_trap, length_right_trap, naive_right_trap,
     set_build_trap, set_finish_trap, not_list_left, not_list_right, improper_left,
     improper_right, improper_naive_left, improper_after_trap, improper_right_long].

seq(First, Last) when First > Last -> [];
seq(First, Last) -> [First | seq(First + 1, Last)].

copies(0, _) -> [];
copies(Count, Term) -> [Term | copies(Count - 1, Term)].

%% Return {Case, Summary, Reductions} for each case. The summary of a long
%% result is its length and its first and last elements.
results(Opts) ->
    [begin
         Pid = spawn_opt(?MODULE, measure, [self(), Case], Opts),
         receive {Pid, Result, Reductions} -> {Case, Result, Reductions} end
     end || Case <- cases()].

measure(Collector, Case) ->
    {Left, Right} = args(Case),
    erlang:yield(),
    Before = ?REDS,
    Result = try Left -- Right catch Class:Reason -> {Class, Reason} end,
    After = ?REDS,
    Collector ! {self(), summary(Result), After - Before}.

summary(List) when is_list(List), length(List) > 30 ->
    {length(List), hd(List), last(List)};
summary(Term) -> Term.

last([Last]) -> Last;
last([_ | Rest]) -> last(Rest).
