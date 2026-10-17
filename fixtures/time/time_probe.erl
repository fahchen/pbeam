-module(time_probe).
-export([monotonic/0, system/0, units/0, convert/0, perf_counter/0, posix/0,
         local_badarg/0, local/0, local_check/0, leap_check/0,
         wall_time/0, wall_time_on/0, wall_time_order/0, wall_time_reds/0,
         clock/0, time_offset_units/0, timestamp/0, perf/0, pid/0, pid_value/0,
         environment/0, clock_utc/0, clock_jst/0, leap_local/0,
         ping/1]).

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

%% The relations of `erlang:system_time/0`, `erlang:monotonic_time/0`, and
%% `erlang:time_offset/0` in the native unit (`erl_time_sup.c:2500-2552`):
%% each unit gives the floor of a native value read before and after it,
%% the system time is between 2023 and 2096, the monotonic time is at most
%% 10 s after the start, the offset is positive, the system time is the
%% monotonic time plus the offset within 10 ms, and the system time of the
%% run agrees with `os:system_time/0` within 10 ms.
clock() ->
    S1 = erlang:system_time(),
    SUnits = [erlang:system_time(U)
              || U <- [second, millisecond, microsecond, nanosecond]],
    S2 = erlang:system_time(),
    M1 = erlang:monotonic_time(),
    MUnits = [erlang:monotonic_time(U)
              || U <- [second, millisecond, microsecond, nanosecond]],
    M2 = erlang:monotonic_time(),
    T1 = erlang:time_offset(),
    TUnits = [erlang:time_offset(U)
              || U <- [second, millisecond, microsecond, nanosecond]],
    T2 = erlang:time_offset(),
    OS = os:system_time(),
    [in(1700000000000000000, S1, 4000000000000000000),
     in(1700000000000000000, OS, 4000000000000000000),
     in(?START, M1, ?START + 10000000000),
     in(0, T1, 3400000000000000000),
     le(S1 - 10000000, M1 + T1), le(M1 + T1, S2 + 10000000),
     le(S1 - 10000000, OS), le(OS, S2 + 10000000)]
    ++ floors(S1, SUnits, S2) ++ floors(M1, MUnits, M2)
    ++ floors(T1, TUnits, T2).

%% `floors/3` for the four non-deprecated units of `clock/0`.
floors(N1, [S, Ms, Us, Ns], N2) ->
    [floor_in(N1, S, 1000000000, N2), floor_in(N1, Ms, 1000000, N2),
     floor_in(N1, Us, 1000, N2), floor_in(N1, Ns, 1, N2)].

%% Each unit that `erlang:time_offset/1` does not accept.
time_offset_units() ->
    [catch_error(fun() -> erlang:time_offset(U) end) || U <- [0, -1, foo, {}]].

%% The parts of `erlang:timestamp/0` and `os:timestamp/0` (`erl_time_sup.c:2555-2587`,
%% `erl_bif_os.c:48-59`): each part has the range of its unit, and the
%% value in microseconds is within 10 ms of `erlang:system_time/1` and
%% `os:system_time/1` in microseconds.
timestamp() ->
    {Mega, Sec, Micro} = erlang:timestamp(),
    Us = erlang:system_time(microsecond),
    {OsMega, OsSec, OsMicro} = os:timestamp(),
    OsUs = os:system_time(microsecond),
    [in(0, Mega, 4000), in(0, Sec, 999999), in(0, Micro, 999999),
     in(0, OsMega, 4000), in(0, OsSec, 999999), in(0, OsMicro, 999999),
     le(Us - 10000, micros({Mega, Sec, Micro})),
     le(micros({Mega, Sec, Micro}), Us + 10000),
     le(OsUs - 10000, micros({OsMega, OsSec, OsMicro})),
     le(micros({OsMega, OsSec, OsMicro}), OsUs + 10000)].

%% The microseconds of `{MegaSecs, Secs, MicroSecs}`.
micros({Mega, Sec, Micro}) -> (Mega * 1000000 + Sec) * 1000000 + Micro.

%% The host performance counter of `os:perf_counter/0` in the
%% `perf_counter` unit (`erl_time_sup.c:2601-2605`): it is non-negative and
%% exclusive across a busy loop, and `os:perf_counter/1` gives its floor in
%% the `second` unit (`lib/kernel/src/os.erl:188-191`).
perf() ->
    P1 = os:perf_counter(),
    burn(100000),
    Seconds = os:perf_counter(second),
    P2 = os:perf_counter(),
    [in(0, P1, P2), lt(P1, P2), floor_in(P1, Seconds, 1000000000, P2)].

%% The process identifier of the host as a flat string of its digits
%% (`sys/unix/sys.c:934-939`).
pid() ->
    S = os:getpid(),
    [is_list(S), S =/= [], S =:= integer_to_list(list_to_integer(S)),
     list_to_integer(S) > 0,
     S =:= [D || D <- S, D >= $0, D =< $9]].

%% The process identifier of the host itself.
pid_value() -> os:getpid().

%% Each result of `os:putenv/2`, `os:getenv/1`, `os:unsetenv/1`, and
%% `os:env/0` on the pinned ERTS, with the bad name and value cases
%% (`erl_bif_os.c:82-142`, `erl_osenv.c:269-311`).
environment() ->
    true = os:putenv("TIME_PROBE_VAR", "value"),
    Put = find("TIME_PROBE_VAR", os:env()),
    true = os:putenv("TIME_PROBE_VAR", ""),
    Empty = os:getenv("TIME_PROBE_VAR"),
    true = os:unsetenv("TIME_PROBE_VAR"),
    Missing = find("TIME_PROBE_VAR", os:env()),
    NotFound = os:getenv("TIME_PROBE_VAR"),
    [Put, Empty, Missing, NotFound, has_key("PATH", os:env()),
     catch_error(fun() -> os:putenv("A=B", "x") end),
     catch_error(fun() -> os:putenv(atom, "x") end),
     catch_error(fun() -> os:putenv("A", [0]) end),
     catch_error(fun() -> os:putenv("A", 1) end),
     catch_error(fun() -> os:unsetenv(1) end),
     catch_error(fun() -> os:unsetenv("A=B") end)].

%% Print `ok` when the local clock BIFs of the host time zone agree with
%% `clock_check/1`, and `mismatch` for another time zone. The test gives
%% the zone as `TZ=UTC0` (offset 0) and `TZ=JST-9` (offset 32400).
clock_utc() -> clock_check(0).

clock_jst() -> clock_check(32400).

clock_check(Offset) ->
    Local = erlang:localtime(),
    Universal = erlang:universaltime(),
    Date = erlang:date(),
    Time = erlang:time(),
    Difference = seconds(Local) - seconds(Universal),
    Same = seconds({Date, Time}) - seconds(Local),
    Checks = [in(Offset - 1, Difference, Offset + 1), in(-1, Same, 1)]
        ++ ranges(Local) ++ ranges(Universal) ++ ranges({Date, Time}),
    io:put_chars(case all_true(Checks) of
                     true -> <<"ok\n">>;
                     false -> <<"mismatch\n">>
                 end).

%% Whether each check of a list holds.
all_true([]) -> true;
all_true([true | Rest]) -> all_true(Rest);
all_true(_) -> false.

%% The range checks of one date and time.
ranges({{Y, Mo, D}, {H, Mi, S}}) ->
    [in(1902, Y, 9999), in(1, Mo, 12), in(1, D, 31), in(0, H, 23),
     in(0, Mi, 59), in(0, S, 59)].

%% The seconds since 1970-01-01 of a date and time of a year from 1902.
seconds({{Y, M, D}, {H, Mi, S}}) ->
    (days(Y) + month_days(Y, M) + D - 1) * 86400 + H * 3600 + Mi * 60 + S.

days(Y) -> days_between(1970, Y).

%% The days from the year `From` to the year `Y`, positive forward.
days_between(From, Y) when Y > From -> 365 + leap(From) + days_between(From + 1, Y);
days_between(From, Y) when Y < From -> -days_between(Y, From);
days_between(_, _) -> 0.

%% The days before the month `M` of the year `Y`: the days of the months
%% before `M`, and the leap day for a month after February.
month_days(Y, M) when M > 2 -> leap(Y) + month_days(M);
month_days(_, M) -> month_days(M).

month_days(1) -> 0;
month_days(2) -> 31;
month_days(3) -> 59;
month_days(4) -> 90;
month_days(5) -> 120;
month_days(6) -> 151;
month_days(7) -> 181;
month_days(8) -> 212;
month_days(9) -> 243;
month_days(10) -> 273;
month_days(11) -> 304;
month_days(12) -> 334.

%% The leap day of the year `Y`: 1 for a leap year, else 0.
leap(Y) ->
    case (Y rem 4 =:= 0) andalso ((Y rem 100 =/= 0) orelse (Y rem 400 =:= 0)) of
        true -> 1;
        false -> 0
    end.

%% The pair of `Key` of a list of pairs, or `false`.
find(Key, [{Key, Value} | _]) -> {Key, Value};
find(Key, [_ | Rest]) -> find(Key, Rest);
find(_, []) -> false.

has_key(Key, Pairs) -> find(Key, Pairs) =/= false.

%% Take at least one microsecond of host CPU time.
burn(0) -> ok;
burn(N) -> burn(N - 1).

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

%% Print which path `erlang:localtime/0` takes in a time zone with leap
%% seconds: `get_localtime` reads the local time with `sys_localtime_r`
%% (`erl_time_sup.c:1543-1557`), which adds no leap second
%% (`sys/unix/erl_unix_sys.h:331-343`), so the local time of the fixture
%% zone is 27 seconds before its UTC time.
leap_local() ->
    Difference = seconds(erlang:localtime()) - seconds(erlang:universaltime()),
    io:put_chars(case in(-28, Difference, -26) of
                     true -> <<"localtime_r\n">>;
                     false -> <<"posix2time\n">>
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

%% erts_internal:scheduler_wall_time/1 traps to a receive of the reply of
%% each scheduler (bif.c:5313-5314, erts_internal.erl:759-774), so a
%% process that the caller spawned runs before the call returns.
wall_time_order() ->
    receive after 1 -> ok end,
    spawn(?MODULE, ping, [self()]),
    A = erts_internal:scheduler_wall_time(false),
    B = receive ping -> ping after 0 -> none end,
    [A, B].

%% The reductions between two reads of process_info/2 around
%% erts_internal:scheduler_wall_time(false): the BIF sends a reference to
%% the caller and traps to await_sched_wall_time_modifications/2, which
%% receives it once for each scheduler (bif.c:5306-5315,
%% erl_process.c:1382-1421, erts_internal.erl:756-774).
wall_time_reds() ->
    {reductions, R0} = process_info(self(), reductions),
    false = erts_internal:scheduler_wall_time(false),
    {reductions, R1} = process_info(self(), reductions),
    R1 - R0.

%% Send ping to Pid.
ping(Pid) -> Pid ! ping.
