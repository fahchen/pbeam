-module(tty_nif_probe).
-export([isatty/0, create/0, window_size/0, setlocale/0, setupterm/0, write/0]).

isatty() ->
    [catch_error(fun() -> prim_tty:isatty(A) end)
     || A <- [stdin, stdout, stderr, foo, 1, make_ref()]].

create() ->
    {ok, _} = prim_tty:tty_create(stdout),
    {ok, _} = prim_tty:tty_create(stderr),
    [catch_error(fun() -> prim_tty:tty_create(A) end) || A <- [stdin, foo, 1, make_ref()]].

window_size() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    {ok, Err} = prim_tty:tty_create(stderr),
    {ok, {re_pattern, _, _, _, Re}} = re:compile("a", []),
    [catch_error(fun() -> prim_tty:tty_window_size(A) end)
     || A <- [Out, Err, foo, make_ref(), Re]].

setlocale() ->
    [prim_tty:setlocale(A) || A <- [foo, make_ref()]].

setupterm() ->
    [prim_tty:setupterm_nif(), prim_tty:setupterm_nif()].

write() ->
    {ok, Out} = prim_tty:tty_create(stdout),
    prim_tty:write_nif(Out, []).

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
