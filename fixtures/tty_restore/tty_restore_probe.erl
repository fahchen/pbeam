-module(tty_restore_probe).
-export([main/0, halt/0, crash/0]).

%% Wait for a line of input, then return. The test changes the terminal
%% mode while the probe waits.
main() ->
    wait().

%% Wait for a line, then halt the runtime.
halt() ->
    wait(),
    erlang:halt().

%% Wait for a line, then exit with the reason boom.
crash() ->
    wait(),
    exit(boom).

wait() ->
    "go\n" = io:get_line(standard_io, "ready> "),
    ok.
