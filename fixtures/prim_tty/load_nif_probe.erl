-module(load_nif_probe).
-export([main/0, twice/0, bad_lib/0, other/0, other_badarg/0]).

main() ->
    [[catch_error(fun() -> prim_tty:load(Path) end)
      || Path <- [foo, [0], "a\0b", [id(-1)], [id(16#110000)], [id(16#D800)],
                  [<<"a">>]]],
     prim_tty:load_body("prim_tty"),
     [catch_error(fun() -> prim_tty:load(Path) end)
      || Path <- ["prim_tty", <<"x">>, <<"a", 0, "b">>, [], [[$a], [], $b],
                  [id(255)], [id(256)], [id(16#10FFFF)], foo, [0]]],
     prim_tty:load_last("prim_tty"),
     catch_error(fun() -> prim_tty:load_body(foo) end),
     catch_error(fun() -> prim_tty:load_last(foo) end)].

twice() ->
    [prim_tty:load("prim_tty"), prim_tty:load("prim_tty")].

bad_lib() ->
    [apply(id(erlang), load_nif, ["prim_tty", #{}]),
     (id(fun erlang:load_nif/2))("prim_tty", #{}),
     apply(id(erlang), load_nif, [foo, #{}])].

other() ->
    erlang:load_nif("load_nif_probe", 0).

other_badarg() ->
    catch_error(fun() -> erlang:load_nif(foo, 0) end).

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.

id(X) -> X.
