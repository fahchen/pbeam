-module(map_share_probe).
-export([main/0, grow/0, errors/0, id/1]).

%% Each copy of a map keeps its pairs when the code changes another copy:
%% a copy in a register, in a stack slot, in a fun, and in a message.
main() ->
    Map = ?MODULE:id(#{1 => a, 2 => b}),
    Copy = ?MODULE:id(Map),
    Exact = Copy#{1 := c},
    Assoc = Map#{3 => d},
    Put = maps:put(2, e, Map),
    Merged = maps:merge(Map, #{2 => f, 4 => g}),
    Removed = maps:remove(1, Map),
    Taken = maps:take(2, Map),
    Fun = fun() -> Map end,
    FromFun = (Fun())#{1 := h},
    self() ! Map,
    Received = receive Message -> Message#{2 := i} end,
    [Map, Copy, Exact, Assoc, Put, Merged, Removed, Taken, FromFun, Fun(), Received].

%% A copy of a flatmap of 32 keys gets its 33rd key and becomes a
%% hashmap. The other copy stays a flatmap. An update of the hashmap does
%% not change the other copy of the hashmap.
grow() ->
    Small = ?MODULE:id(#{1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7, 8 => 8, 9 => 9, 10 => 10, 11 => 11, 12 => 12, 13 => 13, 14 => 14, 15 => 15, 16 => 16, 17 => 17, 18 => 18, 19 => 19, 20 => 20, 21 => 21, 22 => 22, 23 => 23, 24 => 24, 25 => 25, 26 => 26, 27 => 27, 28 => 28, 29 => 29, 30 => 30, 31 => 31, 32 => 32}),
    Big = Small#{33 => 33},
    Exact = Big#{1 := one},
    Assoc = Big#{1 => uno},
    [maps:keys(Small), maps:keys(Big), map_size(Small), map_size(Big),
     map_get(1, Small), map_get(1, Big), map_get(1, Exact), map_get(1, Assoc),
     maps:keys(Exact) =:= maps:keys(Big)].

%% An update of a key that the map does not hold raises badkey. The map
%% does not change.
errors() ->
    Map = ?MODULE:id(#{1 => a}),
    Big = ?MODULE:id(maps:from_list([{K, K} || K <- [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33]])),
    Flat = try Map#{2 := b} catch Class1:Reason1 -> {Class1, Reason1} end,
    Hash = try Big#{34 := 2} catch Class2:Reason2 -> {Class2, Reason2} end,
    [Flat, Hash, Map, map_size(Big), map_get(1, Big)].

id(Term) -> Term.
