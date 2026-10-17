-module(existing_atom_probe).
-export([main/0, bits/0, id/1]).

%% `erlang:list_to_existing_atom/1` (`bif.c:3239-3258`) and
%% `erlang:binary_to_existing_atom/2`, which reads the binary with
%% `must_exist` one (`erl_unicode.c:1962-1971`, `2025-2028`): both give the
%% atom of the name of the argument, when the run holds that atom, and
%% `badarg` for each other argument. It gives the UTF-8 name of the atom,
%% so the text of the probe is not a locale-dependent byte string.
%% `binary_to_existing_atom/2` reads `latin1` as one character for each
%% byte, and `utf8` and `unicode` as the UTF-8 bytes of the name. The binary
%% of 1021 bytes is above `MAX_ATOM_SZ_LIMIT` and the list of 256 elements
%% is above `MAX_ATOM_CHARACTERS` (`atom.h:29-31`). `id/1` hides the
%% arguments from the compiler.
main() ->
    %% Put the atoms of the accepted cases into the atom table of the run:
    %% the BIFs of this probe read the table, they do not add to it. `id/1`
    %% stops the compiler from folding the calls and dropping them.
    _ = erlang:list_to_atom(?MODULE:id([255])),
    _ = erlang:list_to_atom(?MODULE:id(repeat_char($a, 255))),
    [list_result([111, 107]),
     list_result([111, 107, 111, 107]),
     list_result([]),
     list_result([255]),
     list_result(repeat_char($a, 255)),
     list_result(bad),
     list_result([111, 107 | bad]),
     list_result([111, 16#110000]),
     list_result([111, 16#D800]),
     list_result([111, 1.0]),
     list_result([111, -1]),
     list_result(repeat_char($a, 256)),
     binary_result(<<"ok">>, utf8),
     binary_result(<<"ok">>, latin1),
     binary_result(<<"ok">>, unicode),
     binary_result(<<>>, utf8),
     binary_result(<<>>, latin1),
     binary_result(<<255>>, latin1),
     binary_result(<<195, 191>>, utf8),
     binary_result(<<255>>, utf8),
     binary_result(repeat_byte(97, 255), latin1),
     binary_result(repeat_byte(97, 256), latin1),
     binary_result(repeat_byte(97, 256), utf8),
     binary_result(repeat_byte(255, 256), latin1),
     binary_result(repeat_byte(97, 1021), utf8),
     binary_result(<<"ok">>, foo),
     binary_result(<<"ok">>, <<"utf8">>),
     binary_result(ok, utf8),
     binary_result("ok", utf8)].

id(X) -> X.

%% The BIF for a bit string that is not a whole number of bytes. ERTS
%% gives `badarg`. The frame of that exception holds the bit string
%% itself, which the test cannot build, so this probe gives the reason
%% alone.
bits() ->
    try erlang:binary_to_existing_atom(?MODULE:id(<<1:1>>), utf8) of
        Value -> {ok, Value}
    catch
        error:Reason -> {error, Reason}
    end.

%% Give the value of the BIF, or the reason and the stack trace of the
%% exception.
list_result(List) ->
    try erlang:list_to_existing_atom(?MODULE:id(List)) of
        Atom -> {ok, erlang:atom_to_binary(Atom, utf8)}
    catch
        error:Reason:Stacktrace -> {error, Reason, frames(Stacktrace)}
    end.

binary_result(Binary, Encoding) ->
    try erlang:binary_to_existing_atom(?MODULE:id(Binary), ?MODULE:id(Encoding)) of
        Atom -> {ok, erlang:atom_to_binary(Atom, utf8)}
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

%% N copies of Char, as a list.
repeat_char(Char, N) when N > 0 -> [Char | repeat_char(Char, N - 1)];
repeat_char(_, 0) -> [].

%% N copies of Byte, as a binary.
repeat_byte(Byte, N) -> erlang:list_to_binary(repeat_char(Byte, N)).
