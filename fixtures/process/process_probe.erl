-module(process_probe).
-export([main/0, service/0, worker/0]).

main() ->
    process_flag(trap_exit, true),
    {Service, Monitor} = spawn_monitor(?MODULE, service, []),
    Linked = spawn_link(?MODULE, worker, []),
    Reference = make_ref(),
    self() ! {noise, Reference},
    Service ! {request, self(), Reference},
    Io = receive
        {io_reply, Reference, ok} -> io_ok
    end,
    Down = receive
        {'DOWN', Monitor, process, Service, normal} -> down_ok
    end,
    Exit = receive
        {'EXIT', Linked, probe_exit} -> exit_ok
    after 0 ->
        no_exit
    end,
    Noise = receive
        {noise, Reference} -> noise_ok
    end,
    {Io, Down, Exit, Noise}.

service() ->
    receive
        {request, Caller, Reference} -> Caller ! {io_reply, Reference, ok}
    end.

worker() -> exit(probe_exit).
