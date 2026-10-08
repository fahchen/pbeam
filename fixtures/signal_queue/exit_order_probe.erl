-module(exit_order_probe).
-compile([export_all, nowarn_export_all]).

%% Each case gives the numbers of the members in the order of the signals
%% that a process X sends to them at its exit. The result is {Case, Order}.
all() ->
    [{Case, ?MODULE:Case()} || Case <- cases()].

%% The probe calls no other module than erlang, so it has its own list
%% functions.
seq(I, N) when I > N -> [];
seq(I, N) -> [I | seq(I + 1, N)].

nth(1, [X | _]) -> X;
nth(I, [_ | Xs]) -> nth(I - 1, Xs).

delete(_, []) -> [];
delete(X, [X | Xs]) -> Xs;
delete(X, [Y | Xs]) -> [Y | delete(X, Xs)].

cases() ->
    [links_ascending, links_mixed, links_unlink, link_signal, unlink_signal,
     monitors, monitors_demonitor].

%% Seven links in pid order.
links_ascending() ->
    links(7, [{link, I} || I <- seq(1, 7)]).

links_mixed() ->
    links(7, [{link, I} || I <- [4, 1, 6, 2, 7, 3, 5]]).

%% Unlinks that delete nodes with two children, one child, and no child.
links_unlink() ->
    links(8, [{link, I} || I <- seq(1, 8)]
          ++ [{unlink, 4}, {unlink, 2}, {unlink, 6}, {link, 4}]).

%% X handles the link signal of member 1, and then links member 3.
link_signal() ->
    links(3, [{linked_by, 1}, {link, 3}]).

%% X handles the unlink signal of member 2.
unlink_signal() ->
    links(5, [{link, I} || I <- seq(1, 5)] ++ [{unlinked_by, 2}]).

%% ERTS orders the monitors by the bytes of their reference numbers, and
%% the low byte of the first number comes first. The reference counter of
%% ERTS starts at a value from the clock, so the order can change from run
%% to run when the low byte goes past 255 between the monitors. These two
%% cases give the same order for each counter value.
monitors() ->
    monitors(2, [{monitor, 1}, {monitor, 2}]).

monitors_demonitor() ->
    monitors(3, [{monitor, 1}, {monitor, 2}, {demonitor, 1}, {monitor, 3}]).

%% A member ends at a link exit signal, because it does not trap exits.
member() ->
    receive
        {ping, From} -> From ! pong, member();
        {link, P, From} -> link(P), From ! linked, member();
        {unlink, P, From} -> unlink(P), From ! unlinked, member();
        go -> exit(go)
    end.

%% X runs Ops and exits with boom. The members that are linked to X then
%% end in the order of the link exit signals.
links(N, Ops) ->
    Members = [spawn(?MODULE, member, []) || _ <- seq(1, N)],
    [monitor(process, M) || M <- Members],
    Linked = linked(Ops, []),
    x(Members, Ops, boom),
    Order = downs(Members, length(Linked)),
    [exit(M, kill) || M <- Members],
    downs(Members, N - length(Linked)),
    Order.

%% X runs Ops and exits with normal. Its demonitor signals put the
%% monitored members in the run queue. The case process runs first, as it
%% yields before X exits, and sends go to each member. The members then
%% run and end in the order of the run queue.
monitors(N, Ops) ->
    Members = [spawn(?MODULE, member, []) || _ <- seq(1, N)],
    [monitor(process, M) || M <- Members],
    x(Members, Ops, normal),
    [M ! go || M <- Members],
    downs(Members, N).

x(Members, Ops, Reason) ->
    Self = self(),
    X = spawn(fun() ->
                      run(Members, Ops, []),
                      Self ! ready,
                      receive exit -> exit(Reason) end
              end),
    receive ready -> ok end,
    X ! exit,
    erlang:yield().

linked([], Linked) -> Linked;
linked([{Op, I} | Ops], Linked) when Op =:= link; Op =:= linked_by ->
    linked(Ops, [I | delete(I, Linked)]);
linked([{_, I} | Ops], Linked) ->
    linked(Ops, delete(I, Linked)).

run(_, [], _) -> ok;
run(Members, [{link, I} | Ops], Refs) ->
    link(nth(I, Members)),
    run(Members, Ops, Refs);
run(Members, [{unlink, I} | Ops], Refs) ->
    %% The pong comes after the unlink acknowledgment, so X removes the
    %% link before it continues.
    M = nth(I, Members),
    unlink(M),
    M ! {ping, self()},
    receive pong -> ok end,
    run(Members, Ops, Refs);
run(Members, [{linked_by, I} | Ops], Refs) ->
    nth(I, Members) ! {link, self(), self()},
    receive linked -> ok end,
    run(Members, Ops, Refs);
run(Members, [{unlinked_by, I} | Ops], Refs) ->
    nth(I, Members) ! {unlink, self(), self()},
    receive unlinked -> ok end,
    run(Members, Ops, Refs);
run(Members, [{monitor, I} | Ops], Refs) ->
    run(Members, Ops, [{I, monitor(process, nth(I, Members))} | Refs]);
run(Members, [{demonitor, I} | Ops], Refs) ->
    {I, Ref} = lists:keyfind(I, 1, Refs),
    demonitor(Ref),
    run(Members, Ops, Refs).

downs(_, 0) -> [];
downs(Members, K) ->
    receive
        {'DOWN', _, process, P, _} -> [index(P, Members, 1) | downs(Members, K - 1)]
    end.

index(P, [P | _], I) -> I;
index(P, [_ | Members], I) -> index(P, Members, I + 1).
