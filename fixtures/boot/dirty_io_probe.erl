-module(dirty_io_probe).
-export([main/0, writer/1]).

%% A writer process writes 1 MiB to stdout with io:put_chars/1: 1024
%% blocks of 16 lines of 64 bytes. The prim_tty writer calls write_nif/2,
%% which waits while the stdout pipe is full. The main process writes
%% "tick" to stderr after each timer of 10 ms until the writer is done, and
%% then writes "done".
main() ->
    Writer = spawn(?MODULE, writer, [self()]),
    tick(Writer).

tick(Writer) ->
    receive
        {Writer, done} -> io:put_chars(standard_error, "done\n")
    after 10 ->
        io:put_chars(standard_error, "tick\n"),
        tick(Writer)
    end.

writer(Parent) ->
    Block = <<"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n"
              "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n">>,
    io:put_chars(lists:duplicate(1024, Block)),
    Parent ! {self(), done}.
