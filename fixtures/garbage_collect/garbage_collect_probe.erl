-module(garbage_collect_probe).
-export([main/0]).

%% erlang:garbage_collect/0 and erts_internal:garbage_collect/1 with each
%% argument shape (erlang.erl:3052-3053; bif.c:4076-4090).
main() ->
    [erlang:garbage_collect(),
     erts_internal:garbage_collect(minor),
     erts_internal:garbage_collect(major),
     try erts_internal:garbage_collect(full) catch Class:Reason -> {Class, Reason} end,
     try erts_internal:garbage_collect("major") catch Class:Reason -> {Class, Reason} end].
