%% A policy module for processes_probe: the root process gets no
%% capability. The policy process runs in the same run.
-module(processes_policy).
-export([initial/1, grant/3]).

initial(_Requester) ->
    [].

grant(_Kind, _Request, _Requester) ->
    deny.
