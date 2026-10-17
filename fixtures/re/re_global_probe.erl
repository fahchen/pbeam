-module(re_global_probe).
-export([main/0, scheduling/0, uncaught/0, undefined_function/3]).

%% The `re` module is not loaded at the first call. The `global` option and
%% a unicode subject or pattern that needs the conversion trap `re:run/3`
%% to `re:grun/3` or `re:urun/3` (erl_bif_re.c:1530-1534, 1557-1605). The
%% trap target has no loaded export, so the export stub calls
%% `error_handler:undefined_function/3`, which loads `re` and enters the
%% pinned Erlang function (export.c:228-242; beam_common.c:1294-1342;
%% error_handler.erl:85-98). The first call loads the module. Its result is
%% compared. The reductions are those of the runs after the load, so that
%% the comparison does not mix the module load with the trap and the
%% `re.erl` helpers.
main() ->
    Loaded = re:run(<<"abc">>, <<"b">>, [global]),
    io:format("~w~n", [[Loaded, globals(), unicode_globals(), exceptions(),
                        reductions()]]),
    halt().

%% `re:grun/3` runs `re:internal_run/4` for each match, also for a zero
%% length match, a translated capture, and an offset past the subject
%% (re.erl:1817-1889).
globals() ->
    [re:run(<<"abcb">>, <<"b">>, [global]),
     re:run(<<"abcb">>, <<"b">>, [global, {capture, all, binary}]),
     re:run(<<"aa">>, <<"a*">>, [global]),
     re:run(<<"abc">>, <<"b">>, [global, {offset, 10}])].

%% A unicode subject and pattern are not binaries, and a list capture is
%% not `global`, so the call traps to `re:urun/3` (erl_bif_re.c:1530-1534,
%% 1592-1605; re.erl:1754-1790).
unicode_globals() ->
    re:run([$a, 233, $b], [233], [unicode, global, {capture, all, list}]).

%% A bad argument raises in the trap target, and the catch of `grun/3`
%% raises it again with the stack trace of `re:run/3` (re.erl:1796-1815).
exceptions() ->
    [catch_error(fun() -> re:run(foo, <<"a">>, [global]) end),
     catch_error(fun() -> re:run([-1], <<"a">>, [global]) end)].

%% The reductions of a global run and of a global run with two matches,
%% after the load of `re`.
reductions() ->
    [charge(fun() -> re:run(<<"abc">>, <<"b">>, [global]) end),
     charge(fun() -> re:run(<<"abcabc">>, <<"b">>, [global]) end)].

charge(Fun) ->
    {reductions, Before} = process_info(self(), reductions),
    Result = Fun(),
    {reductions, After} = process_info(self(), reductions),
    {After - Before, Result}.

%% A BIF trap switches out the process before the target runs
%% (`instr_bif.cpp:525-536`, `676-701`). This handler loads no module, so
%% the peer process runs at the trap switch, before the `receive` with
%% `after 0`.
scheduling() ->
    process_flag(error_handler, ?MODULE),
    Self = self(),
    spawn_link(fun() -> Self ! {peer, ran} end),
    Result = re:run(<<"abc">>, <<"b">>, [global]),
    Message = receive {peer, ran} -> ran after 0 -> late end,
    process_flag(error_handler, error_handler),
    io:format("~w~n", [{Result, Message}]),
    halt().

undefined_function(_Module, _Function, _Arguments) ->
    handled.

%% A process error handler with no `undefined_function/3` makes the
%% fallback raise `undef` in `undefined_function` itself, and the process
%% that traps exits with that reason.
uncaught() ->
    {Pid, Ref} = spawn_monitor(fun() ->
        process_flag(error_handler, no_such_error_handler),
        re:run(<<"abc">>, <<"b">>, [global])
    end),
    Reason = receive
        {'DOWN', Ref, process, Pid, Down} -> Down
    after 5000 ->
        timeout
    end,
    case Reason of
        {undef, _} -> io:format("~w~n", [undef]);
        Other -> io:format("~w~n", [Other])
    end,
    halt().

catch_error(Fun) ->
    try Fun() catch error:Reason -> Reason end.
