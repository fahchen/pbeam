-module(tty_resources).
-export([tty_create/0, tty_use/0]).

%% Two roots on two schedulers share one TTY resource of `prim_tty`, as the
%% node of ERTS keeps one resource object (prim_tty_nif.c:988-1072). The
%% root `tty_create` makes the resource and gives it to the root `tty_use`,
%% which calls `tty_window_size/1` and `tty_select/2` with it. The test
%% loads the stand-in `prim_tty` of `fixtures/prim_tty/nif`, which binds the
%% NIFs without `prim_tty:init/1`.

tty_create() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    Pid = wait_for(tty_user, 100000),
    Pid ! {tty, self(), TTY},
    receive done -> ok after 60000 -> timeout end.

tty_use() ->
    true = register(tty_user, self()),
    receive
        {tty, From, TTY} ->
            %% fd 1 is a terminal or not, after the host of the test.
            Size =
                case prim_tty:tty_window_size(TTY) of
                    {ok, {Width, Height}} when is_integer(Width), is_integer(Height) -> ok;
                    {error, enotsup} -> ok;
                    Other -> Other
                end,
            Select = prim_tty:tty_select(TTY, make_ref()),
            From ! done,
            {Size, Select}
    after 60000 -> timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid -> Pid
    end.
