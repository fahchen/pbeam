-module(bif_start_probe).
-export([downs/0, sent/0]).

%% Start a process at each BIF and return the reason of its 'DOWN' message.
downs() ->
    [down(M, F, A) || {M, F, A} <- starts()].

%% Start a process at `erlang:send/2`, and return the message that the BIF
%% sends and the reason of the 'DOWN' message.
sent() ->
    Reason = down(erlang, send, [self(), started]),
    receive Message -> {Message, Reason} end.

starts() ->
    [{erlang, throw, [x]},
     {erlang, exit, [x]},
     {erlang, error, [x]},
     {erlang, error, [x, [1]]},
     {erlang, error, [x, [1], [{error_info, #{a => 1}}]]},
     {erlang, abs, [-1]},
     {erlang, apply, [erlang, error, [x]]}].

down(M, F, A) ->
    {_, Ref} = spawn_monitor(M, F, A),
    receive {'DOWN', Ref, process, _, Reason} -> Reason end.
