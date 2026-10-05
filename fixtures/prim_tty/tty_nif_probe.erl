-module(tty_nif_probe).
-export([isatty/0]).

isatty() ->
    [catch_error(fun() -> prim_tty:isatty(A) end)
     || A <- [stdin, stdout, stderr, foo, 1, make_ref()]].

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
