%% A policy module for restricted_guest_probe in boot mode. initial/1 gives
%% the entry restricted_guest_probe:denied/0 the capabilities that the
%% strict policy 'Elixir.RestrictedGuest.Policy' gives RestrictedGuest.main/0,
%% and restricted_guest_probe:no_terminal/0 the same capabilities without
%% the terminal capability. Each other entry gets the value of the strict
%% policy. grant/3 revokes the terminal capability of the requester for
%% the request revoke and then denies it. The strict policy decides each
%% other request.
-module(restricted_guest_probe_policy).
-export([initial/1, grant/3]).

-define(STRICT, 'Elixir.RestrictedGuest.Policy').

initial(#{initial_call := {restricted_guest_probe, denied, 0}} = Requester) ->
    strict(Requester);
initial(#{initial_call := {restricted_guest_probe, no_terminal, 0}} = Requester) ->
    [Capability || #{kind := Kind} = Capability <- strict(Requester), Kind =/= terminal];
initial(Requester) ->
    ?STRICT:initial(Requester).

grant(terminal, revoke, #{pid := Pid}) ->
    ok = pbeam_cap:policy_revoke(Pid, terminal),
    deny;
grant(Kind, Request, Requester) ->
    ?STRICT:grant(Kind, Request, Requester).

strict(Requester) ->
    ?STRICT:initial(Requester#{initial_call := {'Elixir.RestrictedGuest', main, 0}}).
