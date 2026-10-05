-module(logger_start_probe).
-export([reverse/0]).

%% `lists:reverse/2` (erl_bif_lists.c:1246-1263). logger_simple_h:replay_buffer/1
%% (logger_simple_h.erl:150) calls it with the buffer and the drop message, and
%% lists:reverse/1 (lists.erl:298-299) calls it for three or more elements. An
%% empty first list returns the second term, and a first term that is not a
%% list raises badarg.
reverse() ->
    [lists:reverse(id([a, b, c]), id([d])),
     lists:reverse(id([]), id(foo)),
     lists:reverse(id([a]), id([])),
     lists:reverse(id([]), id([])),
     lists:reverse(id([a, b, c, d])),
     try lists:reverse(id(foo), id([])) catch error:R1 -> R1 end,
     try lists:reverse(id(<<1>>), id(foo)) catch error:R2 -> R2 end].

id(X) -> X.
