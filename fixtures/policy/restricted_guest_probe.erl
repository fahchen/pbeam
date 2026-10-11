%% Operations that the strict policy of examples/restricted_guest does not
%% permit. The pbeam binary runs pbeam_boot:start(restricted_guest_probe,
%% Function) in boot mode with the policy restricted_guest_probe_policy,
%% which gives the capabilities of 'Elixir.RestrictedGuest.Policy'. Thus
%% each function runs in the root process after it enters the runtime,
%% with the capabilities of RestrictedGuest.main/0, as RestrictedGuest.main/0
%% does. The IO servers user and standard_error are boot processes and IO
%% services. Do not run the probe on ERTS: there each operation has its
%% effect, and halt/1 stops the runtime.
-module(restricted_guest_probe).
-export([denied/0, no_terminal/0]).

%% Exit with {result, Results}: the result of each operation, or denied
%% for error:capability_denied, also inside the exit reason of a
%% gen_server call. The first operation is permitted: a spawn of a module
%% of the guest, whose worker writes "worker 0: hello" to the terminal.
%% The IO requests that the terminal capability permits write their lines.
%% Each other operation is denied. The last operation asks the policy to
%% revoke the terminal capability of the root process, so the next output
%% is denied. The pbeam binary prints the exit reason.
denied() ->
    User = whereis(user),
    Worker = own_worker(),
    Logger = whereis(logger),
    Results = [
        {spawn_own_module, done},
        {spawn_other_module, result(fun() -> spawn(lists, seq, [1, 2]) end)},
        {getenv, result(fun() -> os:getenv("HOME") end)},
        {open_port, result(fun() -> open_port({spawn_executable, "/bin/sh"}, [binary]) end)},
        {halt, result(fun() -> halt(0) end)},
        {handle_other_signal, result(fun() -> os:set_signal(sigusr2, handle) end)},
        {ignore_signal, result(fun() -> os:set_signal(sigwinch, ignore) end)},
        {exit_user, result(fun() -> exit(User, kill) end)},
        {group_leader_other, result(fun() -> group_leader(self(), User) end)},
        {request, result(fun() -> pbeam_cap:request(env, home) end)},
        {puts, 'Elixir.IO':puts(<<"puts: permitted">>)},
        {standard_error, io:put_chars(standard_error, <<"standard_error: permitted\n">>)},
        {format, io:format("format: ~p~n", [permitted])},
        {write, io:write(permitted)},
        {nl, io:nl()},
        {requests, io:requests([{put_chars, unicode, <<"requests: ">>}, {requests, [{format, "~s~n", ["permitted"]}]}])},
        {empty_requests, io:requests([])},
        {false_from, result(fun() -> request(User, {io_request, Worker, make_ref(), {put_chars, unicode, <<"x\n">>}}) end)},
        {lists_mfa, result(fun() -> request(User, {io_request, self(), make_ref(), {put_chars, unicode, lists, flatten, [["x\n"]]}}) end)},
        {io_lib_nl, result(fun() -> request(User, {io_request, self(), make_ref(), {put_chars, unicode, io_lib, nl, []}}) end)},
        {format_fun, result(fun() -> io:format("~Kp~n", [fun(A, B) -> A =< B end, #{a => 1}]) end)},
        {write_fun, result(fun() -> request(User, {io_request, self(), make_ref(), {put_chars, unicode, io_lib, write, [#{a => 1}, [{maps_order, fun(A, B) -> A =< B end}]]}}) end)},
        {get_line, result(fun() -> io:get_line("") end)},
        {setopts, result(fun() -> io:setopts([binary]) end)},
        {requests_get_line, result(fun() -> io:requests([{put_chars, unicode, <<"x\n">>}, {get_line, unicode, ""}]) end)},
        {requests_not_list, result(fun() -> request(User, {io_request, self(), make_ref(), {requests, x}}) end)},
        {request_atom, result(fun() -> request(User, {io_request, self(), make_ref(), x}) end)},
        {message_atom, result(fun() -> User ! x end)},
        {message_tuple, result(fun() -> User ! {io_request, self(), x} end)},
        {message_tag, result(fun() -> User ! {io_reply, self(), x, {put_chars, unicode, <<"x\n">>}} end)},
        {serve_logger, pbeam_cap:serve(Logger, io)},
        {request_logger, result(fun() -> request(Logger, {io_request, self(), make_ref(), {put_chars, unicode, <<"x\n">>}}) end)},
        {start_child, result(fun() -> supervisor:start_child(logger_sup, #{id => x, start => {lists, seq, [1, 2]}}) end)},
        {add_handler, result(fun() -> logger:add_handler(x, logger_std_h, #{}) end)},
        {revoke, result(fun() -> pbeam_cap:request(terminal, revoke) end)},
        {puts_after_revoke, result(fun() -> 'Elixir.IO':puts(<<"x">>) end)}
    ],
    exit({result, Results}).

%% The policy gives this entry the capabilities of the strict policy
%% without the terminal capability, so IO.puts/1 is denied.
no_terminal() ->
    exit({result, [{puts, result(fun() -> 'Elixir.IO':puts(<<"x">>) end)}]}).

own_worker() ->
    Worker = spawn('Elixir.RestrictedGuest.Worker', run, [self(), 0]),
    receive
        {done, Worker} -> Worker
    end.

%% Send an IO request message to an IO server and wait for its reply, as
%% io:request/2 does (io.erl:1666-1685).
request(Server, {io_request, _From, ReplyAs, _Request} = Message) ->
    Server ! Message,
    receive
        {io_reply, ReplyAs, Reply} -> Reply
    end.

result(Fun) ->
    try
        Fun()
    catch
        error:capability_denied -> denied;
        exit:{{capability_denied, _}, {gen_server, call, _}} -> denied
    end.
