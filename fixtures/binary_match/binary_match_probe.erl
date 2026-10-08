-module(binary_match_probe).
-export([kinds/0, kind/1, upper/0, first/1, usascii/1, utf8/1, byte/1]).

%% Run the bs_start_match3 test on one term of each kind that PBeam has,
%% except a binary. The test fails to its label for each term, as in
%% unicode_util:cp/1 for a list.
kinds() ->
    Terms = [1, 1 bsl 60, a, "s", {t}, [], [a | b], #{k => v}, self(), make_ref(),
             fun kinds/0],
    [?MODULE:kind(Term) || Term <- Terms].

kind(<<_/utf8, _/binary>>) -> utf8;
kind(_) -> other.

%% string:uppercase/1 of a list, as logger_formatter does
%% (logger_formatter.erl:616). unicode_util:cp/1 runs bs_start_match3 on a list.
upper() ->
    [string:uppercase(S) || S <- ["error", "notice", "r"]].

%% bs_start_match3 on a binary.
first(<<C/utf8, _/binary>>) -> {utf8, C};
first(Other) -> {other, Other}.

%% Call is_usascii/1 with a binary, as prim_tty:to_latin1/2 does
%% (prim_tty.erl:1310-1314).
usascii(Bin) -> is_usascii(Bin).

%% The shape of prim_tty:is_usascii/1 (prim_tty.erl:1316-1321). The
%% recursive call gives the match context to the next bs_start_match3.
is_usascii(<<Char/utf8, T/binary>>) when Char < 128 ->
    is_usascii(T);
is_usascii(<<>>) ->
    true;
is_usascii(_) ->
    false.

%% bs_get_utf8 on the first character of a binary.
utf8(<<Char/utf8, _/binary>>) -> Char;
utf8(_) -> false.

%% The integer command of bs_match.
byte(<<Byte, _/binary>>) -> Byte;
byte(_) -> false.
