-module(error_handler_probe).
-compile(nowarn_deprecated_catch).
-export([caught/0, frames/0, first/0, flag/0, flag_errors/0, reductions/0, id/1, caller/0,
         apply_caller/3]).

-define(REDS, element(2, process_info(self(), reductions))).

%% The class, the reason, and the frames of `error:undef` for a function that
%% the loaded module `error_handler_target` does not export. The export entry
%% of the call runs `error_handler:undefined_function/3`
%% (`beam_common.c:1294-1342`), which raises `undef` with the frame of the
%% failed call and the frames of the callers (`error_handler.erl:165-178`).
caught() ->
    init_server(),
    try caller() catch Class:Reason:Frames -> {Class, Reason, Frames} end.

%% The frames of the failed call of `caller/0` and of `caller/0` itself.
%% ERTS puts the frames of `erl_eval` and `init` after them in a `-eval` run,
%% and PBeam puts the frame of its own caller after them.
frames() ->
    init_server(),
    try caller() catch _Class:_Reason:Frames -> Frames end.

%% The first frame of the stack trace of a failed call.
first() ->
    init_server(),
    try caller() catch Class:Reason:Frames -> {Class, Reason, hd(Frames)} end.

caller() ->
    id(error_handler_target):missing(a).

%% Return the old and the new value of the error handler of the process, and
%% the result of a call that the module of the handler answers
%% (`error_handler.erl:85-98`; `process_flag_aux`, `bif.c:2094-2101`).
flag() ->
    init_server(),
    Old = erlang:process_flag(error_handler, error_handler_custom),
    Result = (catch error_handler_target:missing(b)),
    Back = erlang:process_flag(error_handler, Old),
    {Old, Back, Result}.

%% The custom handler answers the calls of the loaded module
%% `error_handler_target` (`error_handler_custom:undefined_function/3`).
%% `process_flag_aux` takes the atom of the module and gives `badarg` for a
%% value that is not an atom (`bif.c:2094-2101`).
flag_errors() ->
    [try erlang:process_flag(error_handler, Value) catch Class:Reason -> {Class, Reason} end
     || Value <- [not_an_atom_1, 1, {a}, []]].

%% The reductions of a failed call in a new process, without the reductions
%% of the measure function. `process_info(self(), reductions)` is a guard BIF
%% and costs no reduction, and the process runs the call after
%% `erlang:yield/0` (`preempt_probe:measure/3`).
reductions() ->
    init_server(),
    [{Name, delta(Argument)} || {Name, Argument} <- [{plain, plain}, {handled, handled}]].

%% Start the init process of `fixtures/undef/undef_init` when the run has no
%% boot, as `undef_probe:init_server/0` does.
init_server() ->
    case erlang:function_exported(undef_init, start, 0) of
        true -> undef_init:start();
        false -> ok
    end.

%% `plain` calls an undefined function of a loaded module, and the default
%% error handler raises `undef` with the frame of the call
%% (`error_handler.erl:165-178`). `handled` sets the error handler of the
%% process and calls the module that exports
%% `'$handle_undefined_function'/2` (`error_handler.erl:206-213`).
delta(plain) ->
    measured(fun() -> _ = (catch error_handler_target:missing(c)) end);
delta(handled) ->
    measured(fun() ->
        Old = erlang:process_flag(error_handler, error_handler_custom),
        _ = (catch error_handler_target:missing(d)),
        erlang:process_flag(error_handler, Old)
    end).

%% Run `Fun` in a new process after a switch out and return its reductions.
%% The process costs the entry of `measured/1` and the `return` of `Fun`,
%% which the same run of the `plain` case has.
measured(Fun) ->
    Parent = self(),
    Pid = spawn(fun() ->
        erlang:yield(),
        Before = ?REDS,
        _ = Fun(),
        Parent ! {self(), ?REDS - Before}
    end),
    receive {Pid, Delta} -> Delta end.

id(X) -> X.

apply_caller(Module, Function, Arguments) ->
    id(x),
    erlang:apply(Module, Function, Arguments).
