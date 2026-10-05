-module(signal_probe).
-export([main/0]).

%% Set each signal of signalterm_to_signum (sys.c:697-722) and each error
%% case of os_set_signal_2 (erl_bif_os.c:145-160). sigusr1 gets a handler
%% first, so default must reset it.
main() ->
    [try os:set_signal(Signal, Mode) catch error:Reason -> Reason end
     || {Signal, Mode} <- [{sighup, ignore}, {sigusr1, handle}, {sigusr1, default}, {sigusr2, handle},
                           {sigquit, ignore}, {sigabrt, ignore}, {sigalrm, ignore}, {sigterm, ignore},
                           {sigchld, default}, {sigstop, handle}, {sigtstp, ignore},
                           {sigcont, handle}, {sigwinch, handle}, {siginfo, handle},
                           {sigint, handle}, {sigkill, ignore}, {sigsegv, ignore}, {foo, ignore},
                           {1, ignore}, {"sighup", ignore}, {sighup, foo}, {sighup, 1}, {foo, foo}]].
