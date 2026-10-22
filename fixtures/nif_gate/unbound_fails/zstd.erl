%% The NIF manifest gate reports init_compress_nif/0, which is in the zstd library and has no binding. A call of it gives the runtime error of the library.
-module(zstd).
-export([load/0, init_compress_nif/0]).
-nifs([init_compress_nif/0]).

load() -> erlang:load_nif("zstd", 0).

init_compress_nif() -> erlang:nif_error(undef).
