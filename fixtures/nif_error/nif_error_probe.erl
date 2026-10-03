-module(nif_error_probe).
-export([main/0, stub/0]).

main() -> stub().

stub() -> erlang:nif_error(undef).
