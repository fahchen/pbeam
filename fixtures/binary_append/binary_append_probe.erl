-module(binary_append_probe).
-export([repeat/2, append/2, append_many/3, match_many/3, match_built/3, repeated/2]).

%% A binary of Count copies of Unit, built by doubling with the retained
%% prefix: each step appends the same half to itself.
repeat(0, _) -> <<>>;
repeat(Count, Unit) ->
    Half = repeat(Count div 2, Unit),
    case Count rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Unit/binary>>
    end.

%% One append of B to A.
append(A, B) -> <<A/binary, B/binary>>.

%% Rounds appends of B to A. Each result is dropped, so the measured path
%% keeps the prefix of one append.
append_many(_A, _B, 0) -> ok;
append_many(A, B, Rounds) ->
    _ = append(A, B),
    append_many(A, B, Rounds - 1).

%% Rounds matches of Needle in Subject. The first call warms a representation
%% that builds its contiguous bytes at the first read.
match_many(_Subject, _Needle, 0) -> ok;
match_many(Subject, Needle, Rounds) ->
    _ = binary:match(Subject, Needle),
    match_many(Subject, Needle, Rounds - 1).

%% Build the doubling subject and match it Rounds times.
match_built(Count, Needle, Rounds) ->
    Subject = repeat(Count, <<"a">>),
    match_many(Subject, Needle, Rounds).

%% Rounds appends of one copy of Unit, so a caller sees the size of the
%% result and the bytes copied by each append.
repeated(Count, Unit) ->
    lists:foldl(fun(_, Acc) -> <<Acc/binary, Unit/binary>> end, <<>>,
                lists:seq(1, Count)).
