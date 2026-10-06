-module(boot_probe).
-export([main/0]).

%% Check the group leader and the trap_exit flag of the entry process, then
%% write "Hello, world!\n" to standard_io. A failed match exits with status 1.
main() ->
    User = whereis(user),
    User = group_leader(),
    true = process_flag(trap_exit, true),
    ok = io:put_chars(standard_io, [<<"Hello, world!">>, 10]).
