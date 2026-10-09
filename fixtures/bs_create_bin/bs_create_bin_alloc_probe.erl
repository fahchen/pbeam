-module(bs_create_bin_alloc_probe).
-export([wrap/1, run/1]).

%% The fun after the binary makes the compiler give bs_create_bin the
%% allocation list {alloc,[{words,1},{floats,0},{funs,1}]} as Alloc, as
%% Elixir.Code.Formatter:binary_op_to_algebra/8 has.
wrap(X) ->
    B = <<"a", X/binary>>,
    fun() -> B end.

%% Build the fun and call it.
run(X) -> (wrap(X))().
