-module(binary_to_atom_probe).
-export([main/0, id/1]).

%% `erlang:binary_to_atom/2` (`erl_unicode.c:1962-2033`): `latin1` reads
%% one byte for each character and `utf8` and `unicode` read the binary as
%% UTF-8. A binary that the encoding does not accept, an encoding that is
%% not valid, and an argument that is not a byte-aligned binary raise
%% `badarg`. A name above the atom limit raises `system_limit`. `id/1`
%% hides the arguments from the compiler.
main() ->
    [result(<<"ok">>, latin1),
     result(<<"ok">>, utf8),
     result(<<"ok">>, unicode),
     result(<<>>, utf8),
     result(<<0>>, latin1),
     result(<<255>>, latin1),
     result(<<197, 128>>, latin1),
     result(<<960/utf8>>, utf8),
     result(<<128512/utf8>>, utf8),
     size(repeat(<<255>>, 255), latin1),
     size(repeat(<<97>>, 255), utf8),
     result(<<255>>, utf8),
     result(<<237, 160, 128>>, utf8),
     result(<<192, 128>>, utf8),
     result(<<226, 130>>, utf8),
     result(repeat(<<255>>, 256), latin1),
     result(repeat(<<97>>, 256), utf8),
     result(repeat(<<255>>, 1021), utf8),
     result(<<255>>, foo),
     result(<<255>>, <<"utf8">>),
     result(ok, latin1),
     result([255], latin1),
     result(255, latin1)].

id(X) -> X.

%% Give the UTF-8 bytes of the name of the atom, or the reason and the
%% stack trace of the exception.
result(Binary, Encoding) ->
    try erlang:binary_to_atom(?MODULE:id(Binary), ?MODULE:id(Encoding)) of
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

%% Give the byte size of the UTF-8 name of an accepted binary, for the
%% cases at the atom limit.
size(Binary, Encoding) ->
    {ok, Name} = result(Binary, Encoding),
    {ok, byte_size(Name)}.

%% N copies of Bin, built by doubling.
repeat(_, 0) -> <<>>;
repeat(Bin, 1) -> Bin;
repeat(Bin, N) ->
    Half = repeat(Bin, N div 2),
    case N rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Bin/binary>>
    end.
