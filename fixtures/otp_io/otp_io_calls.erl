-module(otp_io_calls).
-export([io_call/0, group_call/0, user_drv_call/0, prim_tty_call/0]).

io_call() -> io:columns().

group_call() -> group:whereis_shell().

user_drv_call() -> user_drv:whereis_group().

prim_tty_call() -> prim_tty:ansi_regexp().
