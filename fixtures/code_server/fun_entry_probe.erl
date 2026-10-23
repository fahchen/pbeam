-module(fun_entry_probe).
-export([stale/0, identical/0, deleted/0]).

%% The -on_load function of the first version of fun_entry_send sends two
%% funs of its code to this process and fails, so code_server removes the
%% code (code_server.erl:1360-1368). A load of the second version of the
%% same module name makes the entry of one fun differ in its old unique
%% value and the entry of the other fun absent, so each call raises
%% error:{badfun, Fun} (beam_common.c:1697-1706).
stale() ->
    {First, Second, Waiting} = funs_of_removed_code(),
    %% The name of a fun of code that is not loaded is `[]`
    %% (erl_bif_info.c:3998-4001), also when the code_server purged the
    %% module after the failed -on_load function. `Waiting` holds the names
    %% that the -on_load function read while the code waited for it.
    Before = [erlang:fun_info(F, name) || F <- [First, Second]],
    {module, fun_entry_send} = code:load_abs("fixtures/fun_entry_rev/fun_entry_send"),
    After = [erlang:fun_info(F, name) || F <- [First, Second]],
    %% erlang:fun_info_mfa/1 gives the module, `[]`, and the arity of a fun
    %% of code that is not loaded (erl_bif_info.c:4027-4035).
    Mfas = [erlang:fun_info_mfa(F) || F <- [First, Second]],
    io:format("~w~n", [[call(First), call(Second), Waiting, Before, After, Mfas]]).

%% `erlang:delete_module/1` moves the code of a module to old code, which
%% keeps the name of the function of each fun until the purge
%% (`beam_bif_load.c:2412-2455`, `erl_fun.c:37-46`).
deleted() ->
    {module, fun_entry_send} = code:load_abs("fixtures/fun_entry_rev/fun_entry_send"),
    Fun = fun_entry_send:make_rev(rev),
    Before = erlang:fun_info(Fun, name),
    true = code:delete(fun_entry_send),
    Deleted = erlang:fun_info(Fun, name),
    %% A load of another version makes new current code and keeps the old
    %% code, so the fun of the old code keeps the name of its function.
    {module, fun_entry_send} = code:load_abs("fixtures/code_server/fun_entry_send"),
    Reloaded = erlang:fun_info(Fun, name),
    io:format("~w~n", [[Before, Deleted, Reloaded]]).

%% A later load of the code that made the funs, while this process holds no
%% name, makes the code current again, so each call enters the function of
%% its lambda.
identical() ->
    {First, Second, _Waiting} = funs_of_removed_code(),
    true = unregister(fun_entry_probe),
    {module, fun_entry_send} = code:load_abs("fixtures/code_server/fun_entry_send"),
    io:format("~p~n", [[call(First), call(Second)]]).

%% Load the first version of fun_entry_send and return the two funs that
%% its -on_load function sent before the code_server removed the code.
funs_of_removed_code() ->
    true = register(fun_entry_probe, self()),
    {error, on_load_failure} = code:ensure_loaded(fun_entry_send),
    {First, FirstName} = receive {stale_fun, first, F1, N1} -> {F1, N1} end,
    {Second, SecondName} = receive {stale_fun, second, F2, N2} -> {F2, N2} end,
    {First, Second, [FirstName, SecondName]}.

%% Call the fun and report the class and the reason of the call. The reason
%% of an error holds the called fun, which io_lib prints through
%% erlang:fun_to_list/1, a BIF that PBeam does not have, so the reported
%% reason holds the atom fun_term for the fun of the reason and the clause guard
%% compares that fun with the called fun.
call(Fun) ->
    try Fun()
    catch error:{badfun, BadFun} when BadFun =:= Fun -> {error, {badfun, fun_term}};
          Class:Reason -> {Class, Reason}
    end.
