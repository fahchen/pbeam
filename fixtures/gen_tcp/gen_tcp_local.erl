-module(gen_tcp_local).
-export([main/0]).

%% Print the results of the `local` family at the file name in the
%% environment variable PBEAM_LOCAL_PATH: a transfer, a connect to a name
%% with no socket, and on Linux an abstract address.
main() ->
    Path = os:getenv("PBEAM_LOCAL_PATH"),
    Abstract = case os:type() of
        {unix, linux} -> abstract();
        _ -> linux_only
    end,
    io:format("~w~n", [[transfer(Path), refused(Path ++ "-none"), Abstract]]).

%% Listen, connect, accept, send, and receive on the `local` family at
%% the file name Path. Each address of Path is compared with Path, so the
%% result does not contain the file name.
transfer(Path) ->
    Bin = list_to_binary(Path),
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}, {ifaddr, {local, Path}}]),
    {ok, C} = gen_tcp:connect({local, Path}, 0, [binary, {active, false}]),
    {ok, A} = gen_tcp:accept(L),
    ok = gen_tcp:send(C, <<"ping">>),
    {ok, Ping} = gen_tcp:recv(A, 4),
    ok = gen_tcp:send(A, <<"pong">>),
    {ok, Pong} = gen_tcp:recv(C, 4),
    Result = [
        Ping,
        Pong,
        inet:sockname(L) =:= {ok, {local, Bin}},
        inet:peername(A),
        inet:peername(C) =:= {ok, {local, Bin}},
        inet:sockname(C),
        gen_tcp:listen(0, [binary, {ifaddr, {local, Path}}])
    ],
    ok = gen_tcp:close(A),
    ok = gen_tcp:close(C),
    ok = gen_tcp:close(L),
    Result.

%% Connect to the file name Path, where no socket listens.
refused(Path) ->
    gen_tcp:connect({local, Path}, 0, [binary]).

%% Listen and connect on a Linux abstract address.
abstract() ->
    Name = <<0, "pbeam-abstract-1198">>,
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}, {ifaddr, {local, Name}}]),
    {ok, C} = gen_tcp:connect({local, Name}, 0, [binary, {active, false}]),
    Result = [inet:sockname(L), inet:peername(C)],
    ok = gen_tcp:close(C),
    ok = gen_tcp:close(L),
    Result.
