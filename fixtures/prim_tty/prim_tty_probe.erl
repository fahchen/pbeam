-module(prim_tty_probe).
-export([main/0]).

main() ->
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    [writer_exit(make_ref(), {write, self(), [<<"Hello, world!\n">>]}),
     writer_exit(make_ref(), {write, [<<"Hello, world!\n">>]}),
     writer_exit(stdout, {write, self(), not_iodata}),
     writer_exit(Re, {write, self(), [<<"Hello, world!\n">>]})].

writer_exit(Tty, Message) ->
    Writer = spawn(prim_tty, writer_loop, [Tty, make_ref()]),
    Monitor = monitor(process, Writer),
    Writer ! Message,
    receive {'DOWN', Monitor, process, Writer, Reason} -> Reason end.
