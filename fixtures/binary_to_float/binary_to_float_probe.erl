-module(binary_to_float_probe).
-export([main/0, bits/0, id/1]).

%% `erlang:binary_to_float/1` (`bif.c:3727-3756`): the binary must hold a
%% whole number of bytes and at least one byte, and its text must read as a
%% float. ERTS gives the bytes a NUL terminator, so a zero byte ends the
%% text. An empty binary, a bit string that is not a whole number of bytes,
%% a term that is not a binary, and a text that the scan rejects or that
%% overflows an infinity raise `badarg`. `id/1` hides the argument from the
%% compiler.
main() ->
    [result(<<"1.0">>),
     result(<<"17.0">>),
     result(<<"2.2017764e+1">>),
     result(<<"1,5">>),
     result(<<"-0.0">>),
     result(<<"0.1">>),
     result(<<"1.5e-400">>),
     result(<<"5.0e-324">>),
     result(<<"2.2250738585072011e-308">>),
     result(<<"1.0e309">>),
     result(<<49, 46, 48, 0, 120>>),
     result(<<49, 0, 46, 48>>),
     result(<<0>>),
     result(<<>>),
     result(<<"1">>),
     result(<<"1.0 ">>),
     result(<<" 1.0">>),
     result(<<"0x1.0">>),
     result(1),
     result([]),
     result(ok)].

%% The BIF for a bit string that is not a whole number of bytes, and for
%% the empty bit string. ERTS accepts neither. The frame of such an
%% exception holds the bit string itself, which the test cannot build, so
%% this probe gives the reason alone.
bits() ->
    [reason(<<1:1>>), reason(<<1:0>>)].

reason(Binary) ->
    try erlang:binary_to_float(?MODULE:id(Binary)) of
        Value -> {ok, Value}
    catch
        error:Reason -> {error, Reason}
    end.

id(X) -> X.

%% Give the value of the BIF, or the reason and the stack trace of the
%% exception.
result(Binary) ->
    try erlang:binary_to_float(?MODULE:id(Binary)) of
        Value -> {ok, Value}
    catch
        error:Reason:Stacktrace -> {error, Reason, frames(Stacktrace)}
    end.

%% The frame of the BIF and the frames of this module below it: the
%% `erl -eval` caller adds its own frames after them, and a PBeam run has
%% none.
frames([Bif | Rest]) -> [Bif | frames_of_module(Rest)];
frames(_) -> [].

frames_of_module([Frame = {?MODULE, _, _, _} | Rest]) -> [Frame | frames_of_module(Rest)];
frames_of_module(_) -> [].
