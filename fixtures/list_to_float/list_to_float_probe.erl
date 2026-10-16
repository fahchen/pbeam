-module(list_to_float_probe).
-export([main/0, uncaught/0]).

%% erlang:list_to_float/1 for each text of the scan of sys_chars_to_double
%% (sys_float.c:72-140) and each list shape of list_to_float_1
%% (bif.c:3699-3725). The result of a text that gives a float is the
%% shortest text of that float (float_to_list/2 with short), so the compare
%% reads no float term.
%%
%% The fixture has no line table, so ERTS gives each frame of this module
%% the location [].
%%
%% In the C locale the first strtod attempt of a text with a comma reads the
%% sign and the digits before the comma only. A prefix that overflows an
%% infinity, such as 309 nines, gives a result that is not finite, and ERTS
%% rejects it before the second attempt (sys_float.c:109-124). The call
%% then raises badarg whatever the full text is worth: 309 nines raise
%% badarg, and 308 nines give 1.0. Adjacent string literals hold each count of nines without
%% a call to a BIF.
main() ->
    results(["1.0", "1,5", "-1.5", "+1.0", "-0.0", "0.0", "1.5E3", "1.0e10",
             "1.0e+3", "0001.500", "1.0e-320", "1.0e-400", "0.0e-400", "0.1",
             "0.30000000000000004", "1.7976931348623157e308",
             "1.7976931348623159e308", "5.0e-324", "4.9406564584124654e-324",
             "3.14159265358979323846264338327950288",
             "1", "", "abc", "1.0x", "1.5e400", "1.0e309", "1.", ".1",
             "!1.0", "1.0e", "1.0e+", "0x1.8p3", "1.0 ", [1, 2, 3],
             [$1, $., $0 | foo], [1.0], [4851, 700], abc,
             [$1, $., $0, 0, 120], [$1, $., $0, 0], [$1, 0, $., $0], [0],
             [0, $1, $., $0], [$1, $., $5, 0, 255], [$1, $., $0, 0, 4851],
             [$1, $., $5, $e, 0, $5], [0, 0], [$1, $., $0, 0, 0, $0],
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999"
             ",5e-308",
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "9999999999999999999999999999999999999999999999999999999999999999"
             "99999999999999999999999999999999999999999999999999999"
             ",5e-309"]).

%% The root process exits with the result of main/0, so the run gives its
%% text.
uncaught() ->
    exit(main()).

results([]) -> [];
results([Text | Rest]) -> [result(Text) | results(Rest)].

result(Text) ->
    try {ok, float_to_list(list_to_float(Text), [short])}
    catch Class:Reason:Stack -> {Class, Reason, stack(Stack)} end.

%% Drop the frames of the erl -eval caller of the run.
stack([{Module, _, _, _} = Frame | Rest])
  when Module =/= erl_eval, Module =/= init ->
    [Frame | stack(Rest)];
stack(_) -> [].
