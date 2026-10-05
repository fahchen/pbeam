-module(logger_boot_probe).
-export([main/0]).

%% Read the logger state that the boot makes: the handler ids, the primary
%% level, the id, module, and level of the default handler, which logger
%% process names are registered, and the kernel environment logger_level.
main() ->
    {ok, Default} = logger:get_handler_config(default),
    #{level := Primary} = logger:get_primary_config(),
    Names = [application_controller, logger, logger_sup, logger_proxy,
             logger_handler_watcher, logger_std_h_default, logger_simple_h],
    {logger:get_handler_ids(), Primary, maps:with([id, module, level], Default),
     [{Name, whereis(Name) =/= undefined} || Name <- Names],
     application:get_env(kernel, logger_level)}.
