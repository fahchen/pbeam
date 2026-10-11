-module(error_handler_load_target).
-export([value/1]).

%% The module of the code path that the run does not load. A call of it goes
%% through the error handler of the process.
value(Argument) ->
    {loaded, Argument}.
