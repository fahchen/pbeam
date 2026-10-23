-module(heap_flags_probe).
-export([process_flags/0, process_errors/0, system_flags/0, heap_items/0, queue_costs/0, huge/1]).

%% The heap flags of process_flag/2 (bif.c:2101-2166): fullsweep_after
%% keeps a Uint16, min_heap_size and min_bin_vheap_size keep the node
%% default or the next heap size (erl_gc.c:310-338), and max_heap_size
%% takes the node flags for each flag that a map does not give
%% (erl_gc.c:3824-3873). process_info/2 and the garbage_collection item of
%% process_info/1 report the values (erl_bif_info.c:1990-2094).
process_flags() ->
    in_new(fun() ->
        Set = [process_flag(min_heap_size, 0),
               process_flag(min_heap_size, 300),
               process_flag(min_heap_size, 1),
               process_flag(min_bin_vheap_size, 5),
               process_flag(min_bin_vheap_size, 50000),
               process_flag(min_bin_vheap_size, 0),
               process_flag(fullsweep_after, 100000),
               process_flag(fullsweep_after, 3),
               map(process_flag(max_heap_size, #{size => 1000, kill => false})),
               map(process_flag(max_heap_size,
                                #{size => 2000, error_logger => false,
                                  include_shared_binaries => true})),
               map(process_flag(max_heap_size, 0)),
               process_flag(min_heap_size, 5000)],
        {Set, info(self()), gc(self())}
    end).

%% A value that is not valid raises badarg. The size of max_heap_size is 0
%% or at least the minimum heap size of the process (bif.c:2155-2156).
process_errors() ->
    in_new(fun() ->
        A = [badarg(fun() -> process_flag(max_heap_size, 100) end),
             badarg(fun() -> process_flag(min_heap_size, id(1 bsl 60)) end),
             badarg(fun() -> process_flag(fullsweep_after, id(-1)) end),
             badarg(fun() -> process_flag(max_heap_size, #{size => 500, kill => x}) end),
             badarg(fun() -> process_flag(max_heap_size, #{kill => true}) end),
             process_flag(min_heap_size, 5000)],
        A ++ [badarg(fun() -> process_flag(max_heap_size, 4000) end),
              map(process_flag(max_heap_size, #{size => 6772, kill => false, error_logger => false}))]
    end).

%% system_flag/2 sets the node defaults that a new process takes
%% (bif.c:5037-5105): fullsweep_after keeps at most 65535, and
%% min_heap_size and min_bin_vheap_size keep the next heap size, also for
%% 0, as an int that wraps to 32 bits (erl_init.c:110-111). The probe restores the start values at the end.
system_flags() ->
    Set = [erlang:system_flag(min_heap_size, 0),
           erlang:system_flag(min_heap_size, 300),
           erlang:system_flag(min_bin_vheap_size, 0),
           erlang:system_flag(fullsweep_after, 100000),
           erlang:system_flag(fullsweep_after, 7),
           map(erlang:system_flag(max_heap_size, #{size => 500, kill => false}))],
    Child = in_new(fun() ->
        Info = info(self()),
        {Info,
         process_flag(min_heap_size, 100),
         map(process_flag(max_heap_size, #{size => 1000}))}
    end),
    Error = badarg(fun() -> erlang:system_flag(max_heap_size, 300) end),
    Wrap = erlang:system_flag(min_bin_vheap_size, 1 bsl 33),
    WrapChild = in_new(fun() -> info(self()) end),
    Restore = [erlang:system_flag(min_heap_size, 233),
               erlang:system_flag(min_bin_vheap_size, 46422),
               erlang:system_flag(fullsweep_after, 65535),
               map(erlang:system_flag(max_heap_size, #{size => 0, kill => true}))],
    {Set, Child, Error, Wrap, WrapChild, Restore}.

%% The heap items of process_info/2 for another process and for the
%% caller (erl_bif_info.c:1982-2094). The sizes depend on the heap, so the
%% probe checks their form: heap_size and total_heap_size are positive
%% words, and memory holds more bytes than the words of the total heap.
%% garbage_collection gives the flags of the target with minor_gcs 0.
heap_items() ->
    Self = self(),
    Pid = spawn(fun() ->
        process_flag(fullsweep_after, 3),
        process_flag(min_heap_size, 300),
        Self ! ready,
        receive stop -> ok end
    end),
    receive ready -> ok end,
    Other = sizes(Pid),
    Gc = process_info(Pid, garbage_collection),
    Pid ! stop,
    {Other, sizes(self()), map_gc(Gc)}.

sizes(Pid) ->
    {heap_size, Heap} = process_info(Pid, heap_size),
    {total_heap_size, Total} = process_info(Pid, total_heap_size),
    {memory, Memory} = process_info(Pid, memory),
    [is_integer(Heap) andalso Heap > 0, is_integer(Total) andalso Total >= Heap,
     is_integer(Memory) andalso Memory > 8 * Total].

map_gc({garbage_collection, Gc}) ->
    {garbage_collection, [{Key, map(Value)} || {Key, Value} <- Gc]}.

%% The extra reductions of a read of the caller with 400 queued messages
%% against an empty queue: floor(400 / 4) for memory
%% (erl_bif_info.c:2066-2067) and for total_heap_size with the queue on
%% the heap (erl_bif_info.c:2032-2042), 0 for heap_size and for
%% total_heap_size with the queue off the heap. A read of another process
%% sends a request for memory and total_heap_size
%% (ERTS_PI_FLAG_FORCE_SIG_SEND, erl_bif_info.c:828,839,1381), and ERTS
%% discards the cost of the request, so the last element reads another
%% process with 400 messages and gives the cost of an empty one, and a
%% request costs the caller more than the direct read of heap_size.
queue_costs() ->
    Self = [in_new(fun() -> extra(Item, Data) end)
            || {Item, Data} <- [{memory, on_heap}, {total_heap_size, on_heap},
                                {heap_size, on_heap}, {total_heap_size, off_heap}]],
    Pid = spawn(fun() -> receive stop -> ok end end),
    Empty = cost(Pid, memory),
    send(Pid, 400),
    Other = cost(Pid, memory) - Empty,
    Direct = cost(Pid, heap_size),
    Pid ! stop,
    Self ++ [Other, Empty - Direct > 0].

extra(Item, Data) ->
    process_flag(message_queue_data, Data),
    Empty = cost(self(), Item),
    send(self(), 400),
    cost(self(), Item) - Empty.

send(_Pid, 0) -> ok;
send(Pid, N) -> Pid ! N, send(Pid, N - 1).

cost(Pid, Item) ->
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    _ = process_info(Pid, Item),
    {reductions, After} = process_info(self(), reductions),
    After - Before.

%% A heap size at or above the largest heap size of ERTS. ERTS stops the
%% emulator in erts_next_heap_size (erl_gc.c:335).
huge(process) -> process_flag(min_heap_size, id(1 bsl 55));
huge(system) -> erlang:system_flag(min_bin_vheap_size, id(1 bsl 55)).

info(Pid) ->
    [map(element(2, process_info(Pid, Item)))
     || Item <- [min_heap_size, min_bin_vheap_size, fullsweep_after, max_heap_size]].

%% The garbage_collection item without minor_gcs, which counts the
%% collections of ERTS.
gc(Pid) ->
    {garbage_collection, Gc} = keyfind(garbage_collection, process_info(Pid)),
    [{Key, map(Value)} || {Key, Value} <- Gc, Key =/= minor_gcs].

keyfind(Key, [{Key, _} = Pair | _]) -> Pair;
keyfind(Key, [_ | Rest]) -> keyfind(Key, Rest).

map(#{error_logger := Log, include_shared_binaries := Bins, kill := Kill, size := Size}) ->
    [{error_logger, Log}, {include_shared_binaries, Bins}, {kill, Kill}, {size, Size}];
map(Value) -> Value.

in_new(Fun) ->
    Self = self(),
    Pid = spawn(fun() -> Self ! {self(), Fun()} end),
    receive {Pid, Result} -> Result end.

badarg(Fun) ->
    try Fun() catch error:badarg -> badarg end.

id(X) -> X.
