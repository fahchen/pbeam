-module(release_boot_probe).
-behaviour(application).
-behaviour(supervisor).
-export([main/0, start/2, stop/1, init/1]).

%% The application of the release fixture fixtures/release_boot. The boot
%% script start.boot loads it in embedded mode and starts it as permanent,
%% and -s release_boot_probe main runs main/0 after the boot
%% (init.erl:1192-1224, 1530-1546).

start(normal, []) ->
    supervisor:start_link({local, release_boot_probe_sup}, ?MODULE, []).

stop(_State) ->
    ok.

init([]) ->
    {ok, {#{}, []}}.

%% Display the running applications, the application environment of
%% sys.config, the code loading mode, the plain arguments of -extra, the
%% preloaded modules that are not loaded, the supervisor of the
%% application, and whether the first element of PATH is the bindir,
%% which erlexec puts first in each case (erlexec.c:556-620). Then stop
%% the node with init:stop/0.
main() ->
    {ok, [[Bindir]]} = init:get_argument(bindir),
    [First | _] = string:split(os:getenv("PATH"), ":"),
    io:format("~p~n", [{release_boot,
                        applications, application:which_applications(),
                        greeting, application:get_env(release_boot_probe, greeting),
                        mode, code:get_mode(),
                        plain, init:get_plain_arguments(),
                        not_loaded, [M || M <- erlang:pre_loaded(), not erlang:module_loaded(M)],
                        supervisor, is_pid(whereis(release_boot_probe_sup)),
                        path_bindir, First =:= Bindir}]),
    init:stop().
