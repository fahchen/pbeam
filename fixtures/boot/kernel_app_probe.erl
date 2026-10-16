-module(kernel_app_probe).
-export([main/0]).

%% Write the started applications and the id, type, and modules of each
%% child of kernel_sup after the boot of start_clean.boot. kernel:init/1
%% gives the children (kernel.erl:91-227), and application:start_boot/2
%% starts kernel and stdlib (init.erl:1335-1361). The pid of a child
%% differs between runs, so the probe drops it.
main() ->
    Children = [{Id, Type, Modules} || {Id, _, Type, Modules} <- supervisor:which_children(kernel_sup)],
    io:format("~w~n", [{application:which_applications(), Children}]).
