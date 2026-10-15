-module(b2l_probe).
-export([results/0, costs/1, lows/1, pretty/0]).
-export([measure/3, measure_low/4, id_/1]).

%% The results and the reductions of erlang:binary_to_list/1,3 on the
%% pinned ERTS JIT (binary.c:182-411).

%% The reduction total of the calling process. element/2 is a guard BIF,
%% so it costs no reduction.
-define(REDS, element(2, process_info(self(), reductions))).

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> ?MODULE:id_(X).
id_(X) -> X.

catch_error(Fun) ->
    try Fun()
    catch error:Reason -> {error, Reason}
    end.

%% A binary of 16 bits at bit offset 4: <<16#BC, 16#DE>>.
unaligned() ->
    <<_:4, Sub:16/bits, _/bits>> = id(<<16#AB, 16#CD, 16#EF>>),
    Sub.

%% PBeam forms no bitstring of a size that is not a whole number of bytes
%% yet, so the badarg of such a bitstring is not here.
%% The results and the badarg cases of each arity.
results() ->
    One = [catch_error(fun() -> binary_to_list(Term) end)
           || Term <- [<<>>, <<1, 2, 3>>, unaligned(), id(foo), id([1]), id(1)]],
    Three = [catch_error(fun() -> binary_to_list(Term, Start, Stop) end)
             || {Term, Start, Stop} <- [{<<1, 2, 3>>, 1, 3}, {<<1, 2, 3>>, 2, 2},
                                        {<<1, 2, 3>>, 3, 3}, {<<1, 2, 3>>, 2, 3},
                                        {unaligned(), 1, 2}, {unaligned(), 2, 2},
                                        {<<1, 2, 3>>, 0, 1}, {<<1, 2, 3>>, 1, 4},
                                        {<<1, 2, 3>>, 3, 2}, {<<>>, 1, 1}, {<<>>, 0, 0},
                                        {<<1, 2, 3>>, 1.0, 2}, {<<1, 2, 3>>, -1, 2},
                                        {<<1, 2, 3>>, 1, 1 bsl 64},
                                        {foo, 1, 1}, {<<1>>, a, 1}]],
    {One, Three}.

%% For each input, the reductions of the call in a new process, without
%% the reductions of an empty call.
costs(Opts) ->
    Base = delta(none, Opts),
    [{Input, delta(Input, Opts) - Base} || Input <- inputs()].

%% {one, N} is binary_to_list/1 of N bytes. {three, N, Start, Stop} is
%% binary_to_list/3 of N bytes. The budget at the call is 3997: N below
%% 3997 * 256 = 1023232 is one chunk, and a chunk of a new turn has
%% 4001 * 256 bytes.
inputs() ->
    [{one, 1}, {one, 256}, {one, 257}, {one, 10000}, {one, 1023231}, {one, 1023232},
     {one, 2000000}, {one, 3000000}, {three, 10, 1, 10}, {three, 2000000, 1, 10},
     {three, 2000000, 1000001, 2000000}, {three, 3000000, 2, 2999999}].

delta(Input, Opts) ->
    collect(spawn_opt(?MODULE, measure, [self(), Input, arg(Input)], [monitor | Opts])).

%% Wait for the report of a monitored worker. The worker reports after it
%% checks its result, so a wrong result or a crash of the worker stops the
%% collector with an error. After an ok report, wait for the normal exit of
%% the worker, so that no 'DOWN' message stays in the mailbox.
collect({Pid, Ref}) ->
    receive
        {Pid, {ok, Delta}} -> receive {'DOWN', Ref, process, Pid, normal} -> Delta end;
        {Pid, {error, What}} -> error({check, What});
        {'DOWN', Ref, process, Pid, Reason} -> error({worker, Reason})
    end.

%% Report the reduction delta after the check of the result.
report(Collector, Delta, Input, Arg, Result) ->
    Report = try check(Input, Arg, Result) of
                 ok -> {ok, Delta}
             catch Class:Reason -> {error, {Class, Reason}}
             end,
    Collector ! {self(), Report}.

arg(none) -> none;
arg({one, N}) -> bytes(N);
arg({three, N, _, _}) -> bytes(N).

%% A binary of N bytes 7. erlang:iolist_to_iovec/1 puts the bytes of a
%% list into one binary.
bytes(N) ->
    [Binary] = erlang:iolist_to_iovec(copies(N, 7)),
    Binary.

copies(0, _) -> [];
copies(N, Term) -> [Term | copies(N - 1, Term)].

%% Switch out, so that the call runs with the budget of a new turn.
measure(Collector, Input, Arg) ->
    erlang:yield(),
    Before = ?REDS,
    Result = run(Input, Arg),
    Delta = ?REDS - Before,
    report(Collector, Delta, Input, Arg, Result).

run(none, _) -> [];
run({one, _}, Binary) -> binary_to_list(Binary);
run({three, _, Start, Stop}, Binary) -> binary_to_list(Binary, Start, Stop).

check(none, _, _) -> ok;
check({one, N}, _, List) -> N = length(List), true = all7(List), ok;
check({three, _, Start, Stop}, _, List) ->
    Length = Stop - Start + 1, Length = length(List), true = all7(List), ok.

all7([7 | Rest]) -> all7(Rest);
all7([]) -> true.

%% For each budget left K, the reductions of the call after
%% erlang:bump_reductions/1 leaves K: a binary of N bytes with
%% N < 1000 * 256 = 256000 yields with 100 or fewer, and else runs in
%% one chunk.
lows(Opts) ->
    [{Input, K, low_delta(Input, K, Opts)}
     || Input <- [{one, 100000}, {three, 200000, 1, 10}, {one, 300000}],
        K <- [101, 100, 0]].

low_delta(Input, K, Opts) ->
    collect(spawn_opt(?MODULE, measure_low, [self(), Input, arg(Input), K], [monitor | Opts])).

%% The budget after the bump is 3998 - (3996 - K) - 1 - 1 = K, with 1 for
%% bump_reductions/1 and 1 for the call of run/2.
measure_low(Collector, Input, Arg, K) ->
    erlang:yield(),
    Before = ?REDS,
    erlang:bump_reductions(3996 - K),
    Result = run(Input, Arg),
    Delta = ?REDS - Before,
    report(Collector, Delta, Input, Arg, Result).

%% The text of ~p for binaries, which io_lib_pretty:printable_bin/5 checks
%% with binary_to_list/1,3, as one binary with a line for each binary.
pretty() ->
    Texts = [io_lib:format("~p", [Term])
             || Term <- [<<1, 2>>, <<"hello">>, <<"h", 233, "llo">>, list_to_binary(copies(100, $a))]],
    iolist_to_binary(lists:join($\n, Texts)).
