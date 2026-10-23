-module(spawn_opt_probe).
-export([flags/0, links/0, errors/0, unsupported/1, report/1]).

%% The start flags that each option of erlang:spawn_opt/4 gives
%% (erts_parse_spawn_opts, erl_process.c:12222-12434). The heap flags of the
%% options apply only with an option of SPO_USE_ARGS (erl_process.h:1515-1520,
%% erl_process.c:12561-12573), so min_bin_vheap_size alone changes nothing.
flags() ->
    [start(Options) || Options <-
        [[],
         [{min_heap_size, 1000}],
         [{min_heap_size, 10}],
         [{min_bin_vheap_size, 100000}],
         [{min_bin_vheap_size, 100000}, {fullsweep_after, 5}],
         [{min_bin_vheap_size, 1}, {priority, high}],
         [{min_bin_vheap_size, 100000}, {scheduler, 0}],
         [{fullsweep_after, 70000}],
         [{fullsweep_after, 0}, {fullsweep_after, 9}],
         [{max_heap_size, #{size => 5000, kill => false}}],
         [{max_heap_size, 0}, {min_bin_vheap_size, 200000}],
         [{async_dist, true}],
         [{async_dist, true}, {async_dist, false}],
         [{scheduler, 1}, {scheduler, 0}],
         [{message_queue_data, off_heap}, {priority, low}]]].

%% link, {link, []}, monitor, and {monitor, []} link or monitor the new
%% process (erl_process.c:12256-12265, 12396-12417). A later monitor or
%% {link, []} replaces the options of an earlier one.
links() ->
    [begin
         Result = erlang:spawn_opt(?MODULE, report, [none], Options),
         Pid = case Result of {P, _} -> P; P -> P end,
         {links, Links} = process_info(self(), links),
         unlink(Pid),
         exit(Pid, kill),
         Tag = case Result of
                   {_, Ref} -> receive {T, Ref, process, Pid, killed} -> tag(T) end;
                   _ -> none
               end,
         {lists:member(Pid, Links), Tag}
     end || Options <- [[link], [{link, []}], [monitor], [{monitor, []}],
                        [{monitor, [{tag, t}]}, link],
                        [{monitor, [{alias, demonitor}]}, monitor],
                        [{monitor, [priority]}, {monitor, []}],
                        [{link, [priority]}, {link, []}]]].

%% Each option that is not valid raises badarg (bif.c:1039-1051), also
%% after an option that PBeam does not support.
errors() ->
    Over = erlang:system_info(schedulers) + 1,
    [try erlang:spawn_opt(?MODULE, report, [none], Options) of
         _ -> ok
     catch
         error:Reason -> Reason
     end || Options <-
        [[{min_heap_size, -1}], [{min_heap_size, a}], [{min_heap_size, 1 bsl 62}],
         [{min_bin_vheap_size, -1}], [{fullsweep_after, -1}],
         [{fullsweep_after, 1 bsl 62}], [{max_heap_size, 100}],
         [{max_heap_size, -1}], [{max_heap_size, 100}, {min_heap_size, 100}],
         [{max_heap_size, 1000}, {min_heap_size, 2000}], [{async_dist, x}],
         [{priority, x}], [{message_queue_data, x}], [{scheduler, -1}],
         [{scheduler, Over}], [{link, [x]}], [{link, a}], [{monitor, x}],
         [{reply, yes}], [{reply_tag, x}], [sensitive], [{sensitive, true}],
         [{a, b, c}], [foo], [{1, 2}], [link | x], x,
         [{monitor, [{alias, demonitor}]}, foo], [{link, [priority]}, foo]]].

%% An option that PBeam does not support, and a heap size at or above the
%% largest heap size, where ERTS stops the emulator (erl_gc.c:335).
unsupported(monitor) ->
    erlang:spawn_opt(?MODULE, report, [none], [{monitor, [{alias, demonitor}]}]);
unsupported(link) ->
    erlang:spawn_opt(?MODULE, report, [none], [{link, [priority]}, link]);
unsupported(scheduler) ->
    erlang:spawn_opt(?MODULE, report, [none], [{scheduler, 1}]);
unsupported(Option) ->
    erlang:spawn_opt(?MODULE, report, [none], [{Option, 1 bsl 55}]).

%% The tag as an atom that ~w prints without quotes.
tag('DOWN') -> down;
tag(Tag) -> Tag.

start(Options) ->
    Pid = erlang:spawn_opt(?MODULE, report, [self()], Options),
    receive {Pid, Flags} -> Flags end.

report(none) ->
    receive stop -> ok end;
report(Parent) ->
    Items = [min_heap_size, min_bin_vheap_size, fullsweep_after, priority],
    {max_heap_size, Max} = process_info(self(), max_heap_size),
    Async = process_flag(async_dist, false),
    Queue = process_flag(message_queue_data, on_heap),
    Parent ! {self(), [element(2, process_info(self(), I)) || I <- Items]
                      ++ [lists:sort(maps:to_list(Max)), Async, Queue]}.
