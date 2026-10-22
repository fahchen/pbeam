-module(prim_file_dirty_probe).
-export([fifo/1, errors/1]).

%% Read the FIFO Path with prim_file:read_file/1 in a new process, and
%% return the status of the reader while its read waits, then the result of
%% the read. ERTS runs read_file_nif/1 on a dirty IO scheduler
%% (prim_file_nif.c:212), so the caller runs while the read waits for a
%% writer, and the reader has the status running (erl_process.c:9319-9328).
%% The open of the FIFO for write is also a dirty IO call (prim_file_nif.c:179).
fifo(Path) ->
    Self = self(),
    Reader = spawn(fun() -> Self ! {read, prim_file:read_file(Path)} end),
    receive after 100 -> ok end,
    {status, Status} = lists:keyfind(status, 1, process_info(Reader)),
    {ok, Fd} = prim_file:open(Path, [write]),
    ok = prim_file:write(Fd, <<"dirty">>),
    ok = prim_file:close(Fd),
    receive {read, Result} -> {Status, Result} end.

%% Return the results of the dirty IO prim_file NIFs for the directory Dir,
%% which has no entry "none". The prim_file functions catch each exception
%% of a NIF (prim_file.erl:136-157, 567, 612, 659), so a result is a value
%% or an error tuple.
errors(Dir) ->
    Missing = <<Dir/binary, "/none">>,
    {ok, Fd} = prim_file:open(Dir, [read, directory]),
    {ok, Cwd} = prim_file:get_cwd(),
    [prim_file:read_file(Missing),
     prim_file:read_file_info(Missing),
     prim_file:open(Missing, [read]),
     prim_file:list_dir(Missing),
     prim_file:read(Fd, 1),
     prim_file:position(Fd, {foo, 1}),
     prim_file:position(Fd, bof),
     prim_file:close(Fd),
     prim_file:close(Fd),
     prim_file:read(Fd, 1),
     is_list(Cwd)].
