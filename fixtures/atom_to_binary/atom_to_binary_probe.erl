-module(atom_to_binary_probe).
-export([main/0, id/1]).

%% `erlang:atom_to_binary/2` for each encoding and for an ASCII atom, the
%% atom '', an atom with a character in 128..255, an atom with a character
%% above 255, and an atom with the characters 255 and 256. The encoding
%% `foo` is not valid. `id/1` hides the arguments from the compiler.
main() ->
    Atoms = [ok, '', 'héllo', 'π', 'ÿĀ'],
    Encodings = [latin1, utf8, unicode, foo],
    [[result(Atom, Encoding) || Encoding <- Encodings] || Atom <- Atoms]
        ++ [result("ok", utf8), result(<<"ok">>, latin1), result(1, bad)].

id(X) -> X.

result(Atom, Encoding) ->
    try atom_to_binary(?MODULE:id(Atom), ?MODULE:id(Encoding))
    catch error:Reason -> {error, Reason}
    end.
