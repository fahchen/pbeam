%% The NIF manifest gate reports get_frame_header_nif/1, which is in the zstd library and has no binding. A call of it gives the runtime error of the library.
-module(zstd).
-export([load/0, get_frame_header_nif/1]).
-nifs([get_frame_header_nif/1]).

load() -> erlang:load_nif("zstd", 0).

get_frame_header_nif(_Frame) -> erlang:nif_error(undef).
