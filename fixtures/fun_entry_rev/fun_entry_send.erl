-module(fun_entry_send).
-export([value/0, make_rev/1]).

%% The second version of the module holds one lambda. Its FunT entry has
%% the index of the first lambda of the other version and another old
%% unique value, because the compiler takes the old unique value of each
%% lambda from the MD5 of the module (beam_asm.erl:251-262).
value() ->
    version_two.

make_rev(Value) ->
    fun() -> {Value, rev} end.
