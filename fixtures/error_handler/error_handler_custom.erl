-module(error_handler_custom).
-export([undefined_function/3]).

%% The error handler of a process, which `erlang:process_flag(error_handler,
%% error_handler_custom)` sets (`erts_proc_set_error_handler`,
%% `erl_process.h:2453-2465`). The export entry of a function that a module
%% does not export calls `undefined_function/3` of the handler of the process
%% (`call_error_handler`, `beam_common.c:1294-1342`). This handler continues
%% the call that the default handler makes
%% (`error_handler.erl:85-98`): it runs the function of a module that is
%% loaded, and else raises `undef`.
undefined_function(Module, Function, Arguments) ->
    Arity = length(Arguments),
    case erlang:function_exported(Module, Function, Arity) of
        true -> erlang:apply(Module, Function, Arguments);
        false -> erlang:raise(error, undef, [{Module, Function, Arguments, []}])
    end.
