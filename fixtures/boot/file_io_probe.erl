-module(file_io_probe).
-export([main/0]).

%% Write a scratch file with file:write_file/2, then open it in write mode
%% with file:open/2, which starts a file_io_server, and write, move the
%% position, and close it. Then read the file back and write each result.
%% The path of the scratch file is in the environment variable
%% FILE_IO_PROBE_PATH, in a directory that holds no other file.
%%
%% file:write_file/2 opens the file through file_server_2 and prim_file in
%% write mode, and the file_io_server of file:open/2 opens it with
%% raw_file_io and prim_file (file.erl, file_io_server.erl,
%% raw_file_io_raw.erl, prim_file.erl). In write mode, prim_file keeps no
%% read buffer, so no prim_buffer NIF runs (prim_file.erl:512-527). The
%% append open gives write too, because append alone keeps a read buffer.
main() ->
    Path = os:getenv("FILE_IO_PROBE_PATH"),
    Dir = filename:dirname(Path),
    First = file:write_file(Path, <<"ab\ncd\n">>),
    Read1 = file:read_file(Path),
    {ok, Fd} = file:open(Path, [write]),
    Write = file:write(Fd, <<"first\nsecond\n">>),
    Position = file:position(Fd, 2),
    Iodata = file:write(Fd, ["X", <<"Y">>, [$Z]]),
    End = file:position(Fd, eof),
    Negative = file:position(Fd, {bof, -1}),
    Close = file:close(Fd),
    Again = file:close(Fd),
    Read2 = file:read_file(Path),
    {ok, Append} = file:open(Path, [write, append]),
    Appended = file:write(Append, <<"tail\n">>),
    ok = file:close(Append),
    Read3 = file:read_file(Path),
    Exclusive = file:open(Path, [exclusive]),
    IsDir = file:open(Dir, [write]),
    NotDir = file:open(filename:join(Path, "x"), [write]),
    Missing = file:write_file(filename:join([Dir, "missing", "x"]), <<>>),
    io:format("~p~n", [[First, Read1, Write, Position, Iodata, End, Negative,
                        Close, Again, Read2, Appended, Read3, Exclusive, IsDir,
                        NotDir, Missing]]).
