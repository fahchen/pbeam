-module(badmap_assoc_probe).
-export([main/0, assoc/1, id/1]).

%% A `put_map_assoc` on the argument, with no guard, so the instruction
%% gets a term that is not a map.
assoc(Term) -> Term#{k => 1}.

%% Each case catches the exception and returns the class, the reason, and
%% the top frame of the stack trace. The compiler guards `put_map_assoc`
%% with `is_map` and calls `erlang:error({badmap, Term})`. ERTS raises
%% `{badmap, Term}` for `maps:keys/1` (`erl_map.c:1235-1237`).
main() ->
    NotMap = ?MODULE:id(not_a_map),
    Assoc = try ?MODULE:assoc(NotMap) catch C1:R1:S1 -> {C1, R1, hd(S1)} end,
    Keys = try maps:keys(NotMap) catch C2:R2:S2 -> {C2, R2, hd(S2)} end,
    [Assoc, Keys].

id(Term) -> Term.
