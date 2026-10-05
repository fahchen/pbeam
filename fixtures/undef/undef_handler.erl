-module(undef_handler).
-export(['$handle_undefined_function'/2]).

%% error_handler:call_undefined_function_handler/3 calls this function for
%% each function that this module does not export (error_handler.erl:206-213).
%% A call of stop exits with the handler arguments, so that a process that
%% starts at undef_handler:stop shows them in its exit reason.
'$handle_undefined_function'(stop, Arguments) -> exit({handled, stop, Arguments});
'$handle_undefined_function'(Function, Arguments) -> {handled, Function, Arguments}.
