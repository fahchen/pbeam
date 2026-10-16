-module(kernel_exit_probe).
-export([main/0]).

%% Kill erl_prim_loader, a kernel process of init (init.erl:1178-1184).
%% init halts the node with "Kernel pid terminated (erl_prim_loader)
%% (killed)" (init.erl:783-787, 1139-1148). The probe waits for the halt.
main() ->
    exit(whereis(erl_prim_loader), kill),
    receive after infinity -> ok end.
