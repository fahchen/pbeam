-module(flags_probe).
-export([main/1]).

%% Display the boot flag list that the pbeam binary gives the root
%% function. The module holds no flag name as an atom, so the binary must
%% add the name of each flag that it always gives.
main(Flags) ->
    erlang:display(Flags).
