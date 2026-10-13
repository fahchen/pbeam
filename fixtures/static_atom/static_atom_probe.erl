%% A probe for the comparison of an atom name with a static name. Each
%% expression compares the name of an atom with a static name in one changed
%% group: the table BIFs, the process flags, and the binary split options.
%% The first two calls set a flag, so the return value of the later calls
%% does not depend on the initial flag. The result is the same on the pinned
%% ERTS and on PBeam, and it names no host value.
-module(static_atom_probe).

-export([main/0]).

main() ->
    _ = erlang:process_flag(trap_exit, true),
    _ = erlang:process_flag(message_queue_data, off_heap),
    [erts_internal:garbage_collect(minor),
     erts_internal:garbage_collect(major),
     binary:split(<<1, 2, 1, 2>>, <<1>>, [trim]),
     binary:split(<<1, 1, 1>>, <<1>>, [trim_all]),
     erlang:process_flag(trap_exit, false),
     erlang:process_flag(message_queue_data, on_heap)].
