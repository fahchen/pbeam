-module(clock_signals).
-export([reader/0, writer/0, reference/0]).

-define(ROUNDS, 200).

%% Run the pair of roots on the pinned ERTS, as PBeam runs the two roots on
%% two scheduler threads, and return the results.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, writer()} end),
    _ = spawn(fun() -> Self ! {second, reader()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% The root that sends its clock readings. It sends ROUNDS readings, waits
%% past the first time correction check of an OS-corrected host (1.5 s),
%% and sends ROUNDS readings again.
writer() ->
    true = register(clock_writer, self()),
    Reader = wait_for(clock_reader, 100000),
    ok = send_readings(Reader, ?ROUNDS),
    receive after 1600 -> ok end,
    ok = send_readings(Reader, ?ROUNDS),
    Reader ! done,
    ok.

%% The root that reads its clock after each reading of the other root. Each
%% value must be at or after the value of the other root, and the time
%% offset and the time support settings must be the same.
reader() ->
    true = register(clock_reader, self()),
    check_readings(0).

send_readings(_Reader, 0) ->
    ok;
send_readings(Reader, Count) ->
    Reader ! {reading, readings()},
    send_readings(Reader, Count - 1).

readings() ->
    {erlang:monotonic_time(), erlang:monotonic_time(microsecond),
     erlang:system_time(), erlang:time_offset(), erlang:time_offset(millisecond),
     erlang:system_info(time_correction), erlang:system_info(time_warp_mode)}.

check_readings(Count) ->
    receive
        {reading, {M0, M1, S0, O0, O1, C, W}} ->
            {N0, N1, T0, P0, P1, D, X} = readings(),
            case N0 >= M0 andalso N1 >= M1 andalso T0 >= S0 andalso
                 {P0, P1, D, X} =:= {O0, O1, C, W} of
                true -> check_readings(Count + 1);
                false -> {unexpected, {M0, M1, S0, O0, O1, C, W}, {N0, N1, T0, P0, P1, D, X}}
            end;
        done when Count =:= 2 * ?ROUNDS ->
            ok;
        done ->
            {unexpected, Count}
    after 60000 ->
        timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined ->
            receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid ->
            Pid
    end.
