-module(undef_handler).
-export(['$handle_undefined_function'/2]).

%% error_handler:call_undefined_function_handler/3 calls this function for
%% each function that this module does not export (error_handler.erl:206-213).
'$handle_undefined_function'(Function, Arguments) -> {handled, Function, Arguments}.
