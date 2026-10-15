-module(error_3_probe).
-export([main/0, uncaught/0, device/0, root/0]).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location [] or [{error_info, Map}]. stack/1 drops the file and line
%% of an io frame, because PBeam keeps no line table, and drops the frames
%% of the erl -eval caller.
main() ->
    [try_stack(fun() -> erlang:error(r, [a], []) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, #{cause => c}}]) end),
     try_stack(fun() -> erlang:error(r, none, [{error_info, #{}}]) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, #{a => 1}}, {error_info, #{b => 2}}]) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, notmap}]) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, #{}}, foo]) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, #{}} | bad]) end),
     try_stack(fun() -> erlang:error(r, [a], [{error_info, #{}, x}]) end),
     try_stack(fun() -> erlang:error(r, [a], bad) end),
     try_stack(fun() -> body(id([b])) end),
     device_error() | tail_calls()].

id(X) -> X.

%% A tail call of erlang:error/3 through a fun or through apply/3 with a
%% variable module and function, and through a fun as the initial call of
%% a process (beam_common.c:1245-1253, 1345-1431).
tail_calls() ->
    [try_stack(fun() -> fun_tail(fun erlang:error/3) end),
     try_stack(fun() -> apply_tail(erlang, error) end),
     initial_call()].

fun_tail(F) ->
    F(r, [a], [{error_info, #{fun_tail => 1}}]).

apply_tail(M, F) ->
    apply(M, F, [r, [a], [{error_info, #{apply_tail => 1}}]]).

initial_call() ->
    {_, Ref} = spawn_monitor(?MODULE, root, []),
    receive {'DOWN', Ref, process, _, Reason} -> Reason end.

%% The fun call is the last call of the only frame of the process.
root() ->
    F = id(fun erlang:error/3),
    F(r, [a], [{error_info, #{root => 1}}]).

%% The root process exits with the result of main/0, so the run gives its
%% text.
uncaught() ->
    exit(main()).

%% A body call keeps the frame of the caller in the stack trace.
body(Args) ->
    V = erlang:error(r, Args, [{error_info, #{module => ?MODULE}}]),
    id(V).

%% An IO device that replies {error, foo} makes io:format/2 raise badarg
%% with the error_info cause {device, foo} (io.erl:203-207).
device_error() ->
    Old = group_leader(),
    Device = spawn(?MODULE, device, []),
    group_leader(Device, self()),
    Result = try_stack(fun() -> io:format("x~w", [1]) end),
    group_leader(Old, self()),
    exit(Device, kill),
    Result.

device() ->
    receive
        {io_request, From, ReplyAs, _} ->
            From ! {io_reply, ReplyAs, {error, foo}},
            device()
    end.

try_stack(F) ->
    try F() catch C:R:S -> {C, R, stack(S)} end.

stack([{M, F, A, L} | S]) when M =:= io -> [{M, F, A, [I || {error_info, _} = I <- L]} | stack(S)];
stack([{M, _, _, _} = Frame | S]) when M =:= ?MODULE -> [Frame | stack(S)];
stack(_) -> [].
