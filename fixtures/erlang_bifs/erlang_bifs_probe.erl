-module(erlang_bifs_probe).
-export([main/0, id/1]).

%% The small `erlang` BIFs that the `plug_hello` release and the OTP path
%% import: `integer_to_list/2` (bif.c:3341-3374), `setelement/3`
%% (bif.c:2976-3002), `make_tuple/2,3` (bif.c:3006-3078),
%% `insert_element/3` (bif.c:3110-3142), `delete_element/2`
%% (bif.c:3144-3180), the rounding BIFs (erl_bif_guard.c:134-204), the
%% term text BIFs (bif.c:4505-4517), and the clock BIFs
%% (erl_time_sup.c:2500-2525).
%%
%% The stack trace of each caught error is reduced to its first frame: the
%% frame of the BIF with its arity, which ERTS `save_stacktrace` puts
%% first (`beam_common.c:880-1055`). The locations and the arguments of the
%% frame, and the frames of the caller, vary between the runs; the caller
%% frames of a BIF error are the subject of `bif_stack_probe`. The argument
%% list of a frame of a function of arity 0 is the integer 0.
-define(CAUSE(Expr),
        try Expr catch Class:Reason:Stack -> {Class, Reason, first_frame(Stack)} end).

first_frame([{Module, Function, Arguments, _Location} | _Rest]) ->
    [{Module, Function, frame_arity(Arguments)}];
first_frame(_Frames) ->
    [].

frame_arity(Arguments) when is_list(Arguments) -> length(Arguments);
frame_arity(_Arguments) -> 0.

main() ->
    [integer_to_list(), tuples(), rounding(), terms(), clock()].

%% A base from 2 to 36, an upper-case digit above 9, and the bignum path
%% of `erts_big_to_list`. A base that is not a small integer from 2 to 36,
%% and a term that is not an integer, raise `badarg`.
integer_to_list() ->
    [integer_to_list(id(255), 16), integer_to_list(id(-255), 2),
     integer_to_list(id(35), 36), integer_to_list(id(-1), 10),
     integer_to_list(id(576460752303423488), 16)]
        ++ [?CAUSE(integer_to_list(id(1), B))
            || B <- [1, 37, a, id(576460752303423488)]]
        ++ [?CAUSE(integer_to_list(id(T), 10)) || T <- [a, 1.5, <<"1">>]].

%% The 1-based index of each tuple BIF, and the tuple size of
%% `make_tuple/2,3`: 0 to 2^24 - 1. `setelement/3` copies the tuple, so
%% the original tuple stays as it is.
tuples() ->
    Original = {a, b},
    Copy = setelement(id(1), Original, c),
    [setelement(id(1), {a, b}, c), setelement(id(2), {a, b}, c),
     erlang:insert_element(id(1), {a, b}, c), erlang:insert_element(id(3), {a, b}, c),
     erlang:delete_element(id(1), {a, b}), erlang:delete_element(id(1), {a}),
     erlang:make_tuple(id(0), a), erlang:make_tuple(id(2), a), erlang:make_tuple(id(2), x, []),
     erlang:make_tuple(id(3), x, [{2, b}, {1, a}]), Copy, Original]
        ++ [?CAUSE(setelement(id(0), {a, b}, c)),
            ?CAUSE(setelement(id(3), {a, b}, c)),
            ?CAUSE(setelement(id(1), {}, c)),
            ?CAUSE(setelement(id(1), a, c)),
            ?CAUSE(setelement(id(576460752303423488), {a, b}, c)),
            ?CAUSE(erlang:insert_element(id(0), {a, b}, c)),
            ?CAUSE(erlang:insert_element(id(4), {a, b}, c)),
            ?CAUSE(erlang:insert_element(id(1), a, c)),
            ?CAUSE(erlang:delete_element(id(2), {a})),
            ?CAUSE(erlang:delete_element(id(1), {})),
            ?CAUSE(erlang:delete_element(id(1), a)),
            ?CAUSE(erlang:make_tuple(id(-1), a)),
            ?CAUSE(erlang:make_tuple(id(16777216), a)),
            ?CAUSE(erlang:make_tuple(a, b)),
            ?CAUSE(erlang:make_tuple(id(576460752303423488), a)),
            ?CAUSE(erlang:make_tuple(id(-1), x, [])),
            ?CAUSE(erlang:make_tuple(id(2), x, [{3, b}])),
            ?CAUSE(erlang:make_tuple(id(2), x, [{1, a, b}])),
            ?CAUSE(erlang:make_tuple(id(2), x, [{1, a} | b])),
            ?CAUSE(erlang:make_tuple(id(2), x, [a])),
            ?CAUSE(erlang:make_tuple(id(1), x, [{2, a}])),
            ?CAUSE(erlang:make_tuple(id(0), x, [{1, a}]))].

%% `trunc/1`, `floor/1`, and `ceil/1` round toward zero, toward negative
%% infinity, and toward positive infinity. `round/1` rounds to the nearest
%% integer and away from zero at a halfway case. A value of 2^59 has a
%% bignum result. A term that is not a number raises `badarg`.
rounding() ->
    [trunc(id(N)) || N <- [1.5, -1.5, 0.5, -0.5, 0.0, 7, -7,
                           576460752303423488.0, 1.0e300]]
        ++ [round(id(N)) || N <- [0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.0, 7]]
        ++ [floor(id(N)) || N <- [1.5, -1.5, -0.5, 0.0, -7]]
        ++ [ceil(id(N)) || N <- [1.5, -1.5, 0.5, 0.0, -7, -576460752303423488.0]]
        ++ [?CAUSE(trunc(id(a))), ?CAUSE(round(id(<<"1">>))),
            ?CAUSE(floor(id([1]))), ?CAUSE(ceil(id({1})))].

%% The text of a pid, the `badarg` of a term that is not a port, and the
%% `badarg` of a term that is not a pid. The process number varies between
%% the runs, so the probe gives the characters of the text that do not
%% vary. A run of the probe reaches no port term: `erlang:ports/0` is not
%% implemented, so the text of a port is the subject of a unit test of the
%% runtime.
terms() ->
    Text = erlang:pid_to_list(self()),
    [text_shape(Text, last_char(Text)), length(Text) > 4,
     ?CAUSE(erlang:pid_to_list(id(a))), ?CAUSE(erlang:pid_to_list(id(1))),
     ?CAUSE(erlang:port_to_list(id(self()))), ?CAUSE(erlang:port_to_list(id(a)))].

%% The text of a pid is `<0.Number.Serial>` (`bif.c:4505-4510`): a `<`, a
%% `0`, a `.`, the number, a `.`, the serial, and a `>`. The number and the
%% serial vary between the runs.
text_shape([$<, $0, $. | _Rest], $>) -> true;
text_shape(_Text, _Last) -> false.

%% The clock BIFs give the native time unit, which is one nanosecond, with
%% the offset of the Erlang monotonic time. The values vary between the
%% runs, so the probe gives the shape of each value.
clock() ->
    System = erlang:system_time(),
    Mono = erlang:monotonic_time(),
    Os = os:system_time(nanosecond),
    [is_integer(System), abs(Os - System) < 1000000000,
     is_integer(Mono), Mono < 0, Mono > -576460753000000000,
     abs(Mono - erlang:monotonic_time(nanosecond)) < 1000000000].

last_char([Character]) -> Character;
last_char([_ | Rest]) -> last_char(Rest).

%% Return the argument, so that the compiler does not fold a constant into
%% the BIF call.
id(X) -> X.
