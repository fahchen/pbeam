-module(re_probe).
-export([main/0, options/0, errors/0, badarg/0, iodata/0, unicode_list/0,
         export/0, imported/0, imported_run/0, imported_badarg/0,
         imported_badopt/0,
         unsupported/1, id/1,
         run2/0, run/0, captures/0, named_captures/0, named_errors/0,
         globals/0, run_errors/0, reductions/0, named_reductions/0,
         spot/0, internal_run_first/0, yield/0, yield_charge/0, recursion/0,
         unexpected_error/0, forged_pairs/0, recursion_after_restart/0]).

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

%% re:import/1 (re_import_1, erl_bif_re.c:703-796). A valid export
%% decodes with PCRE2 and gives the pattern tuple without `ok`. The unicode
%% element is the unicode byte of the header, also when it is not 0 or 1.
%% An export from another encode version, PCRE2 version, or architecture
%% re-compiles Source with Options.
imported() ->
    [import_info(re:import(Export)) || Export <- import_exports()]
    ++ [catch_error(fun() -> re:import(Term) end)
        || Term <- import_badarg_terms()]
    ++ [import_info(re:import(Export)) || Export <- import_fallbacks()].

import_exports() ->
    Plain = exported_pattern(<<"a(b)">>, [export]),
    [Plain,
     exported_pattern(<<"(a)(?:b)(c)">>, [export]),
     exported_pattern(<<"a(b)">>, [unicode, export]),
     exported_pattern(<<"(*CRLF)a(b)">>, [export]),
     with_header(Plain, 1, 7),
     header_extra(Plain)].

imported_run() ->
    [re:run(<<"zab">>, re:import(Export), [{capture, all, list}])
     || Export <- import_exports() ++ import_fallbacks()].

imported_badarg() ->
    re:import(foo).

%% The fallback compiles the source with the options, so an execution
%% option raises `badarg` with the `error_info` cause `badopt`
%% (erl_bif_re.c:908-916).
imported_badopt() ->
    re:import({re_exported_pattern, <<"re-PCRE2", 0:32, 2, 0>>, <<"a">>,
               [global], <<>>}).

import_fallbacks() ->
    Plain = exported_pattern(<<"a(b)">>, [export]),
    {re_exported_pattern, _, _, [export], _} = Plain,
    UnicodeOptions = [unicode, export],
    Version2 = with_header(Plain, 2, 0),
    [Version2,
     with_options(Version2, UnicodeOptions),
     set_encoded(Version2, notbin),
     zero_word(Plain, 0),
     zero_word(Plain, 4),
     zero_word(Plain, 8)].

import_badarg_terms() ->
    Plain = exported_pattern(<<"a(b)">>, [export]),
    {re_exported_pattern, H, S, O, E} = Plain,
    <<T:8/binary, C:32, V:8, U:8>> = H,
    Version2 = with_header(Plain, 2, 0),
    [foo,
     {re_exported_pattern},
     {re_exported_pattern, H, S, O},
     {re_exported_pattern, H, S, O, E, extra},
     {other, H, S, O, E},
     {re_exported_pattern, notbin, S, O, E},
     {re_exported_pattern, binary:part(H, 0, 13), S, O, E},
     {re_exported_pattern, <<"re-PCRE3", (binary:part(H, 8, 6))/binary>>,
      S, O, E},
     {re_exported_pattern, <<T/binary, (C bxor 1):32, V, U>>, S, O, E},
     {re_exported_pattern, H, S, O, notbin},
     with_source(Version2, notbin),
     with_source(Version2, <<"(">>),
     with_options(Version2, [export, global]),
     with_options(Version2, foo),
     with_encoded(Plain, binary:part(E, 0, 16))].

exported_pattern(Pattern, Options) ->
    element(2, re:compile(Pattern, Options)).

%% Return Export with a header of Version and Unicode, and the checksum of
%% its encoded binary.
with_header({re_exported_pattern, H, S, O, E}, Version, Unicode) ->
    <<T:8/binary, _:32, _:16>> = H,
    {re_exported_pattern,
     <<T/binary, (crc32(E)):32, Version, Unicode>>, S, O, E}.

%% Return Export with Encoded and the checksum of Encoded. A header of
%% another encode version keeps the checksum but ERTS does not read it.
with_encoded({re_exported_pattern, H, S, O, _}, Encoded) ->
    <<T:8/binary, _:32, V:8, U:8>> = H,
    {re_exported_pattern,
     <<T/binary, (crc32(Encoded)):32, V, U>>, S, O, Encoded}.

%% Return Export with Encoded replaced and the header kept.
set_encoded({re_exported_pattern, H, S, O, _}, Encoded) ->
    {re_exported_pattern, H, S, O, Encoded}.

%% Return Export with Source replaced.
with_source({re_exported_pattern, H, _, O, E}, Source) ->
    {re_exported_pattern, H, Source, O, E}.

%% Return Export with Options replaced.
with_options({re_exported_pattern, H, S, _, E}, Options) ->
    {re_exported_pattern, H, S, Options, E}.

%% Return Export with the header followed by bytes that ERTS ignores.
header_extra({re_exported_pattern, H, S, O, E}) ->
    {re_exported_pattern, <<H/binary, "extra">>, S, O, E}.

%% Return Export with the four bytes of Encoded at Offset set to zero and
%% the checksum of the new Encoded. Offset 0 is the magic number, 4 is the
%% PCRE2 version, and 8 is the PCRE2 configuration
%% (pcre2_serialize.c:53-61).
zero_word({re_exported_pattern, H, S, O, E}, Offset) ->
    <<A:Offset/binary, _:32, B/binary>> = E,
    with_encoded({re_exported_pattern, H, S, O, E}, <<A/binary, 0:32, B/binary>>).

import_info({re_pattern, Count, Unicode, Crlf, Ref}) when is_reference(Ref) ->
    {Count, Unicode, Crlf};
import_info(Other) ->
    Other.

%% The zlib CRC-32 of Bin, as calc_checksum (erl_bif_re.c:635-639) gets
%% it with crc32(0, bytes, size).
crc32(Bin) ->
    crc32(Bin, 16#FFFFFFFF) bxor 16#FFFFFFFF.

crc32(<<Byte, Rest/binary>>, Crc) ->
    crc32(Rest, crc32_byte(Crc bxor Byte, 8));
crc32(<<>>, Crc) ->
    Crc.

crc32_byte(Crc, 0) ->
    Crc;
crc32_byte(Crc, N) ->
    Mask = case Crc band 1 of 1 -> 16#EDB88320; 0 -> 0 end,
    crc32_byte((Crc bsr 1) bxor Mask, N - 1).

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(chardata) -> re:compile(foo, [unicode]).

%% re:run/2: the option list is []. The subject and the pattern are a
%% binary or a list (re_run, erl_bif_re.c:1485-1803).
run2() ->
    [re:run(Subject, Pattern)
     || {Subject, Pattern} <-
            [{<<"abc">>, <<"b">>}, {"abc", "b"}, {<<"abc">>, <<"z">>},
             {<<>>, <<>>}, {[<<"a">>, "bc"], <<"c">>}]].

%% re:run/3 with the options that the Plug serve path uses.
run() ->
    {ok, Compiled} = re:compile(<<"(b)">>, []),
    {ok, Caseless} = re:compile(<<"b">>, [caseless]),
    {ok, CompiledUnicode} = re:compile(<<"(b)">>, [unicode]),
    {re_pattern, _, _, _, CRef} = Compiled,
    [re:run(Subject, RE, Options)
     || {Subject, RE, Options} <- run_cases()] ++
        [re:run(<<"abcabc">>, Compiled, [global]),
         re:run(<<"abcabc">>, Compiled, [{capture, [1], binary}]),
         re:run(<<"abc">>, Caseless, []),
         re:run(<<"abc">>, Compiled, [{newline, lf}]),
         re:run(<<"abc">>, Compiled, [bsr_unicode]),
         re:run("abc", CompiledUnicode, []),
         re:run(<<"abc">>, {re_pattern, -1, 0, 0, CRef}, []),
         re:run(<<>>, {re_pattern, 0, 0, 0, CRef}, [global, {offset, 1}, {newline, crlf}])].

run_cases() ->
    [{<<"abc">>, <<"b">>, []},
     {<<"abc">>, <<"z">>, []},
     {<<"abc">>, <<"(a)(b)">>, [{offset, 1}]},
     {<<"abc">>, <<"b">>, [{offset, 2}]},
     {<<"abc">>, <<"b">>, [{offset, 3}]},
     {"abc", <<"b">>, [{capture, all, list}]},
     {[$a, <<"b">>, "c"], <<"(b)">>, [{capture, all, binary}]},
     {<<"a\nb">>, <<"a.b">>, [dotall]},
     {<<"ab">>, <<"^b">>, [multiline]},
     {<<"abc">>, <<"a b c">>, [extended]},
     {<<"ABC">>, <<"abc">>, [caseless]},
     {<<"abc">>, <<"a+?">>, [ungreedy]},
     {<<"abc">>, <<"(a)(b)">>, [no_auto_capture]},
     {<<"a\r\nb">>, <<"^b">>, [{newline, anycrlf}, multiline]},
     {<<"abc">>, <<"b">>, [bsr_unicode]},
     {<<"abc">>, <<"b">>, [{match_limit, 1000}]},
     {<<"abc">>, <<"b">>, [{match_limit_recursion, 1000}]},
     {<<"abcabc">>, <<"b">>, [{capture, all, binary}]},
     {<<"\x{68}\x{e9}llo"/utf8>>, <<"\x{e9}"/utf8>>, [unicode]},
     {<<"\x{68}\x{e9}llo"/utf8>>, "\x{e9}", [unicode]},
     {<<"abc">>, <<"(b)">>, [unicode, {capture, all, binary}]},
     {<<"abc">>, <<"(b)">>, [unicode, {capture, all, list}]},
     {<<"abc">>, <<"(b)">>, [{capture, all_names}]},
     {<<"abc">>, <<"(b)">>, [notbol]},
     {<<"abc">>, <<"c$">>, [noteol]},
     {<<"abc">>, <<"b">>, [notempty, anchored, no_start_optimize]},
     {<<"abc">>, <<"z">>, [report_errors]},
     {<<"a">>, <<"a">>, [{match_limit, 1}, report_errors]},
     {<<"a">>, <<"a">>, [{match_limit, 1}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, all_but_first}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, [2, 5], index}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, first}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, none}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, none, list}]},
     {<<"ab">>, <<"(a)(z)?(b)">>, [{capture, all}]},
     {<<"abc">>, <<"a(b">>, [report_errors]},
     {<<"aaaaaaaaaaaaaz">>, <<"(a+)*z">>,
      [{match_limit_recursion, 1}, report_errors]},
     {<<"abc">>, <<"b">>, [caseless, global]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, [], index}]}].

%% Each capture value and type, on a match with a group that did not take
%% part in the match (build_exec_return, erl_bif_re.c:1047-1256).
captures() ->
    [re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, Values, Type}])
     || Values <- [all, all_but_first, first, none, [0, 1, 2, 7]],
        Type <- [index, list, binary]].

%% The capture result of a pattern with named groups (build_one_capture,
%% erl_bif_re.c:1269-1307; build_capture, erl_bif_re.c:1309-1479). The
%% name table of PCRE2 orders the names alphabetically, and `all_names`
%% gives one element of each distinct name in that order. A name that the
%% pattern does not define gives an empty element.
named_captures() ->
    [re:run(<<"abc">>, <<"(?<x>a)(?<y>b)?">>, Options)
     || Options <- [[{capture, all}],
                    [{capture, all_but_first}],
                    [{capture, all_names}],
                    [{capture, all_names, binary}],
                    [{capture, all_names, list}],
                    [{capture, [x]}],
                    [{capture, [y], list}],
                    [{capture, ["x", y], binary}],
                    [{capture, [nope]}],
                    [{capture, ["nope"], list}]]]
    ++ [re:run(Subject, Pattern, Options)
        || {Subject, Pattern, Options} <-
               [{<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>, [{capture, all}]},
                {<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>,
                 [{capture, all_but_first}]},
                {<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>,
                 [{capture, all_names}]},
                {<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>,
                 [{capture, [z, m, a]}]},
                {<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>,
                 [{capture, ["a"], binary}]},
                {<<"ac">>, <<"(?<x>a)(?<y>b)?">>, [{capture, all_names}]},
                {<<"ac">>, <<"(?<x>a)(?<y>b)?">>, [{capture, [y], list}]},
                {<<"ab">>, <<"(?<n>a)(?<n>b)">>, [dupnames, {capture, all_names}]},
                {<<"ab">>, <<"(?<n>a)(?<n>b)?">>, [dupnames, {capture, [n]}]},
                {<<"a">>, <<"(?<n>b)?(?<n>a)">>, [dupnames, {capture, [n]}]},
                {<<"a">>, <<"a(?<n>b)?(?<n>c)?">>, [dupnames, {capture, [n]}]}]]
    ++ [re:run(<<"abab">>, <<"(?<x>a)(?<y>b)">>,
               [global, {capture, [y], binary}]),
        re:run(<<"abab">>, <<"(?<x>a)(?<y>b)">>,
               [{capture, [x]}, global])].

%% A name that is not valid iodata gives badarg, as each other value that
%% `build_capture` rejects (erl_bif_re.c:1436-1458). ERTS does not define a
%% name that the pattern does not list, so that case gives an empty element
%% rather than an exception (`named_captures/0`).
named_errors() ->
    [catch_error(fun() ->
         re:run(<<"abc">>, <<"(?<x>a)">>, [{capture, [Name]}])
     end)
     || Name <- [[], [foo], [256]]].

%% A global run with several matches, with an empty match, and with an
%% offset (re.erl:1796-1899).
globals() ->
    [re:run(<<"abcabc">>, <<"b">>, [global]),
     re:run(<<"aaa">>, <<"a*">>, [global]),
     re:run(<<"aba">>, <<"a">>, [global, {offset, 1}]),
     re:run(<<"abc">>, <<"z">>, [global]),
     re:run(<<"abcabc">>, <<"b">>, [global, {capture, all, binary}]),
     re:run(<<"abcabc">>, <<"b">>, [global, {capture, none}]),
     re:run(<<"abcabc">>, <<"b">>, [{capture, all, binary}, global])].

%% The badarg of re:run/2,3 (erl_bif_re.c:1501-1518, 1581-1632, 1646-1701,
%% 1756-1786). Each pair holds a name and the caught result.
run_errors() ->
    {ok, Compiled} = re:compile(<<"b">>, [caseless]),
    [{Label, catch_error(fun() -> re:run(Subject, RE, Options) end)}
     || {Label, Subject, RE, Options} <-
            [{options_type, <<"abc">>, <<"b">>, foo},
             {unknown_option, <<"abc">>, <<"b">>, [bogus]},
             {option_not_atom, <<"abc">>, <<"b">>, [1]},
             {unknown_option_after_global, <<"abc">>, <<"b">>, [global, bogus]},
             {offset_past_subject, <<"abc">>, <<"b">>, [{offset, 5}]},
             {offset_negative, <<"abc">>, <<"b">>, [{offset, ?MODULE:id(-1)}]},
             {offset_float, <<"abc">>, <<"b">>, [{offset, 1.5}]},
             {offset_too_large, <<"abc">>, <<"b">>, [{offset, 2147483648}]},
             {match_limit_negative, <<"abc">>, <<"b">>,
              [{match_limit, ?MODULE:id(-1)}]},
             {capture_type_unknown, <<"abc">>, <<"b">>, [{capture, [1], bogus}]},
             {capture_none_type, <<"abc">>, <<"b">>, [{capture, [1], none}]},
             {capture_values_tuple, <<"abc">>, <<"b">>, [{capture, {a}}]},
             {capture_index_float, <<"abc">>, <<"b">>,
              [{capture, [1.5], index}]},
             {capture_index_negative, <<"abc">>, <<"b">>,
              [{capture, [?MODULE:id(-1)], index}]},
             {export, <<"abc">>, <<"b">>, [export]},
             {global_export, <<"abc">>, <<"b">>, [global, export]},
             {pattern_error, <<"abc">>, <<"a(b">>, []},
             {pattern_not_text, <<"abc">>, foo, []},
             {pattern_not_iodata, <<"abc">>, [foo], []},
             {subject_not_iodata, foo, <<"b">>, []},
             {subject_negative_byte, [?MODULE:id(-1)], <<"b">>, []},
             {subject_tuple, {a}, <<"b">>, []},
             {subject_not_utf8, <<255>>, <<"a">>, [unicode]},
             {pattern_not_utf8, <<"abc">>, <<255>>, [unicode]},
             {compile_option_with_pattern, <<"abc">>, Compiled, [caseless]},
             {newline_mismatch, <<"abc">>, Compiled, [{newline, crlf}]},
             {bsr_mismatch, <<"abc">>, Compiled, [bsr_anycrlf]},
             {foreign_pattern, <<"abc">>, {re_pattern, 1, 0, 0, make_ref()}, []},
             {pattern_tag, <<"abc">>, {pattern, 0, 0, 0, make_ref()}, []},
             {pattern_magic_not_ref, <<"abc">>, {re_pattern, 1, 0, 0, foo}, []},
             {pattern_count_not_small, <<"abc">>,
              {re_pattern, 1 bsl 60, 0, 0, ?MODULE:id(make_ref())}, []}]].

%% One case of each shape of a match, with the value of each one.
spot() ->
    {ok, Compiled} = re:compile(<<"(b)">>, []),
    [re:run(<<"abc">>, <<"b">>, []),
     re:run(<<"abc">>, <<"(a)(b)">>, [{capture, all}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, all_but_first}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, first}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, [1], list}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, all, binary}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, none}]),
     re:run(<<"abc">>, <<"z">>, []),
     re:run(<<"abc">>, <<"b">>, [{offset, 2}]),
     re:run(<<"abc">>, <<"b">>, [{offset, 3}]),
     re:run(<<"abc">>, <<"(b)">>, [{capture, all_names}]),
     re:run(<<"a">>, <<"a">>, [{match_limit, 1}, report_errors]),
     re:run(<<"abc">>, Compiled, [{capture, [1], binary}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, [4], index}])].

%% The reductions that a re:run call charges, as erl_bif_re.c:1718 and 1796
%% read them.
reductions() ->
    {ok, Compiled} = re:compile(<<"b">>, []),
    [charge(Subject, RE, Options)
     || {Subject, RE, Options} <-
            [{<<"abc">>, <<"b">>, []},
             {<<"abc">>, <<"z">>, []},
             {<<"abc">>, <<"(a)(b)">>, [{capture, all, binary}]},
             {<<"abc">>, Compiled, []},
             {dup(1000, $a), <<"b">>, []},
             {dup(10000, $a), <<"a*b">>, []}]].

charge(Subject, RE, Options) ->
    {reductions, Before} = process_info(self(), reductions),
    Result = re:run(Subject, RE, Options),
    {reductions, After} = process_info(self(), reductions),
    {After - Before, Result}.

%% The reductions that a re:run call with named groups charges. The name
%% lookup reads the name table of the code and adds no loop of the match.
named_reductions() ->
    [charge(Subject, RE, Options)
     || {Subject, RE, Options} <-
            [{<<"abc">>, <<"(?<x>a)(?<y>b)?">>, [{capture, all_names}]},
             {<<"abc">>, <<"(?<x>a)(?<y>b)?">>, [{capture, [x], binary}]},
             {<<"abc">>, <<"(?<z>a)(?<m>b)(?<a>c)">>, [{capture, [z, m, a]}]}]].

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

%% A pattern that recurses without a match (PCRE2_ERROR_RECURSELOOP,
%% pcre2.h:418) charges one turn of reductions and gives nomatch
%% (erl_bif_re.c:1751-1754). Only the value is compared.
recursion() ->
    [re:run(<<"x">>, <<"(?R)">>, []), re:run(<<"xx">>, <<"a(?R)">>, [])].

%% A precompiled pattern whose offset pair count is above the pairs of the
%% code writes only the pairs of the match; the other pairs of the match
%% data stay uninitialized, so the result reads only the returned pairs.
forged_pairs() ->
    {ok, {re_pattern, _, _, _, Ref}} = re:compile(<<"a">>, []),
    [re:run(<<"a">>, {re_pattern, Count, 0, 0, Ref}, [{capture, all}])
     || Count <- [1, 3, 100]].

%% An error that PCRE2 returns for a live code raises `internal_error`, as
%% the default case of re_run does (erl_bif_re.c:1788-1793). The pattern
%% reaches PCRE2_ERROR_BAD_BACKSLASH_K (pcre2.h:442).
unexpected_error() ->
    [catch_error(fun() ->
         re:run(<<"a">>, <<"(?(DEFINE)(?<k>a\\K))(?=(?&k))">>, [])
     end)].

%% A match that uses the loop limit of one turn continues with a fresh
%% budget (`erl_bif_re.c:1730-1748`, `1852-1866`). ERTS switches the process
%% out between the turns; PBeam runs them in one call. `yield_charge/0`
%% checks that each ended turn reaches the process total.
yield() ->
    [re:run(dup(600000, $a) ++ "b", <<"a*b">>), re:run(dup(600000, $a), <<"a*b">>)].

%% The reduction charge of the first call of `yield/0`, around the call. The
%% charge is more than one turn (`CONTEXT_REDS`, erl_vm.h:53) because each
%% ended turn adds its budget, as the trap does (erl_bif_re.c:1730-1748).
yield_charge() ->
    Subject = dup(600000, $a) ++ "b",
    {reductions, Before} = process_info(self(), reductions),
    Result = re:run(Subject, <<"a*b">>, []),
    {reductions, After} = process_info(self(), reductions),
    {After - Before, Result}.

%% A recursion loop after a restart: the switch of re_match_trap lists
%% neither PCRE2_ERROR_RECURSELOOP nor PCRE2_ERROR_HEAPLIMIT, so ERTS ends
%% the runtime (erl_bif_re.c:1860-1905). Only PBeam runs this probe.
recursion_after_restart() ->
    [catch_error(fun() ->
         re:run(dup(600000, $a), <<"a+(?<r>(?&r))">>, [])
     end)].

%% The fourth argument of re:internal_run/4 must be true or false
%% (erl_bif_re.c:1805-1816). ERTS does not export the function, so only
%% PBeam runs this probe.
internal_run_first() ->
    [catch_error(fun() -> re:internal_run(<<"abc">>, <<"b">>, [], First) end)
     || First <- [true, false, foo, 1]].

%% A list of N copies of C.
dup(N, C) -> dup(N, C, []).
dup(0, _, Acc) -> Acc;
dup(N, C, Acc) -> dup(N - 1, C, [C | Acc]).

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.
