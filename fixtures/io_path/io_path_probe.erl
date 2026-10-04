-module(io_path_probe).
-export([main/0, writer/0, start_driver/2, start_group/2]).

main() ->
    Old = group_leader(),
    Driver = spawn(?MODULE, start_driver, [self(), make_ref()]),
    Group = spawn(?MODULE, start_group, [self(), Driver]),
    DriverMon = monitor(process, Driver),
    GroupMon = monitor(process, Group),
    Driver ! {group, Group},
    group_leader(Group, self()),
    self() ! noise,
    Result = try io:put_chars(standard_io, [<<"Hello, world!">>, 10]) of
                 Value -> {ok, Value}
             catch
                 error:Reason -> {error, Reason}
             end,
    group_leader(Old, self()),
    Kept = receive noise -> noise_ok end,
    DriverExit = receive {'DOWN', DriverMon, process, Driver, DR} -> DR end,
    GroupExit = receive {'DOWN', GroupMon, process, Group, GR} -> GR end,
    {Result, Kept, DriverExit, GroupExit}.

start_driver(Parent, WriterRef) ->
    put('$ancestors', [self(), Parent]),
    process_flag(trap_exit, true),
    Writer = spawn(prim_tty, writer_loop, [make_ref(), WriterRef]),
    Group = receive {group, G} -> G end,
    put(current_group, Group),
    Tty = {state, make_ref(), undefined, {Writer, WriterRef}, #{input => disabled, output => raw, ofd => stdout},
           true, true, [], [], [], [], undefined, 1, 0, <<>>, 80, 24, false, undefined},
    State = {state, Tty, WriterRef, undefined, disabled, false, undefined, Group, Group,
             {gr, 1, 0, Group, [{group, 0, Group, {}}]}, {false, queue:new()}},
    gen_statem:enter_loop(user_drv, [], server, State).

start_group(Parent, Driver) ->
    true = register(pbeam_probe_user, self()),
    put('$ancestors', [self(), Parent]),
    process_flag(trap_exit, true),
    link(Driver),
    State = {state, list, Driver, false, none, true, noshell, disabled, undefined, false, undefined, undefined, {[], []}, [], undefined},
    gen_statem:enter_loop(group, [], server, State).

writer() ->
    {Writer, Mon} = spawn_monitor(prim_tty, writer_loop, [make_ref(), make_ref()]),
    Writer ! {write, self(), [<<"Hello, world!\n">>]},
    receive {'DOWN', Mon, process, Writer, Reason} -> Reason end.
