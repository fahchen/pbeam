-module(init_kill_probe).
-export([kill_init/0]).

%% The boot script init_kill_probe.script runs kill_init/0 in the boot
%% process of init. init is a system process (erl_init.c:355-399), so its
%% exit stops the emulator with `System process <0.0.0> terminated: killed`
%% (erl_process.c:14186-14188), and the wait does not end. The boot file is
%% term_to_binary/1 of the script, as for init_boot_probe.
kill_init() ->
    exit(whereis(init), kill),
    receive after 5000 -> running end.
