%% Load `host_term_probe` from a BEAM binary at run time and return a local fun
%% of the loaded module. Two runs of one program that load different code under
%% that name hold local funs with the same module name and different code.
-module(host_term_loader).

-export([accept/1, load/1]).

%% Load `host_term_probe` from `beam` and return a local fun of the loaded
%% module.
load(Beam) ->
    {module, host_term_probe} = erlang:load_module(host_term_probe, Beam),
    host_term_probe:local_fun().

%% Return the fun argument.
accept(Fun) ->
    Fun.
