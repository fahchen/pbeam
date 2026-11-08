%% The remote group leader check on a full atom table. The test supplies a
%% policy that denies the operation after the fixture has loaded.
-module(full_atom_remote_leader).
-export([run/0]).

run() ->
    Remote = binary_to_term(<<131, 88, 100, 6:16, "b@host", 7:32, 0:32, 5:32>>),
    erts_internal:group_leader(self(), Remote).
