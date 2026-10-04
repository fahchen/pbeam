-module(demonitor_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, id/1, holder/0, gone/0]).

main() ->
    Holder = spawn(?MODULE, holder, []),
    Ref = monitor(process, Holder),
    Invalid = {reason(catch demonitor(?MODULE:id(foo), [])),
               reason(catch demonitor(Ref, ?MODULE:id([bogus]))),
               reason(catch demonitor(Ref, ?MODULE:id([info, bogus]))),
               reason(catch demonitor(Ref, ?MODULE:id(x))),
               reason(catch demonitor(?MODULE:id(foo), ?MODULE:id(x)))},
    Holder ! {demonitor, self(), Ref},
    Target = receive {target, Result} -> Result end,
    Active = [demonitor(monitor(process, Holder), Options) || Options <- options()],
    First = demonitor(Ref, [info]),
    Removed = [demonitor(Ref, Options) || Options <- options()],
    Unknown = [demonitor(make_ref(), Options) || Options <- options()],
    Holder ! stop,
    Fired = [fired(Options) || Options <- options()],
    {Invalid, Target, Active, First, Removed, Unknown, Fired}.

options() -> [[], [flush], [info], [flush, info], [info, info], [info, flush, flush]].

fired(Options) ->
    {Pid, Ref} = spawn_monitor(?MODULE, gone, []),
    receive {'DOWN', Ref, process, Pid, normal} = Down -> self() ! Down end,
    Result = demonitor(Ref, Options),
    {Result, receive {'DOWN', Ref, process, Pid, normal} -> down after 0 -> none end}.

holder() ->
    receive
        {demonitor, From, Ref} ->
            From ! {target, reason(catch demonitor(Ref, [info]))},
            holder();
        stop -> ok
    end.

gone() -> ok.

id(X) -> X.

reason({'EXIT', {Reason, Stack}}) when is_list(Stack) -> Reason;
reason(Value) -> {value, Value}.
