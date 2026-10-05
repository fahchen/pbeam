-module(host_write_probe).
-export([main/0, init/3]).

%% Write "Hello, world!\n" to stdout through a prim_tty writer, then write
%% the result of that write and a newline to stderr through a second writer.
main() ->
    Out = init(host_write_probe_out, #{input => disabled, output => raw}),
    Err = init(host_write_probe_err, #{input => disabled, output => raw, ofd => stderr}),
    Result = write(Out, "Hello, world!\n"),
    write(Err, Result ++ "\n").

init(Name, Options) ->
    Pid = spawn(?MODULE, init, [self(), Name, Options]),
    receive {Pid, State} -> State end.

%% The writer is linked to the caller of prim_tty:init/1 (prim_tty.erl:268).
%% This process traps exits and sends the exit reason of the writer to its
%% parent.
init(Parent, Name, Options) ->
    process_flag(trap_exit, true),
    true = register(Name, self()),
    Parent ! {self(), prim_tty:init(Options)},
    receive {'EXIT', _Writer, Exit} -> Parent ! {self(), exit, Exit} end.

%% Return "ok" after the reply of the writer. After a failed write, the
%% writer exits with the error reason (prim_tty.erl:559-566), so return
%% "{Reason,Exit}" from the DOWN message of the monitor and the EXIT reason
%% that init/3 sends.
write(State, Chars) ->
    #{write := WriterRef} = prim_tty:handles(State),
    {ok, Monitor} = prim_tty:write(State, Chars, self()),
    receive
        {WriterRef, Reply} ->
            atom_to_list(Reply);
        {'DOWN', Monitor, process, _, Reason} ->
            receive {_, exit, Exit} -> "{" ++ atom_to_list(Reason) ++ "," ++ atom_to_list(Exit) ++ "}" end
    end.
