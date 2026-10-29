-module(init_boot_probe).
-export([start_recorder/0, step/1, report/0]).

%% The boot script init_boot_probe.script runs in the boot process of init
%% (init.erl:1192-1217). Its kernelProcess command starts the recorder
%% (init.erl:1519-1528), and each apply command records its name and the
%% boot status of init (init.erl:1352-1354, 451-452). The last apply
%% command runs report/0 in the boot process.
%% `mise run fixture-beam` compiles this file. The boot file is
%% term_to_binary/1 of the script, as systools:script2boot/1 writes it:
%% erl -noshell -eval '{ok, [S]} = file:consult("fixtures/boot/init_boot_probe.script"), ok = file:write_file("fixtures/boot/init_boot_probe.boot", term_to_binary(S)), halt().'

start_recorder() ->
    Pid = spawn(fun() -> record([]) end),
    true = register(init_boot_probe_recorder, Pid),
    {ok, Pid}.

record(Steps) ->
    receive
        {step, Step} ->
            record([Step | Steps]);
        {steps, From} ->
            From ! {steps, reverse(Steps, [])},
            record(Steps)
    end.

step(Name) ->
    init_boot_probe_recorder ! {step, {Name, init:get_status()}},
    ok.

%% Report the boot from a process that waits until the boot process exits,
%% so init is in its main loop (init.erl:700-704, 783-797).
report() ->
    Boot = self(),
    Watcher = spawn(fun() -> watch(Boot) end),
    receive {Watcher, monitored} -> ok end.

watch(Boot) ->
    Ref = monitor(process, Boot),
    Boot ! {self(), monitored},
    Reason = receive {'DOWN', Ref, process, Boot, Exit} -> Exit end,
    Init = whereis(init),
    init_boot_probe_recorder ! {steps, self()},
    Steps = receive {steps, Recorded} -> Recorded end,
    Linked = element(2, process_info(Init, links)),
    Recorder = whereis(init_boot_probe_recorder),
    erlang:display({boot_exit, Reason,
                    init, pid_to_list(Init), process_info(Init, registered_name),
                    status, started(100),
                    script_id, init:script_id(),
                    recorder_linked_to_init, member(Recorder, Linked),
                    steps, Steps,
                    names, [{Name, is_pid(whereis(Name))}
                            || Name <- [init, erl_prim_loader,
                                        init__boot__on_load_handler,
                                        init_boot_probe_recorder,
                                        erts_code_purger]],
                    purger, [process_info(whereis(erts_code_purger), Item)
                             || Item <- [priority, group_leader]],
                    alive, is_process_alive(Init)}),
    halt().

%% init leaves its boot loop at the exit message of the boot process,
%% which can come after the monitor message of this process.
started(0) ->
    init:get_status();
started(Tries) ->
    case init:get_status() of
        {started, started} = Status ->
            Status;
        _ ->
            receive after 10 -> started(Tries - 1) end
    end.

reverse([H | T], Acc) -> reverse(T, [H | Acc]);
reverse([], Acc) -> Acc.

member(X, [X | _]) -> true;
member(X, [_ | T]) -> member(X, T);
member(_, []) -> false.
