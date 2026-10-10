-module(io_device_probe).
-export([main/0, raw/0]).

%% Read and write standard input and output with file:read/2 and
%% file:write/2, as IO.binread/2 and IO.binwrite/2 of Elixir call them
%% (Elixir 1.20.4 lib/elixir/lib/io.ex). Each call goes to the group leader
%% as an I/O request: through the atom standard_io or through the pid of
%% the group leader. Then write each result.
%%
%% The reads use list mode, then binary mode, then binary mode with the
%% latin1 encoding, as Terra sets it (lib/terra/terminal/owner.ex:184), and
%% then the unicode encoding again. With latin1, a write converts each
%% byte to one character. The error cases give each failure of file.erl
%% and io.erl: a size or a device that is not valid, a name that is not
%% registered, the caller as the device, and a device that is not alive.
main() ->
    W1 = file:write(standard_io, <<"atom device\n">>),
    W2 = file:write(group_leader(), <<"pid device\n">>),
    R1 = file:read(standard_io, 3),
    R2 = file:read(group_leader(), 2),
    ok = io:setopts(standard_io, [binary]),
    R3 = file:read(standard_io, 2),
    ok = io:setopts(standard_io, [{encoding, latin1}]),
    R4 = file:read(group_leader(), 3),
    W3 = file:write(standard_io, <<233, $\n>>),
    ok = io:setopts(standard_io, [{encoding, unicode}]),
    R5 = file:read(standard_io, 2),
    W4 = file:write(group_leader(), <<195, 169, $\n>>),
    Dead = spawn(fun() -> ok end),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, _, _, _} -> ok end,
    Errors = [file:read(standard_io, -1), file:read(standard_io, x),
              file:read(no_device, 1), file:write(no_device, <<"x">>),
              file:read(self(), 1), file:write(self(), <<"x">>),
              file:read(Dead, 1), file:write(Dead, <<"x">>),
              file:read(1, 1), file:write(1, <<>>)],
    R6 = file:read(standard_io, 100),
    R7 = file:read(standard_io, 1),
    io:put_chars([show([W1, W2, R1, R2, R3, R4, W3, R5, W4, Errors, R6, R7]), "\n"]).

%% Read keys from a terminal in raw mode as Terra does: no shell, the raw
%% mode of shell:start_interactive/1, binary mode, the latin1 encoding, and
%% one byte for each read (lib/terra/terminal/tty.ex, lib/terra/input/tty.ex).
%% Write "ready" when the terminal is in raw mode, then the code of each
%% byte until the key q.
raw() ->
    ok = shell:start_interactive({noshell, raw}),
    ok = io:setopts(standard_io, [binary, {encoding, latin1}]),
    ok = file:write(standard_io, <<"ready\r\n">>),
    keys().

keys() ->
    case file:read(standard_io, 1) of
        {ok, <<"q">>} ->
            ok;
        {ok, Byte} ->
            ok = file:write(standard_io, list_to_binary(["key ", show(Byte), "\r\n"])),
            keys()
    end.

%% Write a result as io:format("~w") does, for the atoms, integers, lists,
%% binaries, and {Tag, Value} tuples of the results. io_lib:write/1 needs
%% erl_scan for atoms, which the boot does not load.
show(Atom) when is_atom(Atom) ->
    atom_to_list(Atom);
show(Integer) when is_integer(Integer) ->
    integer_to_list(Integer);
show(List) when is_list(List) ->
    ["[", join([show(Term) || Term <- List]), "]"];
show(Binary) when is_binary(Binary) ->
    ["<<", join([integer_to_list(Byte) || Byte <- unicode:characters_to_list(Binary, latin1)]),
     ">>"];
show({Tag, Value}) ->
    ["{", show(Tag), ",", show(Value), "}"].

join([]) -> [];
join([First | Rest]) -> [First | [[",", Text] || Text <- Rest]].
