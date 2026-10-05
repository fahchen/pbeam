-module(user_drv_state_check).
-export([main/0]).

%% Compares the state of a real disabled-mode user_drv with
%% user_drv_probe:state/3 built from the same pids and reference. Only the
%% tty handle (tty element 2) and ansi_regexp (tty element 19) come from the
%% real state: the probe cannot open a tty handle, and prim_tty:init/1
%% compiles ansi_regexp with re:compile/2.
main() ->
    _ = user_drv:start(#{initial_shell => noshell, input => disabled}),
    {server, Real} = sys:get_state(user_drv),
    RealTty = element(2, Real),
    {Writer, WriterRef} = element(4, RealTty),
    Group = element(8, Real),
    Probe0 = user_drv_probe:state(Group, Writer, WriterRef),
    ProbeTty = setelement(19, setelement(2, element(2, Probe0), element(2, RealTty)),
                          element(19, RealTty)),
    Probe = setelement(2, Probe0, ProbeTty),
    io:format("~p~n", [Real =:= Probe]).
