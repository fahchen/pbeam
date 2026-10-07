-module(run_end_probe).
-export([main/0, crash/0, killed/0]).

%% Start a process that starts a 5000 ms timer again at each expiry, as the
%% repeated file sync of logger_h_common does (logger_h_common.erl:298-311,
%% 454-457), and a process that writes a line after 300 ms. Then write a line
%% and return.
main() ->
    start(),
    io:put_chars("Hello, world!\n").

%% Start the same processes, then exit with the reason boom.
crash() ->
    start(),
    exit(boom).

%% Start the same processes and a process that kills the caller. Then wait.
killed() ->
    start(),
    Self = self(),
    _ = spawn(fun() -> exit(Self, kill) end),
    receive after infinity -> ok end.

start() ->
    _ = spawn(fun repeat/0),
    _ = spawn(fun() -> receive after 300 -> io:put_chars("late\n") end end),
    ok.

repeat() ->
    Ref = erlang:start_timer(5000, self(), repeated_filesync),
    receive {timeout, Ref, repeated_filesync} -> repeat() end.
