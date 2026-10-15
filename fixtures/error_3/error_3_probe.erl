-module(error_3_probe).
-export([main/0, uncaught/0, device/0]).

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
     device_error()].

id(X) -> X.

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
