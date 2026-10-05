-module(re_probe).
-export([main/0, options/0, errors/0, badarg/0, iodata/0, unicode_list/0,
         export/0, unsupported/1, id/1]).

%% prim_tty:init/1 compiles the ANSI regular expression (prim_tty.erl:238).
%% The fifth element is a reference (erl_bif_re.c:607-609).
main() ->
    {ok, {re_pattern, Count, Unicode, Crlf, Ref}} =
        re:compile(prim_tty:ansi_regexp(), [unicode]),
    {ok, {re_pattern, _, _, _, Other}} = re:compile(<<"a">>, []),
    [Count, Unicode, Crlf, kind(Ref), Ref =:= Other].

%% The capture count, the unicode flag, and the newline flag of each
%% compile option (build_compile_result, erl_bif_re.c:581-615).
options() ->
    [info(re:compile(Pattern, Options))
     || {Pattern, Options} <-
            [{<<"a">>, []}, {<<"(a)(?:b)(c)">>, []}, {<<"a">>, [unicode]},
             {<<"a">>, [{newline, cr}]}, {<<"a">>, [{newline, lf}]},
             {<<"a">>, [{newline, crlf}]}, {<<"a">>, [{newline, nul}]},
             {<<"a">>, [{newline, anycrlf}]}, {<<"a">>, [{newline, any}]},
             {<<"(*CRLF)a">>, []}, {<<"(*ANY)a">>, [{newline, cr}]},
             {<<"(*UTF)a">>, []}, {<<"a">>, [{newline, crlf}, {newline, lf}]},
             {<<"(?<n>a)(?<n>b)">>, [dupnames]}, {<<"(a)">>, [no_auto_capture]},
             {<<"a">>, [anchored, caseless, dollar_endonly, dotall, extended,
                        firstline, multiline, ungreedy, ucp, no_start_optimize,
                        bsr_anycrlf, bsr_unicode, never_utf]}]].

%% The PCRE2 message and offset of each compile error
%% (build_compile_error, erl_bif_re.c:802-836).
errors() ->
    [re:compile(Pattern, Options)
     || {Pattern, Options} <-
            [{<<"(a)(b">>, [unicode]}, {<<"(a)(b">>, []}, {<<"[">>, []},
             {<<"a{2,1}">>, []}, {<<"(?<n>a)(?<n>b)">>, []},
             {<<255>>, [unicode]}, {<<"a">>, [never_utf, unicode]},
             {<<"(*UTF)a">>, [never_utf]},
             {[repeat(10001, "(?="), repeat(10001, ")")], []}]].

%% badarg for each option that parse_options rejects
%% (erl_bif_re.c:350-545), each execution option (erl_bif_re.c:914-916),
%% and each pattern that is not iodata (erl_bif_re.c:927-929).
badarg() ->
    [catch_error(fun() -> re:compile(Pattern, Options) end)
     || {Pattern, Options} <-
            [{<<"a">>, foo}, {<<"a">>, [bogus]}, {<<"a">>, [1]},
             {<<"a">>, [{newline, bogus}]}, {<<"a">>, [{bogus, 1}]},
             {<<"a">>, [{1, 2}]}, {<<"a">>, [{newline, lf, x}]},
             {<<"a">>, [global]}, {<<"a">>, [report_errors]},
             {<<"a">>, [notempty]}, {<<"a">>, [notempty_atstart]},
             {<<"a">>, [notbol]}, {<<"a">>, [noteol]},
             {<<"a">>, [{offset, 0}]}, {<<"a">>, [{offset, ?MODULE:id(-1)}]},
             {<<"a">>, [{match_limit, 1}]},
             {<<"a">>, [{match_limit_recursion, 1}]},
             {<<"a">>, [{capture, all}]}, {<<"a">>, [{capture, all, list}]},
             {<<"a">>, [unicode, global]}, {foo, []},
             {[?MODULE:id(256)], []}, {[?MODULE:id(-1)], []},
             {[<<"a">>, foo], []}, {{a}, []}, {foo, [bogus]}]].

%% Without unicode, a pattern is iodata: bytes, binaries, and nested lists
%% (get_iolist_as_bytes, erl_bif_re.c:850-889).
iodata() ->
    [info(re:compile(Pattern, []))
     || Pattern <- ["a(b)", ["a", <<"(b)">>, $c, [[<<"(d)">>]]], <<>>, []]].

%% With unicode, a pattern that is not a binary traps to re:ucompile/2
%% (erl_bif_re.c:921-923).
unicode_list() ->
    [info(re:compile([$a, ?MODULE:id(256), "(b)"], [unicode])),
     info(re:compile(["(a)", <<"(b)">>], [unicode])),
     re:compile("(", [unicode])].

%% The export option (build_compile_export, erl_bif_re.c:644-700).
%% Encoded holds the compile options and the bsr convention of the code,
%% so each compile option gives other bytes. The last bsr option applies.
export() ->
    [exported(re:compile(<<"a">>, [export])),
     exported(re:compile(["a", <<"b">>], [export, caseless])),
     exported(re:compile([$a, ?MODULE:id(256)], [unicode, export]))]
    ++ [exported(re:compile(<<"a">>, [export | Options]))
        || Options <- [[anchored], [caseless], [dollar_endonly], [dotall],
                       [dupnames], [extended], [firstline], [multiline],
                       [ungreedy], [ucp], [no_auto_capture],
                       [no_start_optimize], [bsr_anycrlf], [bsr_unicode],
                       [never_utf],
                       [bsr_anycrlf, bsr_unicode],
                       [bsr_unicode, bsr_anycrlf]]]
    ++ [re:compile(<<"(">>, [export])].

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(chardata) -> re:compile(foo, [unicode]).

info({ok, {re_pattern, Count, Unicode, Crlf, Ref}}) when is_reference(Ref) ->
    {Count, Unicode, Crlf}.

exported({ok, {re_exported_pattern, Header, Source, Options, Encoded}}) ->
    [Header, Source, Options, Encoded].

kind(Ref) when is_reference(Ref) -> reference;
kind(_) -> other.

%% Part N times as iodata. Each step takes a block of Size parts and
%% doubles it, so the iodata has few list cells.
repeat(N, Part) -> repeat(N, Part, 1, Part, []).

repeat(0, _, _, _, Acc) -> Acc;
repeat(N, Part, Size, Block, Acc) when Size =< N ->
    repeat(N - Size, Part, Size + Size, [Block, Block], [Block | Acc]);
repeat(N, Part, _, _, Acc) -> repeat(N, Part, 1, Part, Acc).

id(X) -> X.

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.
