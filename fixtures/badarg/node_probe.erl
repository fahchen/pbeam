-module(node_probe).
-export([main/0, guarded/1, id/1]).

%% ERTS `node_1` raises `badarg` for a term that is not a pid, a port, or a
%% reference, and gives the node of the others (`dist.c:6219-6225`).
main() ->
    Bad = try node(?MODULE:id(foo)) catch error:R:S -> {R, hd(S)} end,
    [Bad, node(?MODULE:id(make_ref())), node(?MODULE:id(self())),
     guarded(foo), guarded(self())].

%% In a guard, the `badarg` fails the guard.
guarded(T) when node(T) =:= nonode@nohost -> local;
guarded(_) -> other.

id(T) -> T.
