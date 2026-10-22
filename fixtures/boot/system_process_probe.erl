-module(system_process_probe).
-export([exits/0, absent/0, improper/0, loaded/0]).

%% erts_internal:spawn_system_process/3 looks up the export first and stops
%% the emulator with `No function M:F/A` when it finds none
%% (erl_init.c:333-353, 436-456). A BIF has an export. The exit of a system
%% process stops the emulator (erl_process.c:14186-14188), so the wait of
%% the caller does not end.
exits() ->
    erts_internal:spawn_system_process(erlang, display, [system_process]),
    receive after 5000 -> running end.

absent() ->
    erts_internal:spawn_system_process(?MODULE, absent_function, []).

%% erts_list_length gives -1 for an improper list.
improper() ->
    erts_internal:spawn_system_process(?MODULE, exits, [a | b]).

%% erlang:loaded/0 lists the loaded modules, the last loaded first
%% (beam_bif_load.c:762-791).
loaded() ->
    erlang:loaded().
