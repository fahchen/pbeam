-module(plug_hello_run).
-export([main/0]).

%% Start the release of examples/plug_hello without its boot script, with the
%% application environment of the release, and wait. scripts/example.sh runs
%% this entry on the pinned ERTS and on the pbeam binary with the same code
%% path, and sets RELEASE_ROOT and RELEASE_SYS_CONFIG as bin/plug_hello sets
%% them.
%%
%% The release boot gives sys.config to application_controller with -config
%% (application_controller.erl:1964-2004), which reads it with
%% erl_prim_loader:read_file/1, erl_scan, and erl_parse, not with
%% file:consult/1 (application_controller.erl:2007-2021, 2057-2075). This
%% entry reads the same file in the same way and sets each value with application:set_env/2 and {persistent, true}
%% (application.erl:805), so that a later load of an application keeps it
%% (application_controller.erl:919-938).
%%
%% The boot script then calls 'Elixir.Config.Provider':boot/0 after the start
%% of kernel and stdlib (releases/0.1.0/start.script). This entry makes the
%% same call. It reads the providers from the key config_provider_init of
%% elixir in sys.config, evaluates releases/0.1.0/runtime.exs with
%% Config.Reader, and sets the result with Application.put_all_env/2
%% (Elixir v1.20.4 lib/elixir/lib/config/provider.ex:218-280).
%%
%% The boot script then starts each application of the release as permanent
%% (application:start_boot/2). This entry starts plug_hello and its
%% dependencies with application:ensure_all_started/2, also as permanent.
main() ->
    {ok, Bin} = erl_prim_loader:read_file(os:getenv("RELEASE_SYS_CONFIG") ++ ".config"),
    {ok, Tokens, _} = erl_scan:string(unicode:characters_to_list(Bin)),
    {ok, Config} = erl_parse:parse_term(Tokens),
    ok = application:set_env(Config, [{persistent, true}]),
    ok = 'Elixir.Config.Provider':boot(),
    {ok, _} = application:ensure_all_started(plug_hello, permanent),
    receive after infinity -> ok end.
