-module(time_probe).
-export([monotonic/0, system/0, units/0, convert/0, perf_counter/0, posix/0,
         local_badarg/0, local/0, local_check/0, leap_check/0,
         wall_time/0, wall_time_on/0]).

%% The Erlang monotonic time at the emulator start in `native`
%% (`erlang:system_info(start_time)` on the pinned ERTS).
-define(START, -576460752000000000).

%% The results of local/0 on the pinned ERTS with
%% `TZ=EST5EDT,M3.2.0,M11.1.0`: UTC-5, and UTC-4 from the second Sunday in
%% March 02:00 to the first Sunday in November 02:00.
-define(LOCAL,
        [{{2026,1,15},{7,0,0}}, {{2026,7,15},{8,0,0}}, {{1969,12,31},{19,0,0}},
         {{2000,2,29},{18,59,59}}, {{2024,2,28},{19,0,0}}, {{1901,12,31},{19,0,0}},
         {{2147483646,12,31},{18,59,59}}, {{2026,3,8},{1,59,59}},
         {{2026,3,8},{3,0,0}}, {{2026,11,1},{1,59,59}}, {{2026,11,1},{1,0,0}}]).

%% Check the relations of `erlang:monotonic_time/1`: the `native` value is
%% at most 10 s after the start, each unit gives the floor of a `native`
%% value between N1 and N2, and the unit 3 traps to
%% `erlang:convert_time_unit/3`.
monotonic() ->
    N1 = erlang:monotonic_time(native),
    Values = [erlang:monotonic_time(U)
              || U <- [second, millisecond, microsecond, nanosecond,
                       seconds, milli_seconds, micro_seconds, nano_seconds,
                       1, 1000, 1000000, 1000000000]],
    T3 = erlang:monotonic_time(id(3)),
    N2 = erlang:monotonic_time(native),
    [in(?START, N1, ?START + 10000000000), in(N1, N2, ?START + 10000000000)
     | relations(N1, N2, Values, T3)].

%% Check the same relations for `os:system_time/1`, with a `native` value
%% between 2023 and 2096.
system() ->
    N1 = os:system_time(native),
    Values = [os:system_time(U)
              || U <- [second, millisecond, microsecond, nanosecond,
                       seconds, milli_seconds, micro_seconds, nano_seconds,
                       1, 1000, 1000000, 1000000000]],
    T3 = os:system_time(id(3)),
    N2 = os:system_time(native),
    [in(1700000000000000000, N1, 4000000000000000000), in(N1, N2, N1 + 10000000000)
     | relations(N1, N2, Values, T3)].

relations(N1, N2, [S, Ms, Us, Ns, OS, OMs, OUs, ONs, IS, IMs, IUs, INs], T3) ->
    [floor_in(N1, S, 1000000000, N2), floor_in(N1, Ms, 1000000, N2),
     floor_in(N1, Us, 1000, N2), floor_in(N1, Ns, 1, N2),
     floor_in(N1, OS, 1000000000, N2), floor_in(N1, OMs, 1000000, N2),
     floor_in(N1, OUs, 1000, N2), floor_in(N1, ONs, 1, N2),
     floor_in(N1, IS, 1000000000, N2), floor_in(N1, IMs, 1000000, N2),
     floor_in(N1, IUs, 1000, N2), floor_in(N1, INs, 1, N2),
     le(T3 * 1000000000, N2 * 3), lt(N1 * 3, (T3 + 1) * 1000000000)].

%% Each unit that the time BIFs do not accept.
units() ->
    Bad = [0, -1, id(-576460752303423489), foo, {}, [], <<>>, self(), make_ref(),
           fun id/1],
    [[catch_error(fun() -> erlang:monotonic_time(U) end) || U <- Bad],
     [catch_error(fun() -> os:system_time(U) end) || U <- Bad]].

convert() ->
    [erts_internal:perf_counter_unit(),
     erlang:convert_time_unit(id(-576460752000000000), 1000000000, 3),
     erlang:convert_time_unit(id(-576460751999999999), 1000000000, 3),
     erlang:convert_time_unit(id(1791221732085333000), 1000000000, 3),
     erlang:convert_time_unit(1, 1000000000, perf_counter),
     erlang:convert_time_unit(-1, 1000000000, perf_counter),
     erlang:convert_time_unit(5, 2, 3),
     erlang:convert_time_unit(-5, 2, 3),
     erlang:convert_time_unit(id(-6), id(2), id(3))].

perf_counter() ->
    erlang:monotonic_time(perf_counter).

%% The date of each 64-bit POSIX time, and `badarg` for each other term.
posix() ->
    [catch_error(fun() -> erlang:posixtime_to_universaltime(T) end)
     || T <- [id(0), id(-1), id(86399), id(86400), id(951782400), id(951868800),
              id(4102444800), id(-2208988800), id(-62167219200), id(-62167219201),
              id(576460752303423488), id(-576460752303423489),
              id(9223372036854775807), id(-9223372036854775808),
              id(foo), id([1]), id({})]].

%% Each date that `erlang:universaltime_to_localtime/1` does not accept: a
%% value out of range, a term that is not `{{Y,M,D},{H,Mi,S}}`, and an
%% element that is not a small integer.
local_badarg() ->
    [catch_error(fun() -> erlang:universaltime_to_localtime(T) end)
     || T <- [{{1901,12,31},{23,59,59}}, {{2147483647,1,1},{0,0,0}},
              {{2100,2,29},{0,0,0}}, {{2026,2,29},{0,0,0}}, {{2026,13,1},{0,0,0}},
              {{2026,0,1},{0,0,0}}, {{2026,4,31},{0,0,0}}, {{2026,1,0},{0,0,0}},
              {{2026,1,1},{24,0,0}}, {{2026,1,1},{0,60,0}}, {{2026,1,1},{0,0,60}},
              {{2026,1,1},{-1,0,0}}, {{2026,1,1},{0,0,0},x}, {{2026,1,1}},
              {2026,1,1}, foo, {{2026,1},{0,0,0}}, {{2026,1,1},{0,0}},
              {{a,1,1},{0,0,0}}, {{2026,1,1},{0,0,a}},
              {{id(576460752303423488),1,1},{0,0,0}},
              {{2026,1,1},{0,0,id(-576460752303423489)}},
              {[2026,1,1],{0,0,0}}]].

%% The local time of each UTC date in the host time zone: winter, summer,
%% the POSIX epoch, leap days, the first and the last year, and the two
%% changes of daylight saving time in 2026.
local() ->
    [erlang:universaltime_to_localtime(T)
     || T <- [{{2026,1,15},{12,0,0}}, {{2026,7,15},{12,0,0}}, {{1970,1,1},{0,0,0}},
              {{2000,2,29},{23,59,59}}, {{2024,2,29},{0,0,0}}, {{1902,1,1},{0,0,0}},
              {{2147483646,12,31},{23,59,59}}, {{2026,3,8},{6,59,59}},
              {{2026,3,8},{7,0,0}}, {{2026,11,1},{5,59,59}}, {{2026,11,1},{6,0,0}}]].

%% Print `ok` when local/0 gives the ERTS results of
%% `TZ=EST5EDT,M3.2.0,M11.1.0`, and `mismatch` for another time zone.
local_check() ->
    io:put_chars(case local() =:= ?LOCAL of
                     true -> <<"ok\n">>;
                     false -> <<"mismatch\n">>
                 end).

%% Print how the local time of a time zone with leap seconds is found.
%% ERTS on macOS adds the leap seconds with `posix2time` before
%% `localtime_r` subtracts them (erl_time_sup.c:1822-1837), so the local
%% time is the UTC time. ERTS on a glibc host has no `posix2time`, so the
%% local time is 27 leap seconds earlier.
leap_check() ->
    io:put_chars(case erlang:universaltime_to_localtime({{2026,1,15},{12,0,0}}) of
                     {{2026,1,15},{12,0,0}} -> <<"posix2time\n">>;
                     {{2026,1,15},{11,59,33}} -> <<"localtime_r\n">>
                 end).

catch_error(F) -> try F() catch error:R -> {error, R} end.

floor_in(N1, V, F, N2) -> in(N1 - F + 1, V * F, N2).

in(Low, V, High) when Low =< V, V =< High -> true;
in(Low, V, High) -> {false, Low, V, High}.

le(A, B) when A =< B -> true;
le(A, B) -> {false, A, B}.

lt(A, B) when A < B -> true;
lt(A, B) -> {false, A, B}.

%% Return the argument. An integer above 2^31 - 1 goes through id/1, so it
%% is an instruction operand and not a literal term.
id(X) -> X.

%% erts_internal:scheduler_wall_time/1, as kernel_refc:init/1 calls it with
%% false (kernel_refc.erl:52-54, 141-142). Each term other than true
%% disables the scheduler wall time and gives the old state
%% (bif.c:5306-5315).
wall_time() ->
    [erts_internal:scheduler_wall_time(X) || X <- [false, foo, 1, false]].

%% erts_internal:scheduler_wall_time/1 with true, which enables it.
wall_time_on() ->
    erts_internal:scheduler_wall_time(true).
