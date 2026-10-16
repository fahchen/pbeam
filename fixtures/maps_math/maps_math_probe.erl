-module(maps_math_probe).
-export([values/0, updates/0, math/0, math_errors/0, math_bignums/0, show/1,
         show_reductions/1]).
-export([measure/2, run_caught/2]).

%% Return the term unchanged. The calls through id/1 keep the compiler from
%% folding the math calls at compile time.
id(Term) -> Term.

-define(REDS, element(2, process_info(self(), reductions))).

%% A map of the keys 1..N, which gives a flatmap for at most 32 keys and a
%% hashmap above that. Integer keys keep the map order independent of the
%% order of the atom table, which changes between ERTS runs.
map(N) ->
    maps:from_list([{I, I * 2} || I <- seq(1, N)]).

seq(First, Last) when First > Last -> [];
seq(First, Last) -> [First | seq(First + 1, Last)].

sum([]) -> 0;
sum([Term | Terms]) -> Term + sum(Terms).

%% The pairs of a map in map order, as maps:to_list/1 builds them
%% (maps.erl:525-542). The fixture calls the BIF `erts_internal:map_next/3`
%% directly, so it loads no stdlib module.
pairs(Map) ->
    collect(erts_internal:map_next(0, Map, [])).

collect([Iter, Map | Acc]) when is_integer(Iter) ->
    collect(erts_internal:map_next(Iter, Map, Acc));
collect(Acc) ->
    Acc.

%% maps:values/1 (`erl_map.c:2262-2310`): the values in map order, [] for
%% the empty map, and error:{badmap, Map} for a term that is not a map.
values() ->
    [maps:values(#{}),
     maps:values(#{1 => a, 2 => b, 3 => c}),
     maps:values(map(32)),
     maps:values(map(33)),
     maps:values(map(40)),
     caught(fun() -> maps:values(1) end),
     caught(fun() -> maps:values([]) end),
     caught(fun() -> maps:values({a}) end)].

%% maps:update/3 (`erl_map.c:2036-2103`, `2241-2253`): the map with the key
%% set to the value. The key must be in the map. A flatmap keeps its key
%% order, a hashmap keeps its layout, and a value that the map already
%% holds gives the same value back.
updates() ->
    [maps:update(2, a, #{1 => x, 2 => y, 3 => z}),
     maps:update(1, x, #{1 => x, 2 => y}),
     maps:update(a, 1, #{a => 1}),
     maps:update(1, a, map(32)),
     maps:update(32, a, map(32)),
     maps:update(1, a, map(33)),
     maps:update(17, a, map(33)),
     maps:update(33, a, map(33)),
     maps:update(33, 66, map(40)),
     caught(fun() -> maps:update(9, x, #{1 => y}) end),
     caught(fun() -> maps:update(1, x, #{}) end),
     caught(fun() -> maps:update(a, x, #{1 => y}) end),
     caught(fun() -> maps:update(34, x, map(33)) end),
     caught(fun() -> maps:update(a, x, not_a_map) end),
     caught(fun() -> maps:update(a, x, []) end)].

%% The math BIFs of one argument (`erl_math.c:35-63`, `235-272`) and
%% math:pow/2 (`erl_math.c:98-141`, `279-282`): the double of the function
%% of the C library, also for a bignum operand and a negative result.
math() ->
    [math:exp(id(0)),
     math:exp(id(1)),
     math:exp(id(-1)),
     math:exp(id(-1000)),
     math:exp(id(709.782712893384)),
     math:log(id(1)),
     math:log(id(2)),
     math:log(id(1000)),
     math:log(id(1 bsl 62)),
     math:log10(id(100)),
     math:log10(id(2)),
     math:sqrt(id(4)),
     math:sqrt(id(0)),
     math:sqrt(id(2)),
     math:sqrt(id(1 bsl 62)),
     math:pow(id(2), id(10)),
     math:pow(id(2), id(0.5)),
     math:pow(id(2), id(-1)),
     math:pow(id(0.0), id(0.0)),
     math:pow(id(0.0), id(0.5)),
     math:pow(id(-2.0), id(3)),
     math:pow(id(-2.0), id(2)),
     math:pow(id(0.5), id(1000))].

%% A result that is not finite raises badarith, and an operand that is not
%% a number raises badarg (`erl_math.c:53-58`, `117-136`). Each error holds
%% the frames of the call: the BIF frame with its arguments, the fun of the
%% case, and the entry function of the process.
math_errors() ->
    [caught(fun() -> math:sqrt(id(-1)) end),
     caught(fun() -> math:sqrt(id(-1.0)) end),
     caught(fun() -> math:log(id(0)) end),
     caught(fun() -> math:log(id(0.0)) end),
     caught(fun() -> math:log(id(-1)) end),
     caught(fun() -> math:log10(id(0)) end),
     caught(fun() -> math:log10(id(-1.0)) end),
     caught(fun() -> math:exp(id(1000)) end),
     caught(fun() -> math:exp(id(1 bsl 62)) end),
     caught(fun() -> math:pow(id(0), id(-1)) end),
     caught(fun() -> math:pow(id(0.0), id(-0.5)) end),
     caught(fun() -> math:pow(id(-8), id(0.5)) end),
     caught(fun() -> math:pow(id(1.0e300), id(2)) end),
     caught(fun() -> math:pow(id(2), id(1 bsl 62)) end),
     caught(fun() -> math:pow(id(a), id(1)) end),
     caught(fun() -> math:pow(id(1), id(a)) end),
     caught(fun() -> math:pow(id([]), id([])) end),
     caught(fun() -> math:sqrt(id([])) end),
     caught(fun() -> math:exp(id(#{})) end),
     caught(fun() -> math:log10(id(a)) end)].

%% The bignum cases without the stack trace: the text of the frame of such
%% a case would hold all 309 digits of the operand. A bignum that
%% `big_to_double` cannot convert raises badarith for math:sqrt/1,
%% math:log/1, math:exp/1, and math:pow/2, also when the other operand is a
%% number.
math_bignums() ->
    [caught_reason(fun() -> math:sqrt(id(1 bsl 1024)) end),
     caught_reason(fun() -> math:sqrt(id(-(1 bsl 1024))) end),
     caught_reason(fun() -> math:log(id(1 bsl 1024)) end),
     caught_reason(fun() -> math:exp(id(1 bsl 1024)) end),
     caught_reason(fun() -> math:pow(id(1 bsl 1024), id(1)) end),
     caught_reason(fun() -> math:pow(id(1), id(1 bsl 1024)) end)].

%% The text of the reductions for the spawn options Opts.
show_reductions(Opts) ->
    iolist_to_binary(text(reductions(Opts))).

%% The reductions of maps:values/1 (`erl_map.c:2299`): 15 units for each
%% reduction, one unit for each key of a hashmap and no unit for a flatmap.
%% Each case runs in a new process after a yield, so the BIF starts with the
%% budget of a new turn. The big map traps. The sum of the values of a map
%% of N keys is N*(N+1).
reductions(Opts) ->
    [begin
         Pid = spawn_opt(?MODULE, measure, [self(), Case], Opts),
         receive {Pid, Result, Reds} -> {Case, Result, Reds} end
     end || Case <- reduction_cases()].

reduction_cases() ->
    [empty, flat32, hash33, hash100, hash100000].

reduction_arg(empty) -> #{};
reduction_arg(flat32) -> map(32);
reduction_arg(hash33) -> map(33);
reduction_arg(hash100) -> map(100);
reduction_arg(hash100000) -> map(100000).

summary(Values) when is_list(Values) ->
    {length(Values), sum(Values)};
summary(Other) ->
    Other.

measure(Collector, Case) ->
    Arg = reduction_arg(Case),
    erlang:yield(),
    Before = ?REDS,
    Result = try maps:values(Arg) catch Class:Reason -> {Class, Reason} end,
    After = ?REDS,
    Collector ! {self(), summary(Result), After - Before}.

%% The value of a call, or {Class, Reason, Frames} for an exception. Each
%% frame is its module, function, and arguments, without the location.
%%
%% The new process ends at run_caught/2, so the stack trace holds no frame
%% of the caller of caught/1. The fixture and the pinned ERTS then give the
%% same frames, also when ERTS runs the fixture from erl -eval.
caught(Fun) ->
    Pid = spawn(?MODULE, run_caught, [self(), Fun]),
    receive {Pid, Result} -> Result end.

%% The same without the frames.
caught_reason(Fun) ->
    case caught(Fun) of
        {Class, Reason, _Frames} -> {Class, Reason};
        Value -> Value
    end.

run_caught(Collector, Fun) ->
    Collector ! {self(),
                 try
                     Fun()
                 catch
                     Class:Reason:Stack -> {Class, Reason, frames(Stack)}
                 end}.

frames(Stack) ->
    [{Module, Function, Arguments} || {Module, Function, Arguments, _Location} <- Stack].

%% Return the text of the result of a function of this module, as `~w`
%% writes a term of numbers, atoms, binaries, tuples, maps, and lists. The
%% text of a float is its shortest text. The function uses no io_lib code,
%% which prints an atom with erl_scan and the init arguments.
show(Name) ->
    iolist_to_binary(text(?MODULE:Name())).

text(Term) when is_float(Term) -> float_to_binary(Term, [short]);
text(Term) when is_integer(Term) -> integer_to_binary(Term);
text(Term) when is_atom(Term) -> atom_to_binary(Term, utf8);
text(Term) when is_binary(Term) -> [<<"<<\"">>, Term, <<"\">>">>];
text(Term) when is_map(Term) -> [$#, ${, join_text([pair(Pair) || Pair <- pairs(Term)]), $}];
text(Term) when is_tuple(Term) -> [${, join(tuple_to_list(Term)), $}];
text(Term) when is_list(Term) -> [$[, join(Term), $]].

pair({Key, Value}) -> [text(Key), <<"=>">>, text(Value)].

join(Terms) -> join_text([text(Term) || Term <- Terms]).

join_text([]) -> [];
join_text([Term]) -> Term;
join_text([Term | Terms]) -> [Term, $,, join_text(Terms)].
