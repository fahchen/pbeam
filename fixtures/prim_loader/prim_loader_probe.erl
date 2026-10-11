-module(prim_loader_probe).
-export([main/0]).

%% Write the erl_prim_loader results of a code path from the boot flags, one
%% term on each line: the path of erl_prim_loader:get_path/0, then the
%% results of erl_prim_loader:get_file/1, erl_prim_loader:read_file/1, and
%% init:get_argument/1 (erl_prim_loader.erl:240-299; init.erl:392-400). The
%% run gives the flags -pa d1 and -pa d2 in this order. Its current directory
%% has d1/a.txt ("one"), d1/c.txt ("c"), and d2/a.txt ("two").
main() ->
    io:format("~p~n", [erl_prim_loader:get_path()]),
    Results = [erl_prim_loader:get_file("a.txt"),
               erl_prim_loader:get_file('c.txt'),
               erl_prim_loader:get_file("missing.txt"),
               erl_prim_loader:get_file("d1/a.txt"),
               erl_prim_loader:read_file("d1/a.txt"),
               erl_prim_loader:read_file("a.txt"),
               erl_prim_loader:read_file("missing.txt"),
               init:get_argument(loader),
               init:get_argument(pa)],
    io:format("~p~n", [bytes(Results)]).

%% Write each binary as {binary, Bytes}: io_lib_pretty prints a binary with
%% erlang:bit_size/1, which PBeam does not run yet (row "Unimplemented BIFs"
%% of docs/erts-differences.md).
bytes(Binary) when is_binary(Binary) -> {binary, [Byte || <<Byte>> <= Binary]};
bytes(Tuple) when is_tuple(Tuple) -> list_to_tuple(bytes(tuple_to_list(Tuple)));
bytes([Head | Tail]) -> [bytes(Head) | bytes(Tail)];
bytes(Term) -> Term.
