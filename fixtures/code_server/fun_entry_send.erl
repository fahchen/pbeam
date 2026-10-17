-module(fun_entry_send).
-export([value/0]).
-on_load(init/0).

%% The on_load function sends two funs of the code of this module to the
%% process registered as fun_entry_probe, and fails, so code_server removes
%% the code (code_server.erl:1360-1368). A load with no such process
%% returns ok, so the code becomes current. Each fun holds the module name,
%% the index of its lambda in the FunT chunk, and the old unique value of
%% its entry.
init() ->
    case whereis(fun_entry_probe) of
        undefined ->
            ok;
        Probe ->
            First = make_first(version_one),
            Second = make_second(version_one),
            %% The name of a fun of code that waits for its -on_load function
            %% is `[]` (`erl_fun.c:37-46`, `erl_bif_info.c:3998-4001`).
            Probe ! {stale_fun, first, First, erlang:fun_info(First, name)},
            Probe ! {stale_fun, second, Second, erlang:fun_info(Second, name)},
            {error, sent}
    end.

%% Each function returns a fun with one free variable, so make_fun3 makes
%% the fun in the code and the loader holds no literal for it
%% (jit/asm_load.c:1224-1232).
make_first(Value) ->
    fun() -> {Value, first} end.

make_second(Value) ->
    fun() -> {Value, second} end.

value() ->
    version_one.
