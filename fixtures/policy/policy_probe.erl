-module(policy_probe).
-export([
    initial/0,
    granted/0,
    forms/0,
    failures/0,
    requester/0,
    effect/0,
    wait/0,
    idle/0,
    exited/0,
    idle_exit/0,
    bad_initial/0,
    atom_initial/0,
    raise_initial/0,
    loop_initial/0,
    exit_initial/0,
    wait_initial/0,
    done/0,
    worker/1,
    hello/1,
    request_for/3
]).

%% The root process holds only the process capability that initial/1 of
%% the policy gives: spawn, link, and monitor for the modules of this
%% probe.
initial() ->
    [
        result(fun() -> spawn(?MODULE, done, []) end),
        result(fun() -> spawn(erl_policy, noop, []) end),
        result(fun() -> os:getenv("HOME") end)
    ].

%% The policy grants env read for HOME only.
granted() ->
    Before = result(fun() -> os:getenv("HOME") end),
    Granted = request(env, home),
    After = [result(fun() -> os:getenv("HOME") end), result(fun() -> os:getenv("PATH") end)],
    {Before, Granted, After}.

%% The policy returns each value that the request names. Only the last one
%% is a grant form.
forms() ->
    Answers = [
        deny,
        other,
        {grant},
        {other, #{}},
        {grant, not_a_map},
        {grant, #{kind => 1, operations => [read], names => any, delegable => false}},
        {grant, #{kind => file, operations => [read], delegable => false}},
        {grant, #{operations => [read], names => any, delegable => false}},
        {grant, #{kind => env, operations => [read], delegable => false}},
        {grant, #{kind => env, names => any, delegable => false}},
        {grant, #{kind => env, operations => [read], names => any}},
        {grant, #{kind => env, operations => [spawn], names => any, delegable => false}},
        {grant, #{kind => env, operations => [read], names => any, delegable => false, extra => 1}},
        {grant, #{kind => terminal, operations => [write], delegable => false}}
    ],
    [request(env, {answer, Answer}) || Answer <- Answers].

%% An exception, an exit exception, and a reduction limit overrun of the
%% policy code deny. A request of the policy process itself is denied, and
%% the policy then grants. A kind that is not valid raises badarg.
failures() ->
    [
        request(env, raise),
        request(env, exit),
        request(env, loop),
        request(env, self_request),
        request(file, home),
        request(<<"env">>, home)
    ].

%% The policy sends the requester map of a worker and the requester map of
%% the root process, which it kept from initial/1, to the worker. The
%% worker is linked to the root process and then crashes, but the policy
%% process is outside this process tree and answers the next request.
requester() ->
    Root = self(),
    process_flag(trap_exit, true),
    Worker = spawn_link(?MODULE, worker, [Root]),
    receive
        {Worker, Seen, RootSeen} ->
            receive
                {'EXIT', Worker, crash} ->
                    {
                        Seen =:= #{pid => Worker, parent => Root, initial_call => {?MODULE, worker, 1}},
                        RootSeen =:=
                            #{pid => Root, parent => none, initial_call => {?MODULE, requester, 0}},
                        request(env, home)
                    }
            end
    end.

%% The policy code spawns a process of a module that the root process
%% cannot spawn, and grants the spawn of that module.
effect() ->
    [
        result(fun() -> spawn(erl_policy, noop, []) end),
        request(process, effect),
        result(fun() -> spawn(erl_policy, noop, []) end)
    ].

%% A message that arrives while the request waits does not end the wait.
wait() ->
    spawn(?MODULE, hello, [self()]),
    Answer = request(env, {spin, 20000}),
    receive
        hello -> {Answer, hello}
    end.

%% A process of the policy code sends a message to the policy process after
%% the call. The policy process then waits for the next request.
idle() ->
    First = request(env, wake),
    receive
    after 10 -> ok
    end,
    [First, request(env, home)].

%% The policy process ends in a call. The requester of the call and a
%% request that waits are denied, and so is each later request.
exited() ->
    Root = self(),
    Worker = spawn(?MODULE, request_for, [Root, env, home]),
    First = request(env, {kill, 20000}),
    receive
        {Worker, Second} -> [First, Second, request(env, home)]
    end.

%% A process of the policy code, which is linked to the policy process,
%% crashes after the call. The link ends the policy process while it waits,
%% so each later request is denied.
idle_exit() ->
    First = request(env, link_crash),
    receive
    after 10 -> ok
    end,
    [First, request(env, home)].

bad_initial() -> done().
atom_initial() -> done().
raise_initial() -> done().
loop_initial() -> done().
exit_initial() -> done().
wait_initial() -> done().

done() -> ok.

worker(Root) ->
    denied = request(process, requester),
    receive
        {requester, Seen, RootSeen} ->
            Root ! {self(), Seen, RootSeen},
            exit(crash)
    end.

hello(Root) -> Root ! hello.

request_for(Root, Kind, Request) -> Root ! {self(), request(Kind, Request)}.

request(Kind, Request) -> result(fun() -> pbeam_cap:request(Kind, Request) end).

%% The class of a result: a pid, a string, or another value, or the error
%% reason.
result(Fun) ->
    try Fun() of
        Pid when is_pid(Pid) -> pid;
        [_ | _] -> string;
        Value -> Value
    catch
        error:capability_denied -> denied;
        error:Reason -> {error, Reason}
    end.
